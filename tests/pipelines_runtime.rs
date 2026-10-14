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

fn compiler(selected: &[&str]) -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler.set_exports(ExportOptions::Only(
        selected.iter().map(|name| (*name).into()).collect(),
    ));
    compiler
}

fn run(source: &str, selected: &[&str], assertions: &str) {
    let mut compiler = compiler(selected);
    let mut baseline = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler
            .compile_source(source)
            .expect("pipeline source compilation");
        let program = compiler.analyze(source).unwrap();
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        Validator::new().validate_all(&bytes).unwrap();
        if let Some(expected) = &baseline {
            assert_eq!(&bytes, expected, "{level:?}");
        } else {
            baseline = Some(bytes.clone());
        }
        let function_indices = Parser::new(0)
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
            const module = new WebAssembly.Module(require('node:fs').readFileSync(0));
            assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
                [['env', 'mark'], ['env', 'unused']]);
            const functionIndices = new Map([{function_indices}]);
            const events = [];
            let onMark = () => {{}};
            const e = new WebAssembly.Instance(module, {{env: {{
                mark(n) {{ events.push(n); onMark(n); return n; }},
                unused() {{ assert.fail('unused import executed'); }}
            }} }}).exports;
            function contents(h, string = false) {{
                const view = new DataView(e.memory.buffer);
                const p = view.getUint32(h >>> 0, true), n = view.getUint32((h >>> 0) + 4, true);
                return string ? Buffer.from(e.memory.buffer, p, n).toString('utf8') :
                    Array.from({{length: n}}, (_, i) => view.getFloat64(p + 8 * i, true));
            }}
            function trap(call, expected) {{
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
fn named_pipeline_arguments_keep_written_order_and_defaults_use_formal_order() {
    run(
        r#"
        fn digits(a: number = 8, b: number = a + 1, c: number = b * 2) -> number
            requires a > 0 requires b > 0 requires c > 0 {
            mark(90); return a * 100 + b * 10 + c;
        }
        fn finish(x: number, extra: number = x + 1) -> number {
            mark(91); return x + extra;
        }
        fn reordered() -> number {
            return mark(1) |> digits(c: mark(3), b: mark(2)) |> finish(extra: mark(4));
        }
        fn mixed() -> number { return mark(1) |> digits(mark(2), c: mark(3)); }
        fn omitted() -> number { return mark(2) |> digits(c: mark(9)) |> finish; }
        fn bare() -> number { return mark(2) |> digits; }
        fn empty() -> number { return mark(2) |> digits(); }
        fn nested() -> number {
            return mark(1) |> digits(c: (mark(2) |> digits(c: mark(9))), b: mark(4));
        }
        "#,
        &["reordered", "mixed", "omitted", "bare", "empty", "nested"],
        r#"
        assert.equal(e.reordered(), 127); assert.deepEqual(events.splice(0), [1,3,2,90,4,91]);
        assert.equal(e.mixed(), 123); assert.deepEqual(events.splice(0), [1,2,3,90]);
        assert.equal(e.omitted(), 479); assert.deepEqual(events.splice(0), [2,9,90,91]);
        assert.equal(e.bare(), 236); assert.deepEqual(events.splice(0), [2,90]);
        assert.equal(e.empty(), 236); assert.deepEqual(events.splice(0), [2,90]);
        assert.equal(e.nested(), 379); assert.deepEqual(events.splice(0), [1,2,9,90,4,90]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn pipeline_input_arguments_contracts_and_stages_execute_once_in_written_order() {
    run(
        r#"
        fn digits(a: number, b: number, c: number) -> number
            requires a > 0 requires b > 0 requires c > 0 {
            mark(90); return a * 100 + b * 10 + c;
        }
        fn finish(x: number, extra: number) -> number requires x == 123 {
            mark(91); return x + extra;
        }
        fn identity(x: number) -> number { mark(92); return x; }
        fn main() -> number {
            return mark(1) |> digits(mark(2), mark(3)) |> finish(mark(4));
        }
        fn nested() -> number {
            return mark(1) |> digits((mark(2) |> identity), mark(3));
        }
        fn bare() -> number { return mark(5) |> identity; }
        fn empty() -> number { return mark(5) |> identity(); }
        fn explicit_defaults(a: number = 8, b: number = a + 1) -> number {
            return a * 10 + b;
        }
        fn full() -> number { return 2 |> explicit_defaults(7); }
        fn ordinary() -> number {
            return explicit_defaults() + explicit_defaults(b: 4, a: 3);
        }
        "#,
        &["main", "nested", "bare", "empty", "full", "ordinary"],
        r#"
        assert.equal(e.main(), 127); assert.deepEqual(events.splice(0), [1,2,3,90,4,91]);
        assert.equal(e.nested(), 123); assert.deepEqual(events.splice(0), [1,2,92,3,90]);
        assert.equal(e.bare(), 5); assert.deepEqual(events.splice(0), [5,92]);
        assert.equal(e.empty(), 5); assert.deepEqual(events.splice(0), [5,92]);
        assert.equal(e.full(), 27);
        assert.equal(e.ordinary(), 123);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn pipeline_defaults_capture_scalars_but_observe_shared_handles_after_written_effects() {
    run(
        r#"
        fn capture(a: number = 1, b: number = a + 1, c: number = b * 2,
                   d: number = c + 1) -> number requires c == b * 2 {
            mark(90); return a * 1000 + b * 100 + c * 10 + d;
        }
        fn scalar() -> number {
            let x = 7;
            let n = x |> capture(d: { x = 9; mark(x) });
            return n + x;
        }
        fn inspect(xs: number[], expected: number[], trigger: number,
                   copy: number[] = xs, n: number = len(copy), same: bool = copy == expected)
            -> number requires same requires n == 2 {
            mark(91); copy[1] = 9;
            return xs[0] * 100 + n * 10 + if same { 1 } else { 0 };
        }
        fn arrays() -> number {
            let xs = [1, 2]; let alias = xs;
            let n = xs |> inspect(trigger: { alias[0] = 8; xs = [99]; mark(6) },
                                  expected: { mark(7); [8, 2] });
            mark(alias[1]); mark(xs[0]);
            return n + alias[1] + xs[0];
        }
        fn text(first: string, copy: string = first, same: bool = first == copy) -> string
            requires same { mark(92); return copy; }
        fn texts() -> string { return "é🦀" |> text() |> text; }
        fn progression(start: number, end: number = start + 4) -> range {
            mark(93); return start..=end by 2;
        }
        fn copy_range(r: range, copy: range = r) -> range { mark(94); return copy; }
        fn total(r: range, copy: range = r) -> number {
            mark(95); let n = 0; for i in copy { n += i; } return n;
        }
        fn ranges() -> number { return mark(1) |> progression |> copy_range() |> total; }
        "#,
        &["scalar", "arrays", "texts", "ranges"],
        r#"
        assert.equal(e.scalar(), 7978); assert.deepEqual(events.splice(0), [9,90]);
        assert.equal(e.arrays(), 929); assert.deepEqual(events.splice(0), [6,7,91,9,99]);
        assert.equal(contents(e.texts(), true), 'é🦀'); assert.deepEqual(events.splice(0), [92,92]);
        assert.equal(e.ranges(), 9); assert.deepEqual(events.splice(0), [1,93,94,95]);
        for (const name of ['progression', 'copy_range', 'total']) assert.ok(!(name in e));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn pipelines_preserve_scalar_captures_array_aliases_and_fresh_literal_storage() {
    run(
        r#"
        fn pair(a: number, b: number) -> number { mark(90); return a * 10 + b; }
        fn inspect(xs: number[], extra: number) -> number requires len(xs) == 2 {
            mark(91); xs[1] = 8; return xs[0] * 100 + xs[1] * 10 + extra;
        }
        fn keep(xs: number[]) -> number[] { mark(92); return xs; }
        fn scalar() -> number {
            let x = 7;
            let n = x |> pair({ x = 9; mark(x) });
            return n + x;
        }
        fn aliases() -> number {
            let xs = [1, 2]; let alias = xs;
            let n = xs |> inspect({ alias[0] = 40; xs = [99]; mark(2) });
            mark(alias[0]); mark(alias[1]); mark(xs[0]);
            return n + alias[1] + xs[0];
        }
        fn fresh() -> number[] { return [1, 2] |> keep; }
        "#,
        &["scalar", "aliases", "fresh"],
        r#"
        assert.equal(e.scalar(), 88); assert.deepEqual(events.splice(0), [9,90]);
        assert.equal(e.aliases(), 4189); assert.deepEqual(events.splice(0), [2,91,40,8,99]);
        const a = e.fresh(), b = e.fresh();
        assert.notEqual(a, b); assert.deepEqual(contents(a), [1,2]);
        assert.deepEqual(contents(b), [1,2]); assert.deepEqual(events.splice(0), [92,92]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn pipelines_carry_every_concrete_type_and_keep_range_stages_internal() {
    run(
        r#"
        fn numeric(x: number) -> number { return x; }
        fn logical(x: bool) -> bool { return !x; }
        fn text(x: string) -> string { return x; }
        fn array(x: number[]) -> number[] { return x; }
        fn progression(x: number, end: number) -> range { return x..=end by 2; }
        fn range_identity(x: range) -> range { return x; }
        fn total(x: range) -> number { let n = 0; for i in x { n += i; } return n; }
        fn number_value(x: number) -> number { return x |> numeric |> numeric(); }
        fn bool_value(x: bool) -> bool { return x |> logical |> logical(); }
        fn string_value() -> string { return "é🦀" |> text |> text(); }
        fn array_value() -> number[] { return [1, 2] |> array |> array(); }
        fn range_value() -> number { return 1 |> progression(5) |> range_identity |> total(); }
        "#,
        &[
            "number_value",
            "bool_value",
            "string_value",
            "array_value",
            "range_value",
        ],
        r#"
        for (const x of [0, -0, 1, -2.5, Number.MIN_VALUE, Number.MAX_VALUE, Infinity, -Infinity, NaN])
            assert.ok(Object.is(e.number_value(x), x));
        for (const x of [0, 1, -7, 42]) assert.equal(e.bool_value(x), x === 0 ? 0 : 1);
        assert.equal(contents(e.string_value(), true), 'é🦀');
        assert.deepEqual(contents(e.array_value()), [1,2]);
        assert.equal(e.range_value(), 9);
        assert.ok(!('progression' in e)); assert.ok(!('range_identity' in e));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn caller_returns_breaks_and_continues_prevent_later_pipeline_effects() {
    run(
        r#"
        fn pair(a: number, b: number) -> number { mark(90); return a * 10 + b; }
        fn next(x: number) -> number { mark(91); return x; }
        fn input_return() -> number {
            return 100 + ({ mark(1); return 7; 0 } |> pair(mark(99)) |> next);
        }
        fn argument_return() -> number {
            return 100 + (mark(2) |> pair({ mark(3); return 8; 0 }) |> next);
        }
        fn later_return() -> number {
            return 100 + (mark(4) |> pair(mark(5)) |> pair({ mark(6); return 9; 0 }) |> next);
        }
        fn value_break() -> number {
            return 10 + loop {
                mark(7) |> pair({ mark(8); break 4; 0 }) |> next;
                break 99;
            };
        }
        fn statement_break() -> number {
            let n = 0;
            while n < 3 {
                n += 1;
                ({ mark(n); if n == 2 { break; } n } |> pair(mark(10)) |> next);
                mark(20);
            }
            return n;
        }
        fn continued() -> number {
            let sum = 0;
            for n in [1, 2, 3] {
                sum += 100 + (mark(n) |> pair({ if n < 3 { continue; } mark(4) }) |> next);
            }
            return sum;
        }
        fn input_continue() -> number {
            let sum = 0;
            for n in 0..3 {
                sum += ({ mark(n); if n == 1 { continue; } n } |> pair(mark(4)) |> next);
            }
            return sum;
        }
        fn skipped() -> bool { return false && (mark(99) |> next) == 0; }
        fn unreachable() -> number { return 6; mark(99) |> next; }
        "#,
        &[
            "input_return",
            "argument_return",
            "later_return",
            "value_break",
            "statement_break",
            "continued",
            "input_continue",
            "skipped",
            "unreachable",
        ],
        r#"
        assert.equal(e.input_return(), 7); assert.deepEqual(events.splice(0), [1]);
        assert.equal(e.argument_return(), 8); assert.deepEqual(events.splice(0), [2,3]);
        assert.equal(e.later_return(), 9); assert.deepEqual(events.splice(0), [4,5,90,6]);
        assert.equal(e.value_break(), 14); assert.deepEqual(events.splice(0), [7,8]);
        assert.equal(e.statement_break(), 2); assert.deepEqual(events.splice(0), [1,10,90,91,20,2]);
        assert.equal(e.continued(), 134); assert.deepEqual(events.splice(0), [1,2,3,4,90,91]);
        assert.equal(e.input_continue(), 28); assert.deepEqual(events.splice(0), [0,4,90,91,1,2,4,90,91]);
        assert.equal(e.skipped(), 0); assert.equal(e.unreachable(), 6);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn named_pipeline_transfers_stop_later_written_arguments_defaults_and_stages() {
    run(
        r#"
        fn consume(xs: number[], a: number = len(xs), b: number = a + 1,
                   c: number = b + 1) -> number requires a > 0 {
            mark(90); return a * 100 + b * 10 + c;
        }
        fn next(x: number, extra: number = x + 1) -> number { mark(91); return x + extra; }
        fn input_return() -> number {
            return 100 + ({ mark(1); return 7; [] } |> consume(c: mark(99)) |> next);
        }
        fn argument_return() -> number {
            return 100 + ([] |> consume(c: { mark(2); return 8; 0 }, b: mark(99)) |> next);
        }
        fn later_return() -> number {
            return [1] |> consume(c: mark(3))
                |> next(extra: { mark(4); return 9; 0 }) |> next(extra: mark(99));
        }
        fn value_break() -> number {
            return 10 + loop {
                [] |> consume(c: { mark(5); break 4; 0 }, b: mark(99)) |> next;
                break 99;
            };
        }
        fn input_break() -> number {
            return 10 + loop {
                ({ mark(6); break 5; [] } |> consume(c: mark(99)) |> next);
                break 99;
            };
        }
        fn statement_break() -> number {
            let n = 0;
            while n < 3 {
                n += 1;
                [] |> consume(c: { mark(n); break; 0 }, b: mark(99)) |> next;
                mark(99);
            }
            return n;
        }
        fn argument_continue() -> number {
            let count = 0;
            for n in [1, 2, 3] {
                count += 1;
                [] |> consume(c: { mark(n); continue; 0 }, b: mark(99)) |> next;
                mark(99);
            }
            return count;
        }
        fn input_continue() -> number {
            let count = 0;
            for n in 1..=3 {
                count += 1;
                ({ mark(n); continue; [] } |> consume(c: mark(99)) |> next);
                mark(99);
            }
            return count;
        }
        fn skipped() -> bool { return false && ([] |> consume(c: mark(99)) |> next) == 0; }
        fn unreachable() -> number { return 6; [] |> consume(c: mark(99)) |> next; }
        "#,
        &[
            "input_return",
            "argument_return",
            "later_return",
            "value_break",
            "input_break",
            "statement_break",
            "argument_continue",
            "input_continue",
            "skipped",
            "unreachable",
        ],
        r#"
        assert.equal(e.input_return(), 7); assert.deepEqual(events.splice(0), [1]);
        assert.equal(e.argument_return(), 8); assert.deepEqual(events.splice(0), [2]);
        assert.equal(e.later_return(), 9); assert.deepEqual(events.splice(0), [3,90,4]);
        assert.equal(e.value_break(), 14); assert.deepEqual(events.splice(0), [5]);
        assert.equal(e.input_break(), 15); assert.deepEqual(events.splice(0), [6]);
        assert.equal(e.statement_break(), 1); assert.deepEqual(events.splice(0), [1]);
        assert.equal(e.argument_continue(), 3); assert.deepEqual(events.splice(0), [1,2,3]);
        assert.equal(e.input_continue(), 3); assert.deepEqual(events.splice(0), [1,2,3]);
        assert.equal(e.skipped(), 0); assert.equal(e.unreachable(), 6);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn named_pipeline_input_argument_and_default_traps_prevent_later_effects() {
    run(
        r#"
        fn consume(xs: number[], a: number = len(xs), b: number = a + 1,
                   c: number = b + 1) -> number requires a > 0 {
            mark(90); return a * 100 + b * 10 + c;
        }
        fn next(x: number, extra: number = x + 1) -> number { mark(91); return x + extra; }
        fn input_trap() -> number {
            return { mark(1); let xs = [1]; let n = xs[2]; [] }
                |> consume(c: mark(99)) |> next;
        }
        fn argument_trap() -> number {
            return { mark(2); [1] }
                |> consume(c: { mark(3); let xs = [1]; xs[2] }, b: mark(99)) |> next;
        }
        fn default_trap(xs: number[]) -> number {
            return xs |> consume(c: mark(4), b: mark(5)) |> next(extra: mark(99));
        }
        fn default_bypass(xs: number[]) -> number {
            return xs |> consume(c: mark(6), b: mark(7), a: mark(8)) |> next(extra: mark(99));
        }
        fn transferred(xs: number[]) -> number {
            return xs |> consume(c: { mark(9); return 77; 0 }, b: mark(99)) |> next;
        }
        fn fail(x: number, extra: number = x + 1) -> number requires extra > 0 {
            mark(92); let xs = [1]; return xs[2];
        }
        fn body_trap() -> number { return mark(7) |> fail(extra: mark(8)) |> next(extra: mark(99)); }
        fn fresh() -> number[] { return [1]; }
        "#,
        &[
            "input_trap",
            "argument_trap",
            "default_trap",
            "default_bypass",
            "transferred",
            "body_trap",
            "fresh",
            "consume",
        ],
        r#"
        trap(() => e.input_trap(), [1]);
        trap(() => e.argument_trap(), [2,3]);
        trap(() => e.body_trap(), [7,8,92]);
        const h = e.fresh() >>> 0;
        function restore() { new DataView(e.memory.buffer).setUint32(h + 4, 1, true); }
        // Deliberately violate trusted-host metadata after wrapper entry validation.
        onMark = n => {
            if ([4,6,9].includes(n)) new DataView(e.memory.buffer).setUint32(h + 4, 0xffffffff, true);
        };
        assert.throws(() => e.default_trap(h), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            const frames = [...error.stack.matchAll(/wasm-function\[(\d+)\]/g)].map(m => Number(m[1]));
            assert.ok(!frames.includes(functionIndices.get('consume')));
            assert.equal(frames.at(-1), functionIndices.get('default_trap'));
            return true;
        });
        assert.deepEqual(events.splice(0), [4,5]); restore();
        // Supplying the default delays this same invalid-descriptor trap to callee entry.
        assert.throws(() => e.default_bypass(h), error => {
            assert.ok(error instanceof WebAssembly.RuntimeError);
            const frames = [...error.stack.matchAll(/wasm-function\[(\d+)\]/g)].map(m => Number(m[1]));
            assert.ok(frames.includes(functionIndices.get('consume')));
            return true;
        });
        assert.deepEqual(events.splice(0), [6,7,8]); restore();
        assert.equal(e.transferred(h), 77); assert.deepEqual(events.splice(0), [9]); restore();
        onMark = () => {};
        assert.equal(e.default_trap(h), 253); assert.deepEqual(events.splice(0), [4,5,90,99,91]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn named_pipeline_parameter_validation_precedes_ordered_contracts_and_body() {
    run(
        r#"
        fn literal() -> string { return "entry-guard"; }
        fn fresh() -> number[] { return [1]; }
        fn checked(first: number[], second: string, third: number[] = first,
                   trigger: number = 1) -> number
            requires "entry-guard" == "entry-guard" requires first == third {
            mark(90); return len(first);
        }
        fn invoke(xs: number[], s: string, ys: number[]) -> number {
            return xs |> checked(trigger: mark(1), third: { mark(2); ys }, second: s)
                |> next(extra: mark(99));
        }
        fn false_first(xs: number[], copy: number[] = xs) -> number
            requires false requires "entry-guard" == "entry-guard" {
            mark(91); return len(copy);
        }
        fn invoke_false(xs: number[]) -> number {
            return xs |> false_first() |> next(extra: mark(99));
        }
        fn invoke_false_copy(xs: number[], ys: number[]) -> number {
            return xs |> false_first(copy: { mark(4); ys }) |> next(extra: mark(99));
        }
        fn ordered(first: bool, last: bool = first) -> number
            requires first requires "entry-guard" == "entry-guard" requires last {
            mark(92); return 7;
        }
        fn invoke_ordered(first: bool, last: bool) -> number {
            return first |> ordered(last: { mark(3); last }) |> next(extra: mark(99));
        }
        fn default_guard(first: bool, earlier: bool = first,
                         later: bool = earlier || "entry-guard" == "entry-guard") -> number
            requires later { mark(93); return 8; }
        fn invoke_default(first: bool) -> number { return first |> default_guard(); }
        fn next(x: number, extra: number = x + 1) -> number { mark(94); return x + extra; }
        "#,
        &[
            "literal",
            "fresh",
            "checked",
            "invoke",
            "false_first",
            "invoke_false",
            "invoke_false_copy",
            "ordered",
            "invoke_ordered",
            "invoke_default",
        ],
        r#"
        const xs = e.fresh() >>> 0, ys = e.fresh() >>> 0, s = e.literal() >>> 0;
        function length(h, n) { new DataView(e.memory.buffer).setUint32(h + 4, n, true); }
        function frames(call, expected) {
            let result;
            assert.throws(call, error => {
                assert.ok(error instanceof WebAssembly.RuntimeError);
                result = [...error.stack.matchAll(/wasm-function\[(\d+)\]/g)].map(m => Number(m[1]));
                assert.ok(result.length > 0);
                return true;
            });
            assert.deepEqual(events.splice(0), expected);
            return result;
        }
        assert.equal(e.invoke(xs, s, ys), 100); assert.deepEqual(events.splice(0), [1,2,90,99,94]);
        // Corrupt each formal after wrapper validation, during written-argument evaluation.
        for (const h of [xs, s, ys]) {
            onMark = n => { if (n === 1) length(h, 0xffffffff); };
            const trace = frames(() => e.invoke(xs, s, ys), [1,2]);
            assert.notEqual(trace[0], functionIndices.get('checked'));
            assert.equal(trace[1], functionIndices.get('checked'));
            assert.equal(trace.at(-1), functionIndices.get('invoke'));
            length(h, h === s ? 11 : 1);
        }
        onMark = () => {};
        // The literal is not a formal of these stages: corrupt it to make the
        // later equality provably trap if an earlier false clause fails to stop it.
        length(s, 0xffffffff);
        const falseTrace = frames(() => e.invoke_false(xs), []);
        assert.deepEqual(falseTrace, [functionIndices.get('false_first'), functionIndices.get('invoke_false')]);
        const orderedFalse = frames(() => e.invoke_ordered(0, 1), [3]);
        assert.deepEqual(orderedFalse, [functionIndices.get('ordered'), functionIndices.get('invoke_ordered')]);
        const orderedTrap = frames(() => e.invoke_ordered(1, 0), [3]);
        assert.notEqual(orderedTrap[0], functionIndices.get('ordered'));
        assert.equal(orderedTrap.at(-2), functionIndices.get('ordered'));
        assert.equal(orderedTrap.at(-1), functionIndices.get('invoke_ordered'));
        assert.equal(e.invoke_default(1), 8); assert.deepEqual(events.splice(0), [93]);
        const defaultTrap = frames(() => e.invoke_default(0), []);
        assert.equal(defaultTrap.at(-1), functionIndices.get('invoke_default'));
        // A false first contract still cannot excuse a malformed later formal.
        onMark = n => { if (n === 4) length(ys, 0xffffffff); };
        const invalidCopy = frames(() => e.invoke_false_copy(xs, ys), [4]);
        assert.equal(invalidCopy[1], functionIndices.get('false_first'));
        assert.equal(invalidCopy.at(-1), functionIndices.get('invoke_false_copy'));
        length(ys, 1);
        // Invoke the raw full-arity stage as well as its source-pipeline wrapper.
        const invalid = frames(() => e.false_first(xs, 0), []);
        assert.equal(invalid[1], functionIndices.get('false_first'));
        onMark = () => {};
        length(s, 11);
        const lastFalse = frames(() => e.invoke_ordered(1, 0), [3]);
        assert.deepEqual(lastFalse, [functionIndices.get('ordered'), functionIndices.get('invoke_ordered')]);
        assert.equal(e.invoke_ordered(1, 1), 106); assert.deepEqual(events.splice(0), [3,92,99,94]);
        assert.equal(e.invoke_default(0), 8); assert.deepEqual(events.splice(0), [93]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn pipeline_input_argument_contract_and_body_traps_stop_later_effects() {
    run(
        r#"
        fn checked(xs: number[], n: number) -> number
            requires n > 0 requires len(xs) > 0 {
            mark(90); return n;
        }
        fn pair(a: number, b: number) -> number { mark(91); return a + b; }
        fn next(x: number) -> number { mark(92); return x; }
        fn fail(x: number) -> number { mark(93); let xs = [1]; return xs[2]; }
        fn input_trap() -> number { return { mark(1); let xs = [1]; xs[2] } |> pair(mark(99)) |> next; }
        fn argument_trap() -> number { return mark(2) |> pair({ mark(3); let xs = [1]; xs[2] }) |> next; }
        fn contract_order() -> number { return [] |> checked(mark(0)) |> next; }
        fn contract_trap() -> number { return [] |> checked(mark(4)) |> next; }
        fn contract_false() -> number { return [1] |> checked(mark(-5)) |> next; }
        fn contract_success() -> number { return [1] |> checked(mark(6)) |> next; }
        fn body_trap() -> number { return mark(7) |> fail |> pair(mark(99)); }
        fn reject(a: number, b: number) -> number requires a < 0 {
            mark(94); return a + b;
        }
        fn later_contract() -> number {
            return mark(8) |> pair(mark(9)) |> reject(mark(10)) |> pair(mark(99));
        }
        fn observe(xs: number[], expected: number[]) -> number requires xs == expected {
            mark(95); return len(xs);
        }
        fn mutated_contract() -> number {
            let xs = [1]; let alias = xs;
            return xs |> observe({ alias[0] = 2; [2] }) |> next;
        }
        "#,
        &[
            "input_trap",
            "argument_trap",
            "contract_order",
            "contract_trap",
            "contract_false",
            "contract_success",
            "body_trap",
            "later_contract",
            "mutated_contract",
            "checked",
        ],
        r#"
        trap(() => e.input_trap(), [1]);
        trap(() => e.argument_trap(), [2,3]);
        trap(() => e.contract_order(), [0]);
        trap(() => e.contract_trap(), [4]);
        trap(() => e.contract_false(), [-5]);
        assert.equal(e.contract_success(), 6); assert.deepEqual(events.splice(0), [6,90,92]);
        trap(() => e.body_trap(), [7,93]);
        trap(() => e.later_contract(), [8,9,91,10]);
        assert.equal(e.mutated_contract(), 1); assert.deepEqual(events.splice(0), [95,92]);
        // Entry validation also applies when the same user stage is called as a raw export.
        trap(() => e.checked(0, 1), []);
        "#,
    );
}
