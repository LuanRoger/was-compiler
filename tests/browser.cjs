'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const SIMPLE_SOURCE = 'fn main() -> number { return 42; }';
const OVERFLOW_SOURCE = `fn main() -> number {\n    return ${'9'.repeat(400)};\n}`;
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

function fails(action, pattern = /./) {
    assert.throws(action, error => {
        assert.ok(error instanceof Error);
        assert.equal(error.name, 'Error');
        assert.match(error.message, pattern);
        return true;
    });
}

function compile(compiler, source) {
    const bytes = compiler.compile(source);
    assert.ok(bytes instanceof Uint8Array);
    assert.deepEqual(Array.from(bytes.subarray(0, 8)), [0, 97, 115, 109, 1, 0, 0, 0]);
    assert.ok(WebAssembly.validate(bytes));
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.customSections(module, 'was-compiler.interface'), []);
    return { bytes, module };
}

function instantiate(compiler, source, imports = {}) {
    return new WebAssembly.Instance(compile(compiler, source).module, imports);
}

function writeDescriptor(instance, type, value) {
    const payload = type === 'string' ? encoder.encode(value) : value;
    const width = type === 'string' ? 1 : 8;
    const handle = instance.exports.__alloc(8 + payload.length * width) >>> 0;
    const buffer = instance.exports.memory.buffer;
    const view = new DataView(buffer);
    view.setUint32(handle, handle + 8, true);
    view.setUint32(handle + 4, payload.length, true);
    if (width === 1) {
        new Uint8Array(buffer, handle + 8, payload.length).set(payload);
    } else {
        payload.forEach((value, index) => view.setFloat64(handle + 8 + index * 8, value, true));
    }
    return handle;
}

function readDescriptor(instance, type, handle) {
    assert.equal(typeof handle, 'number');
    const buffer = instance.exports.memory.buffer;
    const view = new DataView(buffer);
    const data = view.getUint32(handle >>> 0, true);
    const length = view.getUint32((handle >>> 0) + 4, true);
    if (type === 'string') {
        return decoder.decode(new Uint8Array(buffer, data, length));
    }
    return Array.from({ length }, (_, index) => view.getFloat64(data + index * 8, true));
}

function userExports(module) {
    return WebAssembly.Module.exports(module)
        .filter(({ name }) => name !== 'memory' && name !== '__alloc')
        .map(({ name }) => name);
}

async function main() {
    assert.ok(process.argv.length <= 3, 'usage: node tests/browser.cjs [pkg-directory]');
    const directory = path.resolve(process.argv[2] || path.join(__dirname, '..', 'pkg'));
    const modulePath = path.join(directory, 'was_compiler.js');
    const wasmPath = path.join(directory, 'was_compiler_bg.wasm');
    await fs.access(modulePath);
    const wasm = await fs.readFile(wasmPath);
    const { default: init, Compiler } = await import(pathToFileURL(modulePath).href);
    assert.equal(typeof init, 'function');
    assert.equal(typeof Compiler, 'function');
    await init({ module_or_path: wasm });

    let count = 0;
    function test(name, action) {
        const compiler = new Compiler();
        try {
            action(compiler);
            count += 1;
            console.log(`ok ${count} - ${name}`);
        } catch (error) {
            error.message = `${name}: ${error.message}`;
            throw error;
        } finally {
            compiler.free();
        }
    }

    test('arithmetic, control flow, recursion, and internal calls', compiler => {
        const source = `
            fn arithmetic(value: number) -> number { return -(value + 2) * 3 / 2 - 1; }
            fn fib(value: number) -> number {
                if value <= 1 { return value; }
                return fib(value - 1) + fib(value - 2);
            }
            fn sum(limit: number) -> number {
                let index: number = 0;
                let total: number = 0;
                while index < limit {
                    total = total + double(index);
                    index = index + 1;
                }
                if total >= 12 && !(limit == 0) { return total; } else { return -1; }
            }
            fn double(value: number) -> number { return value * 2; }
            fn flag(left: bool, right: bool) -> bool { return !left || right; }
            fn noop() -> void {}
        `;
        const { exports } = instantiate(compiler, source);
        assert.equal(exports.arithmetic(4), -10);
        assert.equal(exports.fib(10), 55);
        assert.equal(exports.sum(4), 12);
        assert.equal(exports.sum(0), -1);
        assert.equal(exports.flag(1, 0), 0);
        assert.equal(exports.flag(0, 0), 1);
        assert.equal(exports.noop(), undefined);
        const wat = compiler.compileWat(source);
        assert.equal(typeof wat, 'string');
        assert.match(wat, /^\(module\b/);
        assert.match(wat, /\(export "sum"/);
        assert.doesNotMatch(wat, /was-compiler\.interface/);
    });

    test('remainder arithmetic, IEEE-754 inputs, host order, and guest-only memory', compiler => {
        const source = `
            fn remainder(left: number, right: number) -> number { return left % right; }
            fn arithmetic() -> number { return 2 + 17 % 5 * 3; }
            fn nested(value: number) -> number { return -(value % 5) % 3; }
        `;
        const { bytes, module } = compile(compiler, source);
        assert.deepEqual(WebAssembly.Module.imports(module), []);
        assert.deepEqual(userExports(module), ['remainder', 'arithmetic', 'nested']);
        const scalar = new WebAssembly.Instance(module).exports;
        assert.equal(scalar.memory, undefined);
        assert.equal(scalar.__alloc, undefined);
        assert.equal(scalar.arithmetic(), 8);
        assert.equal(scalar.nested(17), -2);
        const pairs = [
            [5.5, 2], [-5.5, 2], [5.5, -2], [-5.5, -2],
            [6, 3], [-6, 3], [0, 3], [-0, 3], [0, -3], [-0, -3],
            [1, 0], [1, -0], [0, 0], [-0, 0],
            [Infinity, 3], [-Infinity, 3], [Infinity, Infinity],
            [3, Infinity], [-3, -Infinity], [-0, Infinity],
            [NaN, 3], [3, NaN],
            [Number.MAX_VALUE, 3], [-Number.MAX_VALUE, 3],
            [Number.MAX_VALUE, Number.MIN_VALUE],
            [Number.MAX_VALUE, 2.2250738585072014e-308],
            [Number.MIN_VALUE, Number.MAX_VALUE],
            [-Number.MIN_VALUE, Number.MAX_VALUE],
            [Number.MIN_VALUE * 3, Number.MIN_VALUE * 2],
            [-Number.MIN_VALUE * 3, Number.MIN_VALUE * 2],
            [1, Number.MIN_VALUE], [-1, Number.MIN_VALUE],
            [1e300, 1e-300], [-1e300, 1e-300],
            [1e-300, 1e300], [-1e-300, 1e300],
        ];
        for (const [left, right] of pairs) {
            assert.ok(Object.is(scalar.remainder(left, right), left % right), `${left} % ${right}`);
        }
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            assert.deepEqual(compile(compiler, source).bytes, bytes);
        }
        const memorySource = `
            fn remainder(left: number, right: number) -> number { return left % right; }
            fn items(left: number, right: number) -> number[] { return [left % right, -left % right]; }
            fn edit(values: number[], divisor: number) -> number[] {
                values[0] = values[0] % divisor;
                return values;
            }
        `;
        const memoryModule = compile(compiler, memorySource).module;
        assert.deepEqual(WebAssembly.Module.imports(memoryModule), []);
        const memory = new WebAssembly.Instance(memoryModule);
        assert.ok(memory.exports.memory instanceof WebAssembly.Memory);
        assert.equal(typeof memory.exports.__alloc, 'function');
        for (const [left, right] of pairs) {
            assert.ok(Object.is(memory.exports.remainder(left, right), left % right));
        }
        assert.deepEqual(readDescriptor(memory, 'number[]', memory.exports.items(5.5, 2)), [1.5, -1.5]);
        const values = writeDescriptor(memory, 'number[]', [-6, 7]);
        assert.equal(memory.exports.edit(values, 3), values);
        assert.deepEqual(readDescriptor(memory, 'number[]', values), [-0, 7]);
        compiler.registerHostFunction('left', 'env', 'left', [], 'number');
        compiler.registerHostFunction('right', 'env', 'right', [], 'number');
        const hostSource = `
            fn main() -> number { return left() % right(); }
            fn memoryMain() -> number[] { return [left() % right()]; }
        `;
        const hostModule = compile(compiler, hostSource).module;
        assert.deepEqual(WebAssembly.Module.imports(hostModule), [
            { module: 'env', name: 'left', kind: 'function' },
            { module: 'env', name: 'right', kind: 'function' },
        ]);
        const events = [];
        const host = new WebAssembly.Instance(hostModule, { env: {
            left() { events.push('left'); return -5.5; },
            right() { events.push('right'); return 2; },
        } });
        assert.equal(host.exports.main(), -1.5);
        assert.deepEqual(events, ['left', 'right']);
        events.length = 0;
        assert.deepEqual(readDescriptor(host, 'number[]', host.exports.memoryMain()), [-1.5]);
        assert.deepEqual(events, ['left', 'right']);
    });

    test('string and array execution with explicit raw memory descriptors', compiler => {
        const source = `
            fn literal() -> string { return "héllo 🦀"; }
            fn echo(value: string) -> string { return value; }
            fn same(left: string, right: string) -> bool { return left == right; }

            fn items() -> number[] { return [1, 2, 3]; }
            fn edit(values: number[]) -> number[] { values[1] = values[0] + 10; return values; }
            fn sum(values: number[]) -> number {
                let index: number = 0;
                let total: number = 0;
                while index < len(values) {
                    total = total + values[index];
                    index = index + 1;
                }
                return total;
            }
        `;
        const instance = instantiate(compiler, source);
        assert.ok(instance.exports.memory instanceof WebAssembly.Memory);
        assert.equal(typeof instance.exports.__alloc, 'function');
        assert.equal(readDescriptor(instance, 'string', instance.exports.literal()), 'héllo 🦀');
        for (const value of ['', 'héllo 🦀', '\u0000', '\ufeffBOM', 'x'.repeat(100000)]) {
            const handle = writeDescriptor(instance, 'string', value);
            assert.equal(instance.exports.echo(handle), handle);
            assert.equal(readDescriptor(instance, 'string', handle), value);

            assert.equal(instance.exports.same(handle, writeDescriptor(instance, 'string', value)), 1);
        }
        assert.equal(instance.exports.same(
            writeDescriptor(instance, 'string', 'a'), writeDescriptor(instance, 'string', 'b')), 0);
        assert.deepEqual(readDescriptor(instance, 'number[]', instance.exports.items()), [1, 2, 3]);
        const values = writeDescriptor(instance, 'number[]', [2, 3, 4]);
        assert.equal(instance.exports.edit(values), values);
        assert.deepEqual(readDescriptor(instance, 'number[]', values), [2, 12, 4]);
        assert.equal(instance.exports.sum(values), 18);
        assert.equal(instance.exports.sum(writeDescriptor(instance, 'number[]', [])), 0);
        assert.throws(() => instance.exports.edit(writeDescriptor(instance, 'number[]', [])), WebAssembly.RuntimeError);
        assert.equal(instance.exports.sum(values), 18);
    });

    test('numeric array content equality, mutation, IEEE-754, and descriptor traps', compiler => {
        const source = `
            fn same(left: number[], right: number[]) -> bool { return left == right; }
            fn different(left: number[], right: number[]) -> bool { return left != right; }
            fn literals() -> bool { return [1, 2, 3] == [1, 2, 3]; }
            fn shorter() -> bool { return [1, 2, 3] == [1, 2]; }
            fn items() -> number[] { return [1, 2, 3]; }
            fn edit(values: number[]) -> number[] { values[1] = 9; return values; }
            fn callbacks() -> bool { return arrayLeft() == arrayRight(); }
        `;
        compiler.registerHostFunction('arrayLeft', 'env', 'left', [], 'number[]');
        compiler.registerHostFunction('arrayRight', 'env', 'right', [], 'number[]');
        const { bytes, module } = compile(compiler, source);
        assert.deepEqual(compiler.compile(source), bytes);
        assert.match(compiler.compileWat(source), /f64\.(eq|ne)/);
        let instance;
        let left, right, corrupt = false;
        const events = [];
        instance = new WebAssembly.Instance(module, { env: {
            left() { events.push('left'); return left; },
            right() {
                events.push('right');
                if (corrupt) new DataView(instance.exports.memory.buffer).setUint32(left + 4, 0xffffffff, true);
                return right;
            },
        } });
        const { exports } = instance;
        assert.equal(exports.literals(), 1);
        assert.equal(exports.shorter(), 0);
        for (const [a, b, same] of [
            [[], [], true], [[], [1], false], [[1, 2, 3], [1, 2, 3], true],
            [[1, 2, 3], [3, 2, 1], false], [[0, -0], [-0, 0], true],
            [[NaN], [NaN], false], [[Infinity, -Infinity], [Infinity, -Infinity], true],
        ]) {
            left = writeDescriptor(instance, 'number[]', a);
            right = writeDescriptor(instance, 'number[]', b);
            const before = new Uint8Array(exports.memory.buffer).slice();
            assert.equal(exports.same(left, right), Number(same));
            assert.equal(exports.different(left, right), Number(!same));
            assert.deepEqual(new Uint8Array(exports.memory.buffer), before);
            assert.equal(exports.same(left, left), Number(!a.some(Number.isNaN)));
            assert.equal(exports.different(left, left), Number(a.some(Number.isNaN)));
            events.length = 0;
            assert.equal(exports.callbacks(), Number(same));
            assert.deepEqual(events, ['left', 'right']);
        }
        left = exports.items(); right = exports.items();
        assert.notEqual(left, right);
        assert.equal(exports.same(left, right), 1);
        assert.equal(exports.edit(left), left);
        assert.equal(exports.same(left, right), 0);
        assert.equal(exports.different(left, right), 1);
        assert.deepEqual(readDescriptor(instance, 'number[]', left), [1, 9, 3]);
        assert.deepEqual(readDescriptor(instance, 'number[]', right), [1, 2, 3]);
        assert.throws(() => exports.same(0, right), WebAssembly.RuntimeError);
        assert.throws(() => exports.different(left, 0), WebAssembly.RuntimeError);
        left = writeDescriptor(instance, 'number[]', []);
        right = writeDescriptor(instance, 'number[]', [1]);
        corrupt = true;
        events.length = 0;
        assert.throws(() => exports.callbacks(), WebAssembly.RuntimeError);
        assert.deepEqual(events, ['left', 'right']);
        for (const expression of ['[] == 1', 'true != []', '[] == "text"', '[] < []', '[] + []', '[] && []']) {
            for (const method of ['compile', 'compileWat']) {
                fails(() => compiler[method](`fn invalid() -> void { ${expression}; }`), /semantic error:/);
            }
        }
        assert.equal(instantiate(compiler, SIMPLE_SOURCE, { env: { left: () => 0, right: () => 0 } }).exports.main(), 42);
    });

    test('compile and compileWat report each stage and recover', compiler => {
        const cases = [
            ['fn main() -> void {\n    let n: number = ;\n}', 'parse'],
            ['fn main() -> void {\n    missing();\n}', 'semantic'],
            [OVERFLOW_SOURCE, 'code generation'],
        ];
        for (const [source, stage] of cases) {
            for (const method of ['compile', 'compileWat']) {
                fails(() => compiler[method](source), new RegExp(`^<browser>:2:\\d+: ${stage} error:`));
                fails(() => compiler[method](source), /2 \|.*\n.*\^/);
                assert.equal(instantiate(compiler, SIMPLE_SOURCE).exports.main(), 42);
                assert.match(compiler.compileWat(SIMPLE_SOURCE), /\(export "main"/);
            }
        }
    });

    test('host imports cover every supported type without wrappers', compiler => {
        const signatures = [
            ['numberHost', 'num', ['number'], 'number'],
            ['boolHost', 'flag', ['bool'], 'bool'],
            ['stringHost', 'text', ['string'], 'string'],
            ['arrayHost', 'items', ['number[]'], 'number[]'],
            ['voidHost', 'observe', ['number', 'bool', 'string', 'number[]'], 'void'],
        ];
        for (const [name, field, params, result] of signatures) {
            compiler.registerHostFunction(name, 'app.host', field, params, result);
        }
        const source = `
            fn numeric() -> number { return numberHost(2.5); }
            fn boolean() -> bool { return boolHost(false); }
            fn text() -> string { return stringHost("héllo 🦀"); }
            fn items() -> number[] { return arrayHost([1, 2]); }
            fn observe() -> void { voidHost(7, true, "raw", [3, 4]); }
        `;
        const { module } = compile(compiler, source);
        assert.deepEqual(WebAssembly.Module.imports(module), signatures.map(([, field]) => ({
            module: 'app.host', name: field, kind: 'function',
        })));
        assert.throws(() => new WebAssembly.Instance(module), TypeError);
        let instance;
        let observed = false;
        instance = new WebAssembly.Instance(module, { 'app.host': {
            num(value) { assert.equal(value, 2.5); return value * 4; },
            flag(value) { assert.equal(value, 0); return 1; },
            text(handle) {
                assert.equal(readDescriptor(instance, 'string', handle), 'héllo 🦀');
                return writeDescriptor(instance, 'string', 'returned');
            },
            items(handle) {
                assert.deepEqual(readDescriptor(instance, 'number[]', handle), [1, 2]);
                return writeDescriptor(instance, 'number[]', [9, 8, 7]);
            },
            observe(number, bool, text, items) {
                assert.equal(number, 7);
                assert.equal(bool, 1);
                assert.equal(readDescriptor(instance, 'string', text), 'raw');
                assert.deepEqual(readDescriptor(instance, 'number[]', items), [3, 4]);
                observed = true;
            },
        } });
        assert.equal(instance.exports.numeric(), 10);
        assert.equal(instance.exports.boolean(), 1);
        assert.equal(readDescriptor(instance, 'string', instance.exports.text()), 'returned');
        assert.deepEqual(readDescriptor(instance, 'number[]', instance.exports.items()), [9, 8, 7]);
        assert.equal(instance.exports.observe(), undefined);
        assert.ok(observed);
        fails(() => compiler.compile('fn main() -> number { return numberHost(true); }'), /semantic error:/);
        fails(() => compiler.compile('fn main() -> number { return numberHost(); }'), /semantic error:/);
        fails(() => compiler.compile('fn numberHost() -> void {}'), /semantic error:/);
        assert.equal(instantiate(compiler, source, { 'app.host': {
            num: value => value, flag: value => value, text: value => value,
            items: value => value, observe() {},
        } }).exports.numeric(), 2.5);
    });

    test('host registration rejects invalid names, types, and arrays atomically', compiler => {
        for (const name of ['', '1name', 'has space', 'é', 'a-b', 'fn', 'len', 'number']) {
            fails(() => compiler.registerHostFunction(name, 'env', 'run', [], 'void'));
        }
        for (const args of [
            ['candidate', '', 'run', [], 'void'],
            ['candidate', 'env', '', [], 'void'],
            ['candidate', 'env', 'run', ['void'], 'void'],
            ['candidate', 'env', 'run', ['unknown'], 'void'],
            ['candidate', 'env', 'run', ['bool[]'], 'void'],
            ['candidate', 'env', 'run', ['string[]'], 'void'],
            ['candidate', 'env', 'run', [], 'unknown'],
        ]) {
            fails(() => compiler.registerHostFunction(...args));
        }
        for (const params of [undefined, null, 'number', 1, {}, new Set(), new Uint8Array(),
            [1], [true], [null], [undefined], [{}], ['number', 1], new Array(1)]) {
            fails(() => compiler.registerHostFunction('candidate', 'env', 'run', params, 'void'));
        }
        compiler.registerHostFunction('candidate', 'env', 'original', [], 'number');
        fails(() => compiler.registerHostFunction('candidate', 'other', 'replacement', [], 'void'), /already registered/i);
        const { module } = compile(compiler, 'fn main() -> number { return candidate(); }');
        assert.deepEqual(WebAssembly.Module.imports(module), [{ module: 'env', name: 'original', kind: 'function' }]);
        assert.equal(new WebAssembly.Instance(module, { env: { original: () => 17 } }).exports.main(), 17);
    });

    test('all, none, selected, and invalid exports preserve compilation', compiler => {
        const source = `
            fn hidden() -> number { return 9; }
            fn main() -> number { return helper(); }
            fn helper() -> number { return 65; }
            fn text() -> string { return "memory"; }
        `;
        assert.deepEqual(userExports(compile(compiler, source).module), ['hidden', 'main', 'helper', 'text']);
        compiler.setExports(['text', 'main']);
        assert.deepEqual(userExports(compile(compiler, source).module), ['main', 'text']);
        assert.equal(instantiate(compiler, source).exports.main(), 65);
        assert.doesNotMatch(compiler.compileWat(source), /\(export "helper"/);
        for (const names of [undefined, null, 'main', 1, {}, new Set(['main']), new Uint8Array(),
            [1], [false], [null], [undefined], [{}], ['main', 1], new Array(1)]) {
            fails(() => compiler.setExports(names));
            assert.deepEqual(userExports(compile(compiler, source).module), ['main', 'text']);
        }
        for (const names of [['missing'], ['Main'], [' main'], ['memory'], ['__alloc'], ['main', 'main']]) {
            compiler.setExports(names);
            for (const method of ['compile', 'compileWat']) {
                fails(() => compiler[method](source), /^<browser>: code generation error: (unknown user function export|duplicate requested export)/);
            }
            compiler.setExports(['main']);
            assert.equal(instantiate(compiler, source).exports.main(), 65);
        }
        compiler.exportNone();
        const none = compile(compiler, source).module;
        assert.deepEqual(WebAssembly.Module.exports(none).map(({ name }) => name), ['memory', '__alloc']);
        assert.deepEqual(WebAssembly.Module.exports(compile(compiler, SIMPLE_SOURCE).module), []);
        compiler.setExports([]);
        assert.deepEqual(userExports(compile(compiler, source).module), []);
        fails(() => compiler.compile(OVERFLOW_SOURCE), /code generation error:.*outside the finite f64 range/);
        compiler.exportAll();
        assert.deepEqual(userExports(compile(compiler, source).module), ['hidden', 'main', 'helper', 'text']);
    });

    test('optimization levels are exact integers and currently preserve bytes and state', compiler => {
        compiler.registerHostFunction('clock', 'env', 'now', [], 'number');
        compiler.setExports(['main']);
        const source = 'fn main() -> number { return clock() + helper(); } fn helper() -> number { return 2 * 3; }';
        const expected = compile(compiler, source).bytes;
        const wat = compiler.compileWat(source);
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            assert.deepEqual(compile(compiler, source).bytes, expected);
            assert.equal(compiler.compileWat(source), wat);
            assert.equal(instantiate(compiler, source, { env: { now: () => 4 } }).exports.main(), 10);
            fails(() => compiler.compile(OVERFLOW_SOURCE), /code generation error:.*outside the finite f64 range/);
        }
        for (const level of [-1, 4, 0.5, 1.5, 2.999, NaN, Infinity, -Infinity, 2 ** 32]) {
            fails(() => compiler.setOptimizationLevel(level));
            assert.deepEqual(compile(compiler, source).bytes, expected);
        }
    });

    test('returned wasm bytes remain owned across calls and compiler disposal', compiler => {
        const bytes = compile(compiler, SIMPLE_SOURCE).bytes;
        const snapshot = Uint8Array.from(bytes);
        for (let index = 0; index < 20; index++) {
            compile(compiler, `fn other() -> number { return ${index}; }`);
            compiler.compileWat(SIMPLE_SOURCE);
        }
        assert.deepEqual(bytes, snapshot);
        const other = new Compiler();
        const retained = compile(other, SIMPLE_SOURCE).bytes;
        other.free();
        assert.throws(() => other.compile(SIMPLE_SOURCE), Error);
        assert.throws(() => other.compileWat(SIMPLE_SOURCE), Error);
        assert.equal(new WebAssembly.Instance(new WebAssembly.Module(retained)).exports.main(), 42);
        assert.equal(new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports.main(), 42);
        assert.equal(instantiate(compiler, SIMPLE_SOURCE).exports.main(), 42);
    });

    console.log(`browser compiler: ${count} tests passed`);
}

main().catch(error => {
    console.error(error);
    process.exitCode = 1;
});
