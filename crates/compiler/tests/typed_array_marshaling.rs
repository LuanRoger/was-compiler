#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::Path, process::Command, time::Duration};

const PRELUDE: &str = r#"
'use strict';
const assert = require('node:assert/strict');
const {createAbi, readDescriptor, MAX_COPY_BYTES} = require('./runtime/abi.cjs');
function host(initial = 1, growEveryTime = false) {
    const memory = new WebAssembly.Memory({initial});
    let heap = 8;
    const abi = createAbi(() => ({exports: {memory, __alloc(size) {
        const handle = heap;
        heap += Math.ceil(size / 8) * 8;
        if (growEveryTime) memory.grow(1);
        if (heap > memory.buffer.byteLength) memory.grow(Math.ceil((heap - memory.buffer.byteLength) / 65536));
        return handle;
    }}}));
    return {abi, memory};
}
function header(memory, handle, data, length) {
    const view = new DataView(memory.buffer);
    view.setUint32(handle, data, true);
    view.setUint32(handle + 4, length, true);
}
"#;

fn node(body: &str) {
    node_with_wasm(body, None);
}

fn node_with_wasm(body: &str, wasm: Option<&[u8]>) {
    let mut command = Command::new("node");
    command
        .current_dir(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")))
        .args(["-e", &format!("{PRELUDE}\n{body}")]);
    let output = test_process::output(&mut command, wasm, Duration::from_secs(60));
    assert!(
        output.status.success(),
        "Node failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
#[ignore = "requires Node.js"]
fn bool_arrays_copy_canonical_bytes_and_reject_wrong_native_values() {
    node(
        r#"
        const {abi, memory} = host();
        for (const value of [[], [false], [true], [true, false, true]]) {
            assert.deepEqual(abi.decode('bool[]', abi.encode('bool[]', Object.freeze(value))), value);
        }
        const input = [true, false];
        const handle = abi.encode('bool[]', input);
        assert.equal(handle % 8, 0);
        const {data} = readDescriptor(memory, handle, 1);
        assert.deepEqual(Array.from(new Uint8Array(memory.buffer, handle, 10)),
            [data, 0, 0, 0, 2, 0, 0, 0, 1, 0]);
        input[0] = false;
        const first = abi.decode('bool[]', handle);
        const second = abi.decode('bool[]', handle);
        first[0] = false;
        assert.deepEqual(second, [true, false]);
        new Uint8Array(memory.buffer)[data + 1] = 1;
        assert.deepEqual(second, [true, false]);
        assert.deepEqual(abi.decode('bool[]', handle), [true, true]);
        for (const value of [null, true, 'true', new Uint8Array([1]), {0: true, length: 1}]) {
            assert.throws(() => abi.encode('bool[]', value, 'flags'), /flags must be bool\[\]/);
        }
        for (const value of [0, 1, '', null, undefined, [], new Boolean(true)]) {
            assert.throws(() => abi.encode('bool[]', [true, value], 'flags'), /flags\[1\] must be bool/);
        }
        assert.throws(() => abi.encode('bool[]', new Array(1)), /value\[0\] must be bool/);
        header(memory, 8, 101, 2);
        new Uint8Array(memory.buffer, 101, 2).set([0, 1]);
        assert.deepEqual(abi.decode('bool[]', 8), [false, true]);
        for (const byte of [2, 127, 255]) {
            new Uint8Array(memory.buffer)[102] = byte;
            assert.throws(() => abi.decode('bool[]', 8, 'flags'), /flags\[1\] must be ABI bool \(0 or 1\)/);
        }
        assert.equal(abi.encode('bool', true), 1);
        assert.equal(abi.decode('bool', 1), true);
        assert.throws(() => abi.decode('bool', 2), /ABI bool/);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn string_arrays_copy_utf8_handles_and_refresh_after_nested_growth() {
    node(
        r#"
        const {abi, memory} = host(1, true);
        for (const input of [[], [''], ['hé\u0000🦀', '\ufeffBOM', '', 'same', 'same']]) {
            const before = memory.buffer;
            const handle = abi.encode('string[]', Object.freeze(input));
            assert.notEqual(memory.buffer, before);
            assert.deepEqual(abi.decode('string[]', handle), input);
        }
        const input = ['first', 'x'.repeat(100000), 'last'];
        const handle = abi.encode('string[]', input);
        input[0] = 'changed';
        const first = abi.decode('string[]', handle);
        const second = abi.decode('string[]', handle);
        first[0] = 'native';
        assert.equal(second[0], 'first');
        const replacement = abi.encode('string', 'guest');
        const outer = readDescriptor(memory, handle, 4);
        outer.view.setUint32(outer.data, replacement, true);
        assert.equal(second[0], 'first');
        assert.equal(abi.decode('string[]', handle)[0], 'guest');
        for (const value of [null, '', true, new Uint32Array(1), {0: '', length: 1}]) {
            assert.throws(() => abi.encode('string[]', value, 'texts'), /texts must be string\[\]/);
        }
        for (const value of [1, false, null, undefined, [], new String('s')]) {
            assert.throws(() => abi.encode('string[]', ['', value], 'texts'), /texts\[1\] must be string/);
        }
        assert.throws(() => abi.encode('string[]', new Array(1)), /value\[0\] must be string/);
        const golden = host();
        assert.equal(golden.abi.encode('string[]', ['A', 'é']), 8);
        assert.deepEqual(Array.from(new Uint8Array(golden.memory.buffer, 8, 12)),
            [16, 0, 0, 0, 2, 0, 0, 0, 24, 0, 0, 0]);
        assert.equal(new DataView(golden.memory.buffer).getUint32(20, true), 40);
        assert.deepEqual(Array.from(new Uint8Array(golden.memory.buffer, 24, 9)),
            [32, 0, 0, 0, 1, 0, 0, 0, 65]);
        assert.deepEqual(Array.from(new Uint8Array(golden.memory.buffer, 40, 10)),
            [48, 0, 0, 0, 2, 0, 0, 0, 0xc3, 0xa9]);
        header(memory, 8, 100, 2);
        header(memory, 24, 151, 2);
        new Uint8Array(memory.buffer, 151, 2).set([0xc3, 0xa9]);
        let view = new DataView(memory.buffer);
        view.setUint32(100, 24, true);
        view.setUint32(104, 24, true);
        assert.deepEqual(abi.decode('string[]', 8), ['é', 'é']);
        header(memory, 8, 101, 2);
        assert.throws(() => abi.decode('string[]', 8), /payload.*4-byte aligned/);
        header(memory, 8, 100, 2);
        for (const invalid of [0, 9, memory.buffer.byteLength]) {
            view.setUint32(104, invalid, true);
            assert.throws(() => abi.decode('string[]', 8), /descriptor/);
        }
        view.setUint32(104, 24, true);
        header(memory, 24, memory.buffer.byteLength, 1);
        assert.throws(() => abi.decode('string[]', 8), /payload.*out of bounds/);
        header(memory, 24, 151, 2);
        new Uint8Array(memory.buffer, 151, 2).set([0xc3, 0x28]);
        assert.throws(() => abi.decode('string[]', 8), /encoded data/);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn accessor_arrays_encode_each_initial_element_once_at_a_fixed_length() {
    node(
        r#"
        const {abi, memory} = host();
        for (const [type, first, unchecked] of [
            ['bool[]', true, 'false'],
            ['string[]', '', 'x'.repeat(34 * 1024 * 1024)]
        ]) {
            const input = new Array(2);
            const reads = [0, 0];
            for (let i = 0; i < 2; i++) {
                Object.defineProperty(input, i, {get() {
                    return ++reads[i] === 1 ? first : unchecked;
                }});
            }
            const handle = abi.encode(type, input);
            assert.deepEqual(abi.decode(type, handle), [first, first]);
            assert.deepEqual(reads, [1, 1]);
        }
        for (const [type, scalar] of [['bool[]', false], ['string[]', 'kept']]) {
            let elementReads = 0;
            let lengthReads = 0;
            const input = new Array(1);
            Object.defineProperty(input, 0, {get() {
                elementReads++;
                input.length = 0xffffffff;
                input[1] = {unchecked: true};
                return scalar;
            }});
            const observed = new Proxy(input, {get(target, key) {
                if (key === 'length') lengthReads++;
                return Reflect.get(target, key);
            }});
            const handle = abi.encode(type, observed);
            assert.deepEqual(abi.decode(type, handle), [scalar]);
            assert.equal(readDescriptor(memory, handle, type === 'bool[]' ? 1 : 4).length, 1);
            assert.equal(elementReads, 1);
            assert.equal(lengthReads, 1);
            const shrunk = [scalar, scalar];
            Object.defineProperty(shrunk, 0, {get() {
                shrunk.length = 1;
                return scalar;
            }});
            const unavailable = createAbi(() => { throw Error('must validate missing initial element'); });
            assert.throws(() => unavailable.encode(type, shrunk), /value\[1\] must be/);
        }
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn accessor_string_budgets_validate_captured_values_before_guest_allocation() {
    node(
        r#"
        let instanceReads = 0;
        let allocations = 0;
        const memory = new WebAssembly.Memory({initial: 1});
        const abi = createAbi(() => {
            instanceReads++;
            return {exports: {memory, __alloc() { allocations++; throw Error('unexpected allocation'); }}};
        });
        const oversized = 'x'.repeat(34 * 1024 * 1024);
        const input = new Array(2);
        const reads = [0, 0];
        for (let i = 0; i < 2; i++) {
            Object.defineProperty(input, i, {get() {
                return ++reads[i] === 1 ? oversized : '';
            }});
        }
        assert.throws(() => abi.encode('string[]', input), /67108864-byte runtime copy limit/);
        assert.deepEqual(reads, [1, 1]);
        assert.equal(instanceReads, 0);
        assert.equal(allocations, 0);
        // Slots count toward the budget even when one string is just under the standalone limit.
        const single = new Array(1);
        let singleReads = 0;
        Object.defineProperty(single, 0, {get() {
            singleReads++;
            return 'x'.repeat(MAX_COPY_BYTES - 3);
        }});
        assert.throws(() => abi.encode('string[]', single), /copy limit/);
        assert.equal(singleReads, 1);
        assert.equal(instanceReads, 0);
        assert.equal(allocations, 0);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js; copies up to the 64 MiB logical budget"]
fn aggregate_copy_budgets_count_every_utf8_occurrence_before_copying() {
    node(
        r#"
        const {abi, memory} = host(1025);
        const limit = /67108864-byte runtime copy limit/;
        const unavailable = createAbi(() => { throw Error('must reject before memory access'); });
        for (const [type, length] of [['bool[]', 67108865], ['string[]', 16777217]]) {
            const oversized = new Array(length);
            Object.defineProperty(oversized, 0, {get() { throw Error('must check size before elements'); }});
            assert.throws(() => unavailable.encode(type, oversized), limit);
            header(memory, 8, 16, length);
            assert.throws(() => abi.decode(type, 8), limit);
        }
        // At the exact boolean limit, element validation (not the budget) decides rejection.
        const boundaryFlags = new Array(67108864);
        assert.throws(() => unavailable.encode('bool[]', boundaryFlags), /value\[0\] must be bool/);
        header(memory, 8, 16, 67108864);
        new Uint8Array(memory.buffer)[16] = 2;
        assert.throws(() => abi.decode('bool[]', 8), /value\[0\] must be ABI bool/);
        const text = 'é'.repeat(16777214); // 33,554,428 UTF-8 bytes, not JS code units.
        const exactInput = new Array(2);
        const exactReads = [0, 0];
        for (let i = 0; i < 2; i++) {
            Object.defineProperty(exactInput, i, {get() {
                return ++exactReads[i] === 1 ? text : '';
            }});
        }
        const exact = abi.encode('string[]', exactInput);
        assert.deepEqual(exactReads, [1, 1]);
        const decoded = abi.decode('string[]', exact);
        assert.equal(decoded.length, 2);
        assert.equal(decoded[0], text);
        assert.equal(decoded[1], text);
        assert.throws(() => unavailable.encode('string[]', [text, text + 'x']), limit);
        assert.throws(() => unavailable.encode('string[]', [text, text, '']), limit);
        // A tiny outer payload can repeat one large string handle: charge both occurrences.
        const string = abi.encode('string', text);
        header(memory, 8, 100, 2);
        let view = new DataView(memory.buffer);
        view.setUint32(100, string, true);
        view.setUint32(104, string, true);
        assert.deepEqual(abi.decode('string[]', 8), [text, text]);
        const data = view.getUint32(string, true);
        header(memory, 24, data, 33554429);
        view.setUint32(104, 24, true);
        // Invalid UTF-8 in the first string must not be decoded before the budget preflight.
        new Uint8Array(memory.buffer)[data] = 0xff;
        assert.throws(() => abi.decode('string[]', 8), limit);
        header(memory, 8, 100, 3);
        view.setUint32(104, string, true);
        view.setUint32(108, string, true);
        assert.throws(() => abi.decode('string[]', 8), limit);
        // Empty strings still cost four bytes each, and an operation gets a fresh budget.
        const empty = abi.encode('string[]', ['', '', '']);
        assert.deepEqual(abi.decode('string[]', empty), ['', '', '']);
        assert.deepEqual(abi.decode('string[]', empty), ['', '', '']);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn explicit_marshaling_roundtrips_through_raw_compiled_host_imports() {
    use was_compiler::{ArrayElementType, Compiler, Type};

    let mut compiler = Compiler::new();
    for (name, element) in [
        ("flags_host", ArrayElementType::Bool),
        ("texts_host", ArrayElementType::String),
    ] {
        compiler
            .register_host_function(
                name,
                "env",
                name,
                vec![Type::Array(element)],
                Type::Array(element),
            )
            .unwrap();
    }
    compiler
        .register_host_function("truth", "env", "truth", vec![], Type::Bool)
        .unwrap();
    let program = compiler
        .analyze(
            r#"
        fn flags(value: bool[]) -> bool[] { value[0] = false; return flags_host(value); }
        fn texts(value: string[]) -> string[] { value[0] = "guest"; return texts_host(value); }
        fn normalized() -> bool { return truth(); }
        fn consume(value: bool[]) -> bool { return value[0]; }
    "#,
        )
        .unwrap();
    let wasm = compiler.compile(&program).unwrap();
    wasmparser::Validator::new().validate_all(&wasm).unwrap();
    node_with_wasm(
        r#"
        const {instantiate} = require('./runtime/index.cjs');
        const wasm = require('node:fs').readFileSync(0);
        let runtime;
        const abi = createAbi(() => runtime.instance);
        let retained;
        runtime = instantiate(wasm, {env: {
            flags_host(handle) {
                assert.equal(typeof handle, 'number');
                retained = abi.decode('bool[]', handle);
                assert.deepEqual(retained, [false, true]);
                retained[0] = true;
                return abi.encode('bool[]', retained);
            },
            texts_host(handle) {
                assert.equal(typeof handle, 'number');
                const values = abi.decode('string[]', handle);
                assert.deepEqual(values, ['guest', 'é']);
                values[1] = 'x'.repeat(100000);
                return abi.encode('string[]', values);
            },
            truth() { return 255; }
        }});
        const flags = [true, true];
        const original = abi.encode('bool[]', flags);
        const returned = runtime.call('flags', [original]);
        assert.notEqual(returned, original);
        assert.deepEqual(abi.decode('bool[]', original), [false, true]);
        assert.deepEqual(abi.decode('bool[]', returned), [true, true]);
        retained[0] = false;
        assert.deepEqual(abi.decode('bool[]', returned), [true, true]);
        assert.deepEqual(flags, [true, true]);
        const texts = ['native', 'é'];
        const textHandle = abi.encode('string[]', texts);
        const before = runtime.instance.exports.memory.buffer;
        const result = runtime.call('texts', [textHandle]);
        assert.notEqual(runtime.instance.exports.memory.buffer, before);
        assert.deepEqual(abi.decode('string[]', result), ['guest', 'x'.repeat(100000)]);
        assert.deepEqual(abi.decode('string[]', textHandle), ['guest', 'é']);
        assert.deepEqual(texts, ['native', 'é']);
        assert.equal(abi.decode('bool', runtime.call('normalized')), true);
        const memory = runtime.instance.exports.memory;
        const raw = runtime.call('__alloc', [32]);
        header(memory, raw, raw + 17, 1);
        new Uint8Array(memory.buffer)[raw + 17] = 255;
        assert.throws(() => abi.decode('bool[]', raw), /ABI bool/);
        assert.throws(() => runtime.call('consume', [raw]), error => error.stage === 'invocation');
        // Raw allocation remains byte allocation, not a typed-array constructor.
        assert.equal(raw % 8, 0);
        assert.equal(runtime.call('__alloc', [1]) % 8, 0);
    "#,
        Some(&wasm),
    );
}
