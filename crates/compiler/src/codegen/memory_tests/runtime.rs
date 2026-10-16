use std::{process::Command, time::Duration};

use super::*;

const NODE_PRELUDE: &str = r#"
const assert = require('node:assert/strict');
const bytes = require('node:fs').readFileSync(0);
const module_ = new WebAssembly.Module(bytes);
let e;
const view = () => new DataView(e.memory.buffer);
function descriptor(data, length) {
    const h = e.__alloc(8) >>> 0;
    view().setUint32(h, data, true);
    view().setUint32(h + 4, length, true);
    return h;
}
function string(value) {
    const bytes = new TextEncoder().encode(value);
    const data = e.__alloc(bytes.length) >>> 0;
    const h = descriptor(data, bytes.length);
    new Uint8Array(e.memory.buffer, data, bytes.length).set(bytes);
    return h;
}
function array(values) {
    const data = e.__alloc(values.length * 8) >>> 0;
    const h = descriptor(data, values.length);
    for (let i = 0; i < values.length; i++) view().setFloat64(data + i * 8, values[i], true);
    return h;
}
function readString(h) {
    const data = view().getUint32(h, true), length = view().getUint32(h + 4, true);
    return new TextDecoder('utf-8', {fatal: true}).decode(new Uint8Array(e.memory.buffer, data, length));
}
function readArray(h) {
    const data = view().getUint32(h, true), length = view().getUint32(h + 4, true);
    return Array.from({length}, (_, i) => view().getFloat64(data + i * 8, true));
}
function trap(f) { assert.throws(f, WebAssembly.RuntimeError); }
"#;

fn run_node(bytes: &[u8], script: &str) {
    let script = format!("{NODE_PRELUDE}\n{script}");
    let output = crate::test_process::output(
        Command::new("node").args(["-e", &script]),
        Some(bytes),
        Duration::from_secs(15),
    );
    assert!(
        output.status.success(),
        "Node.js failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_at_all_levels(source: &str, hosts: &[HostFunction], script: &str) {
    let mut snapshot = None;
    for level in LEVELS {
        let bytes = compiler(level, hosts)
            .compile_source(source)
            .expect("compile memory runtime");
        decode(&bytes);
        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }
        run_node(&bytes, script);
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_arrays_sum_len_mutation_alias_reassignment_and_fresh_literals() {
    let source = r#"
        fn sum(a: number[]) -> number {
            let i: number = 0; let total: number = 0;
            while i < len(a) { total = total + a[i]; i = i + 1; }
            return total;
        }
        fn sample() -> number { return sum([10, 20, 30]); }
        fn empty() -> number[] { return []; }
        fn alias() -> number {
            let a: number[] = [1, 2]; let b: number[] = a;
            b[0] = 7; a = [9]; a[0] = 10;
            return b[0] + b[1] + a[0];
        }
        fn make() -> number[] { return [1, 2]; }
        fn repeat() -> void {
            let i: number = 0;
            while i < 4 { let a: number[] = [i]; seen(a); a[0] = 99; i = i + 1; }
        }
        fn recurse(n: number) -> number[] {
            if n <= 0 { return [1]; }
            let a: number[] = recurse(n - 1); a[0] = a[0] + 1; return a;
        }
        fn fresh_recursion(n: number) -> void {
            let a: number[] = [n]; seen(a);
            if n > 0 { fresh_recursion(n - 1); }
            a[0] = a[0] + 10;
        }
        fn nested() -> number { return sum(recurse(3)); }
    "#;
    run_at_all_levels(
        source,
        &[host(
            0,
            "seen",
            &[Type::Array(crate::ast::ArrayElementType::Number)],
            Type::Void,
        )],
        r#"
        const handles = [], values = [];
        e = new WebAssembly.Instance(module_, {env: {seen(h) {
            handles.push(h); values.push(readArray(h));
        }}}).exports;
        assert.equal(e.sample(), 60);
        assert.equal(e.sum(array([])), 0);
        const empty = e.empty();
        assert.deepEqual(readArray(empty), []);
        assert.equal(e.sum(empty), 0);
        assert.equal(e.alias(), 19);
        const first = e.make(), second = e.make();
        assert.notEqual(first, second);
        view().setFloat64(view().getUint32(first, true), 99, true);
        assert.deepEqual(readArray(second), [1, 2]);
        e.repeat();
        assert.deepEqual(values, [[0], [1], [2], [3]]);
        e.fresh_recursion(3);
        assert.deepEqual(values.slice(4), [[3], [2], [1], [0]]);
        assert.equal(new Set(handles).size, handles.length);
        for (const h of handles.slice(0, 4)) assert.deepEqual(readArray(h), [99]);
        assert.deepEqual(handles.slice(4).map(readArray), [[13], [12], [11], [10]]);
        assert.deepEqual(readArray(e.recurse(4)), [5]);
        assert.equal(e.nested(), 4);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_string_utf8_empty_nul_interning_and_distinct_descriptor_content_equality() {
    let source = "fn text() -> string { return \"é🙂\"; }\n\
        fn repeat() -> string { return \"é🙂\"; }\n\
        fn empty() -> string { return \"\"; }\n\
        fn nul() -> string { return \"A\0B\"; }\n\
        fn same(a: string, b: string) -> bool { return a == b; }\n\
        fn different(a: string, b: string) -> bool { return a != b; }\n\
        fn identity(a: string) -> string { return a; }";
    run_at_all_levels(
        source,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        assert.equal(readString(e.text()), 'é🙂');
        assert.equal(e.text(), e.repeat());
        assert.equal(view().getUint32(e.text() + 4, true), 6);
        assert.equal(readString(e.empty()), '');
        assert.equal(readString(e.nul()), 'A\0B');
        for (const value of ['', 'é🙂', 'A\0B']) {
            const a = string(value), b = string(value);
            assert.notEqual(a, b);
            assert.equal(e.same(a, b), 1);
            assert.equal(e.different(a, b), 0);
            assert.equal(e.identity(a), a);
        }
        assert.equal(e.same(string('é'), string('e')), 0);
        assert.equal(e.same(string('A\0B'), string('A\0C')), 0);
        assert.equal(e.same(string('A'), string('AA')), 0);
        assert.equal(e.different(string('é🙂'), e.empty()), 1);
        assert.equal(e.same(e.nul(), string('A\0B')), 1);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_all_invalid_indices_trap_and_negative_zero_is_valid_for_reads_and_writes() {
    run_at_all_levels(
        r#"
        fn get(a: number[], i: number) -> number { return a[i]; }
        fn set(a: number[], i: number, value: number) -> void { a[i] = value; }
    "#,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const a = array([10, 20]);
        for (const i of [NaN, Infinity, -Infinity, -1, -0.5, 0.5, 1.5, 2, 3, 4294967296, Number.MAX_VALUE]) {
            trap(() => e.get(a, i)); trap(() => e.set(a, i, 99));
            assert.deepEqual(readArray(a), [10, 20]);
        }
        assert.equal(e.get(a, -0), 10);
        e.set(a, -0, 7);
        assert.equal(e.get(a, 0), 7);
        e.set(a, 1, -Infinity);
        assert.equal(e.get(a, 1), -Infinity);
        const empty = array([]);
        trap(() => e.get(empty, 0)); trap(() => e.set(empty, -0, 99));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_host_memory_roundtrips_reentrant_allocator_growth_and_reference_mutation() {
    let hosts = [
        host(0, "text_host", &[Type::String], Type::String),
        host(
            1,
            "array_host",
            &[Type::Array(crate::ast::ArrayElementType::Number)],
            Type::Array(crate::ast::ArrayElementType::Number),
        ),
    ];
    run_at_all_levels(
        r#"
        fn text(a: string) -> string { return text_host(a); }
        fn values(a: number[]) -> number[] { return array_host(a); }
        fn sample() -> string { return text_host("guest"); }
    "#,
        &hosts,
        r#"
        let textCalls = 0, arrayCalls = 0;
        e = new WebAssembly.Instance(module_, {env: {
            text_host(h) {
                const value = readString(h), oldBuffer = e.memory.buffer;
                e.__alloc(131072);
                assert.notEqual(e.memory.buffer, oldBuffer);
                textCalls++;
                return string(value + 'é\0🙂');
            },
            array_host(h) {
                const values = readArray(h);
                e.__alloc(131072);
                const data = view().getUint32(h, true);
                if (values.length) view().setFloat64(data, 42, true);
                arrayCalls++;
                return array(values.map(x => x + 1));
            }
        }}).exports;
        assert.equal(readString(e.sample()), 'guesté\0🙂');
        assert.equal(readString(e.text(string('host'))), 'hosté\0🙂');
        assert.equal(readString(e.text(string(''))), 'é\0🙂');
        const a = array([1, 2, 3]);
        assert.deepEqual(readArray(e.values(a)), [2, 3, 4]);
        assert.deepEqual(readArray(a), [42, 2, 3]);
        assert.deepEqual(readArray(e.values(array([]))), []);
        assert.equal(textCalls, 3); assert.equal(arrayCalls, 2);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_allocator_alignment_reservation_growth_overflow_and_large_arrays() {
    run_at_all_levels(
        r#"
        fn len_array(a: number[]) -> number { return len(a); }
        fn get(a: number[], i: number) -> number { return a[i]; }
        fn marker() -> string { return "protected"; }
    "#,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const marker = e.marker();
        let previousEnd = marker + 8 + 9;
        for (const bytes of [0, 0, 1, 7, 8, 9, 63, 65536, 131072]) {
            const p = e.__alloc(bytes) >>> 0;
            assert.equal(p % 8, 0); assert.ok(p >= previousEnd);
            previousEnd = p + Math.max(8, Math.ceil(bytes / 8) * 8);
            assert.ok(previousEnd <= e.memory.buffer.byteLength);
        }
        assert.equal(readString(marker), 'protected');
        const values = Array.from({length: 20000}, (_, i) => i + 0.25);
        const a = array(values);
        assert.equal(e.len_array(a), 20000);
        assert.equal(e.get(a, 0), 0.25);
        assert.equal(e.get(a, 19999), 19999.25);
        const before = e.memory.buffer.byteLength;
        for (const bytes of [-1, -8, 0xfffffff0, 0xffff0000]) trap(() => e.__alloc(bytes));
        assert.equal(e.memory.buffer.byteLength, before);
        assert.equal(readString(marker), 'protected');
        assert.deepEqual(readArray(a), values);
        assert.ok((e.__alloc(8) >>> 0) >= previousEnd);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_guest_large_array_growth_and_bounded_allocator_exhaustion() {
    let elements = (0..9000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("fn large() -> number[] {{ return [{elements}]; }}");
    run_at_all_levels(
        &source,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const initial = e.memory.buffer.byteLength;
        const h = e.large();
        assert.ok(e.memory.buffer.byteLength > initial);
        const values = readArray(h);
        assert.equal(values.length, 9000);
        assert.equal(values[0], 0); assert.equal(values[8999], 8999);
    "#,
    );
    let bytes = compile_source("fn empty() -> number[] { return []; }");
    let initial = decode(&bytes).memories[0].0;
    let mut capped = wasm_encoder::Module::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        let payload = payload.unwrap();
        if let Some((id, range)) = payload.as_section() {
            if id == 5 {
                let mut memory = wasm_encoder::MemorySection::new();
                memory.memory(wasm_encoder::MemoryType {
                    minimum: initial,
                    maximum: Some(initial),
                    memory64: false,
                    shared: false,
                    page_size_log2: None,
                });
                capped.section(&memory);
            } else {
                let start = usize::try_from(range.start).expect("section start fits usize");
                let end = usize::try_from(range.end).expect("section end fits usize");
                capped.section(&wasm_encoder::RawSection {
                    id,
                    data: &bytes[start..end],
                });
            }
        }
    }
    let bytes = capped.finish();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const p = e.__alloc(8) >>> 0, before = e.memory.buffer.byteLength;
        trap(() => e.__alloc(65536));
        assert.equal(e.memory.buffer.byteLength, before);
        assert.equal(e.__alloc(8) >>> 0, p + 8);
        assert.deepEqual(readArray(e.empty()), []);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_invalid_exported_and_host_descriptors_trap_before_effects() {
    let hosts = [
        host(0, "provide_string", &[], Type::String),
        host(
            1,
            "provide_array",
            &[],
            Type::Array(crate::ast::ArrayElementType::Number),
        ),
        host(2, "consume_string", &[Type::String], Type::Void),
        host(
            3,
            "consume_array",
            &[Type::Array(crate::ast::ArrayElementType::Number)],
            Type::Void,
        ),
        host(4, "effect", &[], Type::Number),
    ];
    run_at_all_levels(
        r#"
        fn ignore_string(a: string) -> number { return effect(); }
        fn ignore_array(a: number[]) -> number { return effect(); }
        fn text() -> string { return provide_string(); }
        fn values() -> number[] { return provide_array(); }
        fn send_string(a: string) -> void { consume_string(a); }
        fn send_array(a: number[]) -> void { consume_array(a); }
        fn same(a: string, b: string) -> bool { return a == b; }
        fn unused_string_result() -> void { provide_string(); }
        fn unused_array_result() -> void { provide_array(); }
        fn after_mutation(a: number[]) -> void { effect(); consume_array(a); }
        fn after_string_mutation(a: string) -> void { effect(); consume_string(a); }
    "#,
        &hosts,
        r#"
        let supplied = 0, effects = 0, consumed = 0, mutate = 0;
        e = new WebAssembly.Instance(module_, {env: {
            provide_string() { return supplied; }, provide_array() { return supplied; },
            consume_string() { consumed++; }, consume_array() { consumed++; },
            effect() { effects++; if (mutate) view().setUint32(mutate + 4, 0xffffffff, true); return 1; }
        }}).exports;
        const end = e.memory.buffer.byteLength;
        const invalidHandles = [0, 1, 7, 9, -1, end, end - 4, end + 8];
        const badData = [
            descriptor(0, 0), descriptor(7, 1),
            descriptor(end + 8, 0), descriptor(end, 1), descriptor(end - 1, 2),
            descriptor(0xfffffff8, 16), descriptor(8, 0xffffffff),
        ];
        const validString = string('valid');
        for (const h of [...invalidHandles, ...badData]) {
            supplied = h;
            trap(() => e.ignore_string(h)); trap(() => e.ignore_array(h));
            trap(() => e.text()); trap(() => e.values());
            trap(() => e.send_string(h)); trap(() => e.send_array(h));
            trap(() => e.same(h, h)); trap(() => e.same(validString, h));
            trap(() => e.unused_string_result()); trap(() => e.unused_array_result());
        }
        const unalignedArray = descriptor(17, 1);
        supplied = unalignedArray;
        trap(() => e.ignore_array(unalignedArray)); trap(() => e.values());
        trap(() => e.send_array(unalignedArray));
        trap(() => e.ignore_array(descriptor(17, 0)));
        assert.equal(e.same(unalignedArray, unalignedArray), 1);
        assert.equal(effects, 0); assert.equal(consumed, 0);
        assert.equal(e.ignore_string(descriptor(end, 0)), 1);
        assert.equal(e.ignore_array(descriptor(end, 0)), 1);
        const a = array([1]); mutate = a;
        trap(() => e.after_mutation(a));
        assert.equal(effects, 3); assert.equal(consumed, 0);
        mutate = string('safe');
        trap(() => e.after_string_mutation(mutate));
        assert.equal(effects, 4); assert.equal(consumed, 0);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_memory_expression_order_live_prefixes_and_assignment_bounds_before_rhs() {
    let hosts = [
        host(0, "mark", &[Type::Number], Type::Number),
        host(
            1,
            "base",
            &[],
            Type::Array(crate::ast::ArrayElementType::Number),
        ),
        host(2, "position", &[], Type::Number),
        host(3, "rhs", &[], Type::Number),
    ];
    run_at_all_levels(
        r#"
        fn identity(a: number[]) -> number[] { return a; }
        fn combine(n: number, a: number[], s: string, b: bool) -> number {
            if b { return n + len(a); } else { return n; }
        }
        fn build() -> number[] { return [mark(1), mark(2), mark(3)]; }
        fn nested_array() -> number[] { return [len([mark(4), mark(5)]), mark(6)]; }
        fn read() -> number { let a: number[] = base(); return a[position()]; }
        fn write(a: number[]) -> void { a[position()] = rhs(); }
        fn live(a: number[]) -> number {
            return mark(10) + combine(mark(20), identity([mark(1), len(identity(a))]), "x", "x" == "x");
        }
    "#,
        &hosts,
        r#"
        const events = [];
        let supplied, position = 0, replacement = 0;
        e = new WebAssembly.Instance(module_, {env: {
            mark(n) { events.push(n); return n; },
            base() { events.push('base'); return supplied; },
            position() { events.push('index'); return position; },
            rhs() {
                events.push('rhs');
                if (replacement) view().setUint32(supplied, replacement, true);
                e.__alloc(131072);
                return 77;
            }
        }}).exports;
        assert.deepEqual(readArray(e.build()), [1, 2, 3]);
        assert.deepEqual(events.splice(0), [1, 2, 3]);
        assert.deepEqual(readArray(e.nested_array()), [2, 6]);
        assert.deepEqual(events.splice(0), [4, 5, 6]);
        supplied = array([4, 5]);
        assert.equal(e.read(), 4);
        assert.deepEqual(events.splice(0), ['base', 'index']);
        position = 2;
        trap(() => e.write(supplied));
        assert.deepEqual(events.splice(0), ['index']);
        position = 1;
        const originalData = view().getUint32(supplied, true);
        replacement = e.__alloc(16) >>> 0;
        view().setFloat64(replacement + 8, 123, true);
        e.write(supplied);
        assert.deepEqual(events.splice(0), ['index', 'rhs']);
        assert.equal(view().getFloat64(originalData + 8, true), 77);
        assert.equal(view().getFloat64(replacement + 8, true), 123);
        assert.equal(e.live(array([9, 8, 7])), 32);
        assert.deepEqual(events.splice(0), [10, 20, 1]);
    "#,
    );
}
