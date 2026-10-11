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

    test('while transfers target nearest loops, recheck conditions, and retain diagnostics', compiler => {
        compiler.registerHostFunction('poll', 'env', 'poll', ['number', 'number'], 'bool');
        compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'void');
        const source = `
            fn main() -> number {
                let i = 0;
                let total = 0;
                while poll(i, 4) {
                    i += 1;
                    let j = 0;
                    while j < 3 {
                        j += 1;
                        if j == 2 { if i > 0 { continue; } }
                        if j == 3 { if true { break; } }
                        mark(i * 10 + j);
                        total += 1;
                    }
                    if i == 2 { continue; }
                    if i == 3 { break; }
                    total += 10;
                    mark(i * 100);
                }
                return total;
            }
        `;
        const expected = compile(compiler, source).bytes;
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            const { bytes, module } = compile(compiler, source);
            assert.deepEqual(bytes, expected);
            const events = [];
            const e = new WebAssembly.Instance(module, { env: {
                poll(i, limit) { events.push('p' + i); return i < limit ? 1 : 0; },
                mark(n) { events.push(n); }
            }}).exports;
            assert.equal(e.main(), 13);
            assert.deepEqual(events, ['p0', 11, 100, 'p1', 21, 'p2', 31]);
            for (const keyword of ['break', 'continue']) {
                fails(() => compiler.compile(`fn main() -> void {\n    ${keyword};\n}`),
                    /<browser>:2:5: semantic error:.*only allowed inside a loop/);
                fails(() => compiler.compile(`fn main() -> void { while true { ${keyword} 1; } }`),
                    keyword === 'break' ? /semantic error:.*break value is not allowed/ : /parse error/);
                fails(() => compiler.compile(`fn main() -> number { while true { ${keyword}; return 1; } }`),
                    /does not return a value on every path/);
            }
            assert.deepEqual(compile(compiler, source).bytes, expected);
        }
        fails(() => compiler.registerHostFunction('break', 'env', 'run', [], 'void'), /reserved/);
        fails(() => compiler.registerHostFunction('continue', 'env', 'run', [], 'void'), /reserved/);
    });

    test('statement and value loops preserve types, effects, transfers, Never, diagnostics, and parity', compiler => {
        fails(() => compiler.registerHostFunction('loop', 'env', 'run', [], 'void'), /reserved/);
        compiler.registerHostFunction('mark', 'loop', 'loop', ['number'], 'number');
        compiler.registerHostFunction('bridge', 'module with spaces/λ', 'field:with,delimiters', [], 'number');
        const source = `
            fn statements() -> number {
                let i = 0;
                let total = 0;
                loop {
                    i += 1;
                    let j = 0;
                    loop {
                        j += 1;
                        if j == 2 { continue; }
                        if j == 3 { break; }
                        mark(i * 10 + j);
                        total += 1;
                    }
                    if i == 2 { continue; }
                    if i == 3 { break; }
                    total += 10;
                    mark(i * 100);
                }
                return total;
            }
            fn numeric() -> number {
                let i = 0;
                return loop {
                    i += 1;
                    mark(i);
                    if i == 2 { continue; }
                    if i == 4 { break { mark(40); i + 2 }; }
                };
            }
            fn boolean(flag: bool) -> bool {
                return loop { if flag { break true; } else { break false; } };
            }
            fn text(flag: bool) -> string {
                return loop { if flag { break "é🦀"; } break "other"; };
            }
            fn items() -> number[] { return loop { break [mark(10), mark(11)]; }; }
            fn alias(a: number[]) -> number[] {
                let result = loop {
                    while true { break; }
                    loop { break; }
                    break a;
                };
                result[0] += 1;
                return result;
            }
            fn noop() -> void { loop { mark(41); break; }; }
            fn early(flag: bool) -> number {
                return loop {
                    if flag { break { return mark(50); }; }
                    break mark(51);
                };
            }
            fn statement_return() -> number { loop { return mark(52); } }
            fn tail() -> number { return { loop { break mark(60); } }; }
            fn skipped() -> number {
                let ignored = false && loop {};
                let other = true || loop { continue; };
                return mark(7);
            }
            fn imported() -> number { return loop { break bridge(); }; }
        `;
        // These functions deliberately have no exit and must never be invoked.
        const noExit = `
            fn statement() -> number { loop {} }
            fn value() -> number { return loop { continue; }; }
            fn annotated() -> bool { let value: string = loop {}; }
        `;
        const cases = [
            ['fn main() -> void {\n    while true {\n        break 1;\n    }\n}',
                /A break value is not allowed in a statement loop/, 'break 1;', 3, 9],
            ['fn main() -> void {\n    while true {\n        break outer;\n    }\n}',
                /A break value is not allowed in a statement loop/, 'break outer;', 3, 9],
            ['fn main() -> void {\n    loop {\n        break 1;\n    }\n}',
                /A break value is not allowed in a statement loop/, 'break 1;', 3, 9],
            ['fn main() -> number {\n    return loop {\n        break;\n    };\n}',
                /A value loop requires a break value/, 'break;', 3, 9],
            ['fn main() -> number {\n    return loop {\n        break {};\n    };\n}',
                /A break value cannot have type void/, 'break {};', 3, 9],
            ['fn main(flag: bool) -> void {\n    let value = loop { if flag { break 1; } else { break false; } };\n}',
                /Type mismatch: expected Number, found Bool/,
                'let value = loop { if flag { break 1; } else { break false; } };', 2, 5],
            ['fn main() -> void {\n    let value = loop {};\n}',
                /Cannot infer a concrete type for divergent initializer 'value'/, 'let value = loop {};', 2, 5],
            ['fn main() -> number {\n    return loop {\n        while false { break 1; }\n        break 2;\n    };\n}',
                /A break value is not allowed in a statement loop/, 'while false { break 1; }', 3, 23],
            ['fn main() -> void {\n    loop {\n        let value = loop { break; };\n        break;\n    }\n}',
                /A value loop requires a break value/, 'let value = loop { break; };', 3, 28],
            ['fn main() -> number {\n    let value = false && loop { break; };\n    return 1;\n}',
                /A value loop requires a break value/, 'let value = false && loop { break; };', 2, 33],
        ];
        const expected = compile(compiler, source).bytes;
        const wat = compiler.compileWat(source);
        const neverBytes = compile(compiler, noExit).bytes;
        const neverWat = compiler.compileWat(noExit);
        const diagnostics = new Map();
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            const { bytes, module } = compile(compiler, source);
            assert.deepEqual(bytes, expected);
            assert.equal(compiler.compileWat(source), wat);
            assert.deepEqual(compile(compiler, noExit).bytes, neverBytes);
            assert.equal(compiler.compileWat(noExit), neverWat);
            assert.deepEqual(WebAssembly.Module.imports(module), [
                { module: 'loop', name: 'loop', kind: 'function' },
                { module: 'module with spaces/λ', name: 'field:with,delimiters', kind: 'function' },
            ]);
            const events = [];
            const instance = new WebAssembly.Instance(module, {
                loop: { loop(value) { events.push(value); return value; } },
                'module with spaces/λ': { 'field:with,delimiters': () => 19 },
            });
            const e = instance.exports;
            for (const [name, args, result, effects] of [
                ['statements', [], 13, [11, 100, 21, 31]],
                ['numeric', [], 6, [1, 2, 3, 4, 40]],
                ['noop', [], undefined, [41]],
                ['early', [1], 50, [50]], ['early', [0], 51, [51]],
                ['statement_return', [], 52, [52]], ['tail', [], 60, [60]],
                ['skipped', [], 7, [7]], ['imported', [], 19, []],
            ]) {
                events.length = 0;
                assert.equal(e[name](...args), result);
                assert.deepEqual(events, effects);
            }
            for (const flag of [0, 1]) {
                assert.equal(e.boolean(flag), flag);
                assert.equal(readDescriptor(instance, 'string', e.text(flag)), flag ? 'é🦀' : 'other');
            }
            events.length = 0;
            const a = e.items(), b = e.items();
            assert.notEqual(a, b);
            assert.deepEqual(readDescriptor(instance, 'number[]', a), [10, 11]);
            assert.deepEqual(readDescriptor(instance, 'number[]', b), [10, 11]);
            assert.deepEqual(events, [10, 11, 10, 11]);
            assert.equal(e.alias(a), a);
            assert.deepEqual(readDescriptor(instance, 'number[]', a), [11, 11]);
            assert.deepEqual(readDescriptor(instance, 'number[]', b), [10, 11]);
            for (const [invalid, message, excerpt, line, column] of cases) {
                for (const method of ['compile', 'compileWat']) {
                    assert.throws(() => compiler[method](invalid), error => {
                        assert.ok(error instanceof Error);
                        assert.equal(error.name, 'Error');
                        assert.match(error.message, new RegExp(`^<browser>:${line}:${column}: semantic error:`));
                        assert.match(error.message, message);
                        assert.ok(error.message.includes(excerpt));
                        assert.ok(error.message.includes(` |${' '.repeat(column)}^`));
                        if (diagnostics.has(invalid)) {
                            assert.equal(error.message, diagnostics.get(invalid));
                        } else {
                            diagnostics.set(invalid, error.message);
                        }
                        return true;
                    });
                    assert.deepEqual(compile(compiler, source).bytes, expected);
                    assert.equal(compiler.compileWat(source), wat);
                }
            }
            for (const invalid of [
                'fn loop() -> void {}',
                'fn main() -> void { outer: loop { break; } }',
                'fn main() -> void { loop { continue outer; } }',
            ]) {
                fails(() => compiler.compile(invalid), /parse error/);
                fails(() => compiler.compileWat(invalid), /parse error/);
                assert.deepEqual(compile(compiler, source).bytes, expected);
                assert.equal(compiler.compileWat(source), wat);
            }
        }
    });

    test('compound updates preserve old values, target traps, and optimization parity', compiler => {
        compiler.registerHostFunction('index', 'env', 'index', [], 'number');
        compiler.registerHostFunction('rhs', 'env', 'rhs', ['number[]'], 'number');
        const source = `
            fn main() -> number {
                let values = [10];
                let alias = values;
                alias[index()] += rhs(values);
                return values[0];
            }
            fn checked(values: number[], i: number) -> number {
                values[i] += rhs(values);
                return values[i];
            }
            fn arithmetic(n: number) -> number {
                n += 2; n -= 1; n *= 3; n /= 2; n %= 5;
                return n;
            }
            fn remainder(n: number, divisor: number) -> number { n %= divisor; return n; }
        `;
        const expected = compile(compiler, source).bytes;
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            const { bytes, module } = compile(compiler, source);
            assert.deepEqual(bytes, expected);
            let instance;
            const events = [];
            instance = new WebAssembly.Instance(module, { env: {
                index() { events.push('index'); return 0; },
                rhs(handle) {
                    events.push('rhs');
                    instance.exports.memory.grow(1);
                    const view = new DataView(instance.exports.memory.buffer);
                    view.setFloat64(view.getUint32(handle, true), 100, true);
                    return 2;
                }
            }});
            assert.equal(instance.exports.main(), 12);
            assert.deepEqual(events, ['index', 'rhs']);
            assert.equal(instance.exports.arithmetic(10), 1.5);
            for (const [a, b] of [[-0, 3], [NaN, 1], [Infinity, 2], [1, 0],
                [2, Infinity], [Number.MAX_VALUE, Number.MIN_VALUE]]) {
                assert.ok(Object.is(instance.exports.remainder(a, b), a % b));
            }
            const handle = writeDescriptor(instance, 'number[]', [10]);
            for (const invalid of [-1, 1, 0.5, NaN, Infinity]) {
                events.length = 0;
                assert.throws(() => instance.exports.checked(handle, invalid), WebAssembly.RuntimeError);
                assert.deepEqual(events, []);
                assert.deepEqual(readDescriptor(instance, 'number[]', handle), [10]);
            }
            fails(() => compiler.compile('fn bad() -> void { let b = true; b += 1; }'), /semantic error/);
            fails(() => compiler.compile('fn bad() -> void { let n = 1; let result = n += 2; }'), /parse error/);
        }
    });

    test('comments preserve bytes across optimization levels and execute without changing strings or division', compiler => {
        const plain = 'fn main() -> number { return 84 / 2; } fn text() -> string { return "// /* */ é🦀"; }';
        const commented = '// leading comment\n'
            + '/* multiline\n/* nested opener is text */fn/**/main()/* signature */->number{\n'
            + '// stops at CR\rreturn/* left */84 / /* divisor */2;// stops at LF\n'
            + '}\nfn text()->string{return "// /* */ é🦀";}// EOF';
        const expected = compile(compiler, plain).bytes;
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            assert.deepEqual(compile(compiler, plain).bytes, expected);
            const { bytes, module } = compile(compiler, commented);
            assert.deepEqual(bytes, expected);
            const instance = new WebAssembly.Instance(module);
            assert.equal(instance.exports.main(), 42);
            assert.equal(readDescriptor(instance, 'string', instance.exports.text()), '// /* */ é🦀');
        }
    });

    test('inferred locals preserve all types, host order, and annotated bytes at every level', compiler => {
        compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'number');
        const annotated = `
            fn numeric() -> number {
                let n: number = 10;
                let values: number[] = [mark(1), mark(2)];
                let alias: number[] = values;
                alias[1] = mark(3);
                let length: number = len(values);
                let indexed: number = values[1];
                let result: number = later(n) + length + indexed;
                return result;
            }
            fn boolean() -> bool { let flag: bool = true; let value: bool = flag && !false; return value; }
            fn text() -> string { let text: string = "é🦀"; let copy: string = text; return copy; }
            fn items() -> number[] { let values: number[] = [1, 2]; return values; }
            fn empty() -> number[] { let values: number[] = []; return values; }
            fn later(value: number) -> number { return value * 2; }
        `;
        const inferred = annotated.replace(/: (number\[\]|number|bool|string) =/g, ' =');
        const expected = compile(compiler, annotated).bytes;
        const wat = compiler.compileWat(annotated);
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            assert.deepEqual(compile(compiler, annotated).bytes, expected);
            const { bytes, module } = compile(compiler, inferred);
            assert.deepEqual(bytes, expected);
            assert.equal(compiler.compileWat(inferred), wat);
            const events = [];
            const instance = new WebAssembly.Instance(module, { env: {
                mark(value) { events.push(value); return value; },
            } });
            assert.equal(instance.exports.numeric(), 25);
            assert.deepEqual(events, [1, 2, 3]);
            assert.equal(instance.exports.boolean(), 1);
            assert.equal(readDescriptor(instance, 'string', instance.exports.text()), 'é🦀');
            const a = instance.exports.items(), b = instance.exports.items();
            assert.notEqual(a, b);
            assert.deepEqual(readDescriptor(instance, 'number[]', a), [1, 2]);
            assert.deepEqual(readDescriptor(instance, 'number[]', b), [1, 2]);
            assert.deepEqual(readDescriptor(instance, 'number[]', instance.exports.empty()), []);
        }
    });

    test('inferred reassignment and void diagnostics retain locations and recover', compiler => {
        const valid = 'fn main() -> number { let n = 10; n = n / 4; return n; }';
        const cases = [
            ['fn main() -> void {\n    let n = 10;\n    n = true;\n}', 3,
                /Type mismatch: expected Number, found Bool/, 'n = true;'],
            ['fn main() -> void {\n    let value = noop();\n}\nfn noop() -> void {}', 2,
                /'value' cannot have type void/, 'let value = noop();'],
        ];
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            for (const [source, line, message, excerpt] of cases) {
                for (const method of ['compile', 'compileWat']) {
                    assert.throws(() => compiler[method](source), error => {
                        assert.ok(error instanceof Error);
                        assert.equal(error.name, 'Error');
                        assert.match(error.message, new RegExp(`^<browser>:${line}:5: semantic error:`));
                        assert.match(error.message, message);
                        assert.ok(error.message.includes(`${line} |     ${excerpt}`));
                        assert.match(error.message, /\| {5}\^/);
                        return true;
                    });
                    assert.equal(instantiate(compiler, valid).exports.main(), 2.5);
                    assert.match(compiler.compileWat(valid), /\(export "main"/);
                }
            }
        }
    });

    test('value blocks and if preserve concrete results, effects, transfers, errors, and parity', compiler => {
        compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'number');
        const source = `
            fn numeric(flag: bool) -> number {
                let base = { mark(1); let n = 4; n + 1 };
                return base + if { mark(2); flag } { mark(3) }
                    else if { mark(4); false } { mark(5) } else { mark(6) };
            }
            fn boolean(flag: bool) -> bool {
                return { if flag { true } else { false } };
            }
            fn text(flag: bool) -> string {
                return { if flag { "é🦀" } else { "other" } };
            }
            fn items(flag: bool) -> number[] {
                return { if flag { [mark(10), mark(11)] } else { [] } };
            }
            fn mutate(flag: bool, a: number[], b: number[]) -> number {
                let selected = if flag { a } else { b };
                selected[0] += { selected[0] = 99; mark(2) };
                return selected[0];
            }
            fn noop(flag: bool) -> void {
                {};
                (if flag { mark(40); } else { mark(41); });
            }
            fn early(flag: bool) -> number {
                let n = if flag { return mark(50); } else { mark(51) };
                return n + mark(1);
            }
            fn divergent(flag: bool) -> number {
                let n: bool = if flag { return mark(70); } else { return mark(71); };
            }
            fn skipped() -> number {
                let ignored = false && { mark(90); return 99; };
                let other = true || { mark(91); return 98; };
                return mark(7);
            }
            fn transfers() -> number {
                let i = 0;
                let total = 0;
                while i < 4 {
                    i += 1;
                    let value: number = {
                        let j = 0;
                        while j < 3 {
                            j += 1;
                            let inner: number = if j == 2 { continue; }
                                else if j == 3 { break; } else { mark(i * 10 + j) };
                            total += inner;
                        }
                        if i == 2 { continue; }
                        if i == 4 { break; }
                        i
                    };
                    total += value;
                }
                return total;
            }
        `;
        const cases = [
            ['fn main() -> number {\n    let value = if true { 1 };\n    return value;\n}',
                /An if expression requires an else branch/, 'let value = if true { 1 };', 5],
            ['fn main() -> number {\n    let value = { return 1; };\n}',
                /Cannot infer a concrete type for divergent initializer 'value'/, 'let value = { return 1; };', 5],
            ['fn main() -> void {\n    let value = { 1; };\n}',
                /'value' cannot have type void/, 'let value = { 1; };', 5],
            ['fn main() -> void {\n    let value = if true { 1 } else { false };\n}',
                /Type mismatch: expected Number, found Bool/, 'let value = if true { 1 } else { false };', 5],
            ['fn main() -> void {\n    let value = if true { 1 } else { missing };\n}',
                /Undefined variable 'missing'/, 'let value = if true { 1 } else { missing };', 36],
            ['fn main() -> number {\n    let value: number = { return 1; missing };\n}',
                /Undefined variable 'missing'/, 'let value: number = { return 1; missing };', 25],
            ['fn main() -> number {\n    let value = false && { missing };\n    return 1;\n}',
                /Undefined variable 'missing'/, 'let value = false && { missing };', 26],
        ];
        const expected = compile(compiler, source).bytes;
        const wat = compiler.compileWat(source);
        const diagnostics = new Map();
        for (const level of [0, 1, 2, 3]) {
            compiler.setOptimizationLevel(level);
            const { bytes, module } = compile(compiler, source);
            assert.deepEqual(bytes, expected);
            assert.equal(compiler.compileWat(source), wat);
            const events = [];
            const instance = new WebAssembly.Instance(module, { env: {
                mark(value) { events.push(value); return value; },
            } });
            const e = instance.exports;
            for (const [flag, result, effects] of [[1, 8, [1, 2, 3]], [0, 11, [1, 2, 4, 6]]]) {
                events.length = 0;
                assert.equal(e.numeric(flag), result);
                assert.deepEqual(events, effects);
                assert.equal(e.boolean(flag), flag);
                assert.equal(readDescriptor(instance, 'string', e.text(flag)), flag ? 'é🦀' : 'other');
            }
            events.length = 0;
            const a = e.items(1), b = e.items(1);
            assert.notEqual(a, b);
            assert.deepEqual(readDescriptor(instance, 'number[]', a), [10, 11]);
            assert.deepEqual(readDescriptor(instance, 'number[]', b), [10, 11]);
            assert.deepEqual(readDescriptor(instance, 'number[]', e.items(0)), []);
            assert.deepEqual(events, [10, 11, 10, 11]);
            events.length = 0;
            assert.equal(e.mutate(1, a, b), 12);
            assert.deepEqual(readDescriptor(instance, 'number[]', a), [12, 11]);
            assert.deepEqual(readDescriptor(instance, 'number[]', b), [10, 11]);
            assert.equal(e.mutate(0, a, b), 12);
            assert.deepEqual(readDescriptor(instance, 'number[]', b), [12, 11]);
            assert.deepEqual(events, [2, 2]);
            for (const [name, args, result, effects] of [
                ['noop', [1], undefined, [40]], ['noop', [0], undefined, [41]],
                ['early', [1], 50, [50]], ['early', [0], 52, [51, 1]],
                ['divergent', [1], 70, [70]], ['divergent', [0], 71, [71]],
                ['skipped', [], 7, [7]], ['transfers', [], 108, [11, 21, 31, 41]],
            ]) {
                events.length = 0;
                assert.equal(e[name](...args), result);
                assert.deepEqual(events, effects);
            }
            for (const [invalid, message, excerpt, column] of cases) {
                for (const method of ['compile', 'compileWat']) {
                    assert.throws(() => compiler[method](invalid), error => {
                        assert.ok(error instanceof Error);
                        assert.equal(error.name, 'Error');
                        assert.match(error.message, new RegExp(`^<browser>:2:${column}: semantic error:`));
                        assert.match(error.message, message);
                        assert.ok(error.message.includes(`2 |     ${excerpt}`));
                        assert.ok(error.message.includes(` |${' '.repeat(column)}^`));
                        if (diagnostics.has(invalid)) {
                            assert.equal(error.message, diagnostics.get(invalid));
                        } else {
                            diagnostics.set(invalid, error.message);
                        }
                        return true;
                    });
                    assert.deepEqual(compile(compiler, source).bytes, expected);
                    assert.equal(compiler.compileWat(source), wat);
                }
            }
            fails(() => compiler.compile('fn main() -> number { 1 }'), /parse error/);
            fails(() => compiler.compile('fn main() -> void { let n: never = { return; }; }'), /parse error/);
        }
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

    test('unterminated block comments retain original locations and compilation recovers', compiler => {
        const source = '// heading\r\nfn main() -> void {\r\n    /* é🦀 */ /* unterminated\nstill open';
        for (const method of ['compile', 'compileWat']) {
            fails(() => compiler[method](source), /^<browser>:3:14: parse error:/);
            fails(() => compiler[method](source), /3 \|     \/\* é🦀 \*\/ \/\* unterminated\n.*\| {14}\^/);
            assert.equal(instantiate(compiler, SIMPLE_SOURCE).exports.main(), 42);
            assert.match(compiler.compileWat(SIMPLE_SOURCE), /\(export "main"/);
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

    test('private ranges require explicit exports and allow contextual by host calls', compiler => {
        const source = `
            fn identity(value: range) -> range { return value; }
            fn main() -> number {
                let value = identity(1..=5 by by());
                return 65;
            }
        `;
        compiler.registerHostFunction('by', 'env', 'step', [], 'number');
        const privateRange = /code generation error: function `identity` has a private range signature.*explicit export selection.*ExportOptions::Only.*ExportOptions::None/;
        for (const method of ['compile', 'compileWat']) {
            fails(() => compiler[method](source), privateRange);
        }
        compiler.setExports(['main']);
        const selected = compile(compiler, source).module;
        assert.deepEqual(WebAssembly.Module.exports(selected), [
            { name: 'main', kind: 'function' },
            { name: 'memory', kind: 'memory' },
            { name: '__alloc', kind: 'function' },
        ]);
        assert.deepEqual(WebAssembly.Module.imports(selected), [
            { module: 'env', name: 'step', kind: 'function' },
        ]);
        let calls = 0;
        const instance = new WebAssembly.Instance(selected, { env: { step() { calls += 1; return 2; } } });
        assert.equal(instance.exports.main(), 65);
        assert.equal(calls, 1);
        assert.match(compiler.compileWat(source), /\(export "main"/);
        compiler.exportNone();
        const none = compile(compiler, source).module;
        assert.deepEqual(WebAssembly.Module.exports(none), [
            { name: 'memory', kind: 'memory' },
            { name: '__alloc', kind: 'function' },
        ]);
        assert.deepEqual(WebAssembly.Module.imports(none), WebAssembly.Module.imports(selected));
        assert.doesNotMatch(compiler.compileWat(source), /\(export "(?:main|identity)"/);
        compiler.exportAll();
        fails(() => compiler.compile(source), privateRange);
    });

    test('range host parameters, results, and reserved names are rejected atomically', compiler => {
        fails(() => compiler.registerHostFunction('candidate', 'env', 'run', ['range'], 'void'), /unknown type.*range/i);
        fails(() => compiler.registerHostFunction('candidate', 'env', 'run', [], 'range'), /unknown type.*range/i);
        fails(() => compiler.registerHostFunction('range', 'env', 'run', [], 'void'), /reserved/i);
        compiler.registerHostFunction('candidate', 'env', 'run', [], 'number');
        const instance = instantiate(compiler, 'fn main() -> number { return candidate(); }', { env: { run: () => 17 } });
        assert.equal(instance.exports.main(), 17);
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
