#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type};
use wasmparser::{ExternalKind, Parser, Payload, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("unused", "requires", "requires", vec![], Type::Void)
        .unwrap();
    compiler
}

fn run(source: &str, assertions: &str) {
    let mut compiler = compiler();
    let mut baseline = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        if let Some(expected) = &baseline {
            assert_eq!(&bytes, expected, "{level:?}");
        } else {
            baseline = Some(bytes.clone());
        }
        let indices = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|payload| match payload.unwrap() {
                Payload::ExportSection(reader) => Some(reader),
                _ => None,
            })
            .flatten()
            .map(Result::unwrap)
            .filter(|export| export.kind == ExternalKind::Func)
            .map(|export| format!("[{:?}, {}]", export.name, export.index))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const module = new WebAssembly.Module(bytes);
            assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
                [['env', 'mark'], ['requires', 'requires']]);
            const functionIndices = new Map([{indices}]);
            const events = [];
            let corruptHandle = 0;
            let e;
            e = new WebAssembly.Instance(module, {{
                env: {{ mark(n) {{
                    events.push(n);
                    if (n === 90 && corruptHandle) {{
                        new DataView(e.memory.buffer).setUint32(corruptHandle + 4, 0xffffffff, true);
                    }}
                    return n;
                }} }},
                requires: {{ requires() {{ throw new Error('unused import executed'); }} }}
            }}).exports;
            function array(values) {{
                const h = e.__alloc(8 + values.length * 8) >>> 0;
                const view = new DataView(e.memory.buffer);
                view.setUint32(h, h + 8, true);
                view.setUint32(h + 4, values.length, true);
                values.forEach((v, i) => view.setFloat64(h + 8 + i * 8, v, true));
                return h;
            }}
            function text(value) {{
                const bytes = Buffer.from(value, 'utf8');
                const h = e.__alloc(8 + bytes.length) >>> 0;
                const view = new DataView(e.memory.buffer);
                view.setUint32(h, h + 8, true);
                view.setUint32(h + 4, bytes.length, true);
                new Uint8Array(e.memory.buffer, h + 8, bytes.length).set(bytes);
                return h;
            }}
            function trap(call, expected = []) {{
                assert.throws(call, WebAssembly.RuntimeError);
                assert.deepEqual(events.splice(0), expected);
            }}
            {assertions}
            assert.deepEqual(events, []);
        "#
        );
        let output = test_process::output(
            Command::new("node").args(["-e", &script]),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn calls_bind_written_arguments_then_defaults_before_contracts_and_body() {
    run(
        r#"
        fn target(a: number, b: number = a + 1, c: number = b * 2) -> number
            requires a > 0 requires b == a + 1 requires c > b {
            mark(10); return a * 100 + b * 10 + c;
        }
        fn named() { return target(c: mark(9), a: mark(3)); }
        fn mixed() { return target(mark(3), c: mark(9)); }
        fn omitted() { return target(2); }
        fn failed() { return target(c: mark(1), a: mark(3)); }
        fn sized(xs: number[], trigger: number, n: number = len(xs)) -> number
            requires n == 7 { mark(30); return n; }
        fn default_trap(xs: number[]) { return sized(xs: xs, trigger: mark(90)); }
        fn default_false(xs: number[]) { return sized(xs: xs, trigger: mark(20)); }
        fn default_bypass(xs: number[]) { return sized(xs: xs, trigger: mark(20), n: 7); }
        fn short_default(xs: number[], trigger: number, guard: bool = false && len(xs) > 0) -> number
            requires guard { mark(31); return 1; }
        fn skipped_default_len(xs: number[]) { return short_default(xs: xs, trigger: mark(90)); }
        fn recursive(n: number, next: number = n - 1) -> number requires n >= 0 {
            mark(n); if n == 0 { return 0; } return n + recursive(next);
        }
        fn recursive_failure(n: number) -> number requires n > 0 {
            mark(n); if n == 1 { return recursive_failure(0); } return recursive_failure(n - 1);
        }
        "#,
        r#"
        assert.equal(e.named(), 349); assert.deepEqual(events.splice(0), [9, 3, 10]);
        assert.equal(e.mixed(), 349); assert.deepEqual(events.splice(0), [3, 9, 10]);
        assert.equal(e.omitted(), 236); assert.deepEqual(events.splice(0), [10]);
        trap(() => e.failed(), [1, 3]);
        assert.equal(e.target.length, 3);
        assert.equal(e.target(3, 4, 9), 349); assert.deepEqual(events.splice(0), [10]);
        trap(() => e.target(3, 0, 9)); // Raw exports do not substitute defaults.
        trap(() => e.target(3)); // Missing JS f64 arguments become NaN, not defaults.
        trap(() => e.target(-1, 0, 9));
        const xs = array([1, 2]);
        trap(() => e.default_false(xs), [20]);
        assert.equal(e.default_bypass(xs), 7); assert.deepEqual(events.splice(0), [20, 30]);
        corruptHandle = xs;
        assert.throws(() => e.default_trap(xs), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            assert.ok(!error.stack.includes(`wasm-function[${functionIndices.get('sized')}]`));
            return true;
        });
        assert.deepEqual(events.splice(0), [90]);
        new DataView(e.memory.buffer).setUint32(xs + 4, 2, true);
        // The false default guard skips the now-invalid len. Invocation proceeds
        // to callee entry, where mandatory descriptor validation still traps.
        assert.throws(() => e.skipped_default_len(xs), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            assert.ok(error.stack.includes(`wasm-function[${functionIndices.get('short_default')}]`));
            return true;
        });
        assert.deepEqual(events.splice(0), [90]);
        new DataView(e.memory.buffer).setUint32(xs + 4, 2, true);
        corruptHandle = 0;
        assert.equal(e.recursive(4, 3), 10); assert.deepEqual(events.splice(0), [4, 3, 2, 1, 0]);
        trap(() => e.recursive(-1, -2));
        trap(() => e.recursive_failure(3), [3, 2, 1]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn caller_transfers_and_skipped_calls_never_invoke_contracts_or_body() {
    run(
        r#"
        fn reject(a: number, b: number = a + 1) -> number requires false { mark(99); return b; }
        fn returned() -> number { return 5 + reject(b: mark(1), a: { mark(2); return 77; 0 }); }
        fn broken() -> number { return 10 + loop { reject(b: mark(3), a: { mark(4); break 8; 0 }); break 99; }; }
        fn continued() -> number {
            let n = 0; for i in [1, 2, 3] {
                n += 1; reject(b: mark(i), a: { mark(10 + i); continue; 0 });
            } return n;
        }
        fn skipped() -> bool { return false && reject(mark(98)) == 0; }
        fn hidden() -> number { return 8; reject(mark(98)); }
        "#,
        r#"
        assert.equal(e.returned(), 77); assert.deepEqual(events.splice(0), [1, 2]);
        assert.equal(e.broken(), 18); assert.deepEqual(events.splice(0), [3, 4]);
        assert.equal(e.continued(), 3); assert.deepEqual(events.splice(0), [1, 11, 2, 12, 3, 13]);
        assert.equal(e.skipped(), 0); assert.equal(e.hidden(), 8);
        trap(() => e.reject(1, 2));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn guards_shared_contents_and_all_descriptor_parameters_are_checked_at_entry() {
    run(
        r#"
        fn equal(xs: number[], ys: number[], trigger: number, same: bool = xs == ys) -> number
            requires same requires xs == ys { mark(40); return len(xs); }
        fn mutation() -> number {
            let xs = [1]; let alias = xs; let ys = [8];
            return equal(ys: ys, xs: xs, trigger: { alias[0] = 8; mark(41) });
        }
        fn changed(xs: number[], ys: number[]) -> number {
            return equal(same: true, xs: xs, ys: ys, trigger: { xs[0] = 9; mark(42) });
        }
        fn guarded(xs: number[], enabled: bool) -> number
            requires !enabled || len(xs) > 0 requires enabled || len(xs) == 0 {
            mark(50); return len(xs);
        }
        fn ordered(xs: number[], first: bool, second: bool) -> number
            requires first requires second && len(xs) > 0 { mark(51); return 1; }
        fn descriptors(first: number[], second: string, third: number[], flag: bool) -> number
            requires flag requires first == third requires second == "é" { mark(52); return len(first); }
        fn descriptors_false(first: number[], second: string, third: number[]) -> number
            requires false requires len(third) > 0 { mark(53); return 1; }
        "#,
        r#"
        assert.equal(e.mutation(), 1); assert.deepEqual(events.splice(0), [41, 40]);
        const xs = array([1]), ys = array([1]), empty = array([]), s = text('é');
        trap(() => e.changed(xs, ys), [42]);
        new DataView(e.memory.buffer).setFloat64(ys + 8, 9, true);
        assert.equal(e.equal(xs, ys, 0, 1), 1); assert.deepEqual(events.splice(0), [40]);
        assert.equal(e.guarded(empty, 0), 0); assert.deepEqual(events.splice(0), [50]);
        assert.equal(e.guarded(xs, 1), 1); assert.deepEqual(events.splice(0), [50]);
        trap(() => e.guarded(empty, 1)); trap(() => e.guarded(xs, 0));
        trap(() => e.ordered(xs, 0, 1)); trap(() => e.ordered(xs, 1, 0));
        assert.equal(e.ordered(xs, 1, 1), 1); assert.deepEqual(events.splice(0), [51]);
        assert.equal(e.descriptors(xs, s, ys, 1), 1); assert.deepEqual(events.splice(0), [52]);
        // A true first contract cannot excuse validation of any later parameter.
        for (const args of [[0, s, ys, 1], [xs, 0, ys, 1], [xs, s, 0, 1]]) {
            trap(() => e.descriptors(...args));
        }
        // Nor can a false first clause or a skipped len relax entry validation.
        assert.throws(() => e.descriptors_false(xs, s, ys), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            const first = /wasm-function\[(\d+)\]/.exec(error.stack);
            assert.equal(Number(first[1]), functionIndices.get('descriptors_false'));
            return true;
        });
        assert.throws(() => e.descriptors_false(xs, s, 0), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            const first = /wasm-function\[(\d+)\]/.exec(error.stack);
            assert.notEqual(Number(first[1]), functionIndices.get('descriptors_false'));
            assert.ok(error.stack.includes(`wasm-function[${functionIndices.get('descriptors_false')}]`));
            return true;
        });
        trap(() => e.guarded(0, 0));
        const view = new DataView(e.memory.buffer);
        view.setUint32(ys + 4, 0xffffffff, true);
        trap(() => e.descriptors(xs, s, ys, 1));
        trap(() => e.descriptors_false(xs, s, ys));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn declaration_order_and_logical_guards_skip_a_proven_trapping_later_check() {
    run(
        r#"
        fn literal() -> string { return "contract-literal"; }
        fn ordered(first: bool) -> number
            requires first requires "contract-literal" == "contract-literal" { mark(55); return 1; }
        fn guarded() -> number
            requires true || "contract-literal" == "contract-literal" { mark(56); return 2; }
        fn false_guard() -> number
            requires false && "contract-literal" == "contract-literal" { mark(57); return 3; }
        "#,
        r#"
        const handle = e.literal() >>> 0;
        const view = new DataView(e.memory.buffer);
        const length = view.getUint32(handle + 4, true);
        // Deliberately violate trusted-host metadata on an interned literal.
        // Unlike formal descriptors, this is not validated at parameter entry.
        view.setUint32(handle + 4, 0xffffffff, true);
        assert.throws(() => e.ordered(1), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            const frames = [...error.stack.matchAll(/wasm-function\[(\d+)\]/g)].map(m => Number(m[1]));
            assert.ok(frames.length > 1); // The later equality really traps in a helper.
            assert.equal(frames.at(-1), functionIndices.get('ordered'));
            return true;
        });
        for (const [name, call] of [['ordered', () => e.ordered(0)], ['false_guard', () => e.false_guard()]]) {
            assert.throws(call, error => {
                assert.ok(error instanceof WebAssembly.RuntimeError);
                const frames = [...error.stack.matchAll(/wasm-function\[(\d+)\]/g)].map(m => Number(m[1]));
                assert.deepEqual(frames, [functionIndices.get(name)]);
                return true;
            });
        }
        assert.equal(e.guarded(), 2); assert.deepEqual(events.splice(0), [56]);
        new DataView(e.memory.buffer).setUint32(handle + 4, length, true);
        assert.equal(e.ordered(1), 1); assert.deepEqual(events.splice(0), [55]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn contracts_preserve_ieee_remainder_signed_zero_and_elementwise_equality() {
    run(
        r#"
        fn identity(x: number) -> number requires x == x { mark(60); return x; }
        fn is_nan(x: number) -> bool requires x != x { mark(61); return true; }
        fn zero(x: number) -> number requires x == 0 requires x % 3 == 0 { return x % 3; }
        fn remainder(x: number, y: number, expected: number) -> number
            requires x % y == expected { return x % y; }
        fn nan_remainder(x: number, y: number) -> number
            requires x % y != x % y { return x % y; }
        fn arrays(xs: number[], ys: number[]) -> bool requires xs == ys { return true; }
        fn texts(a: string, b: string) -> bool requires a == b { return true; }
        "#,
        r#"
        for (const value of [0, -0, Infinity, -Infinity, Number.MIN_VALUE, -Number.MIN_VALUE, Number.MAX_VALUE]) {
            assert.ok(Object.is(e.identity(value), value)); assert.deepEqual(events.splice(0), [60]);
        }
        trap(() => e.identity(NaN));
        assert.equal(e.is_nan(NaN), 1); assert.deepEqual(events.splice(0), [61]);
        trap(() => e.is_nan(Infinity));
        assert.ok(Object.is(e.zero(-0), -0)); assert.ok(Object.is(e.zero(0), 0));
        for (const [x, y] of [[-5, 3], [5, -3], [-0, 3], [Number.MIN_VALUE, Infinity],
            [Number.MAX_VALUE, 3], [Number.MIN_VALUE * 3, Number.MIN_VALUE * 2]]) {
            const expected = x % y;
            assert.ok(Object.is(e.remainder(x, y, expected), expected));
        }
        for (const [x, y] of [[NaN, 1], [Infinity, 3], [1, 0], [1, -0]]) {
            assert.ok(Number.isNaN(e.nan_remainder(x, y)));
        }
        const nan = array([NaN]);
        trap(() => e.arrays(nan, nan)); // No handle-identity shortcut.
        assert.equal(e.arrays(array([0, Infinity, Number.MIN_VALUE]), array([-0, Infinity, Number.MIN_VALUE])), 1);
        trap(() => e.arrays(array([1]), array([1, 2])));
        assert.equal(e.texts(text('é🦀'), text('é🦀')), 1);
        trap(() => e.texts(text('é'), text('e\u0301')));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn selected_export_calls_private_contracts_using_the_analyzed_import_snapshot() {
    let source = "fn private(n: number = 2) -> number requires n > 0 { mark(70); return n; } fn main(b: bool) { if b { return private(); } return private(-1); }";
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let program = compiler.analyze(source).unwrap();
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    let mut baseline = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        if let Some(expected) = &baseline {
            assert_eq!(&bytes, expected);
        } else {
            baseline = Some(bytes.clone());
        }
        let script = r#"
            const assert = require('node:assert/strict');
            const module = new WebAssembly.Module(require('node:fs').readFileSync(0));
            assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
                [['env', 'mark'], ['requires', 'requires']]);
            assert.deepEqual(WebAssembly.Module.exports(module).map(e => e.name), ['main']);
            const events = [];
            const e = new WebAssembly.Instance(module, {
                env: {mark(n) {events.push(n); return n;}},
                requires: {requires() {throw new Error('unused import executed');}}
            }).exports;
            assert.equal(e.main(1), 2); assert.deepEqual(events.splice(0), [70]);
            assert.throws(() => e.main(0), WebAssembly.RuntimeError);
            assert.deepEqual(events, []);
        "#;
        let output = test_process::output(
            Command::new("node").args(["-e", script]),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn selected_exports_and_analyzed_host_snapshot_preserve_contracts_at_all_levels() {
    let source = "fn private(n: number = 2) -> number requires n > 0 { return n; } fn main() { return private(); }";
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let program = compiler.analyze(source).unwrap();
    let baseline = compiler.compile(&program).unwrap();
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(bytes, baseline);
        Validator::new().validate_all(&bytes).unwrap();
        let mut exports = Vec::new();
        let mut imports = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::ExportSection(reader) => {
                    exports.extend(
                        reader
                            .into_iter()
                            .map(|entry| entry.unwrap().name.to_owned()),
                    );
                }
                Payload::ImportSection(reader) => {
                    imports.extend(reader.into_imports().map(|entry| {
                        let entry = entry.unwrap();
                        (entry.module.to_owned(), entry.name.to_owned())
                    }));
                }
                _ => {}
            }
        }
        assert_eq!(exports, ["main"]);
        assert_eq!(
            imports,
            [
                ("env".into(), "mark".into()),
                ("requires".into(), "requires".into())
            ]
        );
    }
}
