#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type};
use wasmparser::Validator;

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
        let script = format!(
            r#"
            const assert = require('node:assert/strict');
            const module = new WebAssembly.Module(require('node:fs').readFileSync(0));
            assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
                [['env', 'mark'], ['env', 'unused']]);
            const events = [];
            const e = new WebAssembly.Instance(module, {{env: {{
                mark(n) {{ events.push(n); return n; }},
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
