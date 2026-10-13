#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type};
use wasmparser::{BlockType, Operator, Parser, Payload, ValType, Validator};

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
        .register_host_function("flag", "env", "flag", vec![], Type::Bool)
        .unwrap();
    compiler
        .register_host_function("unused", "match", "_", vec![], Type::Void)
        .unwrap();
    compiler
}

fn run(source: &str, selected: &[&str], assertions: &str) {
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(
        selected.iter().map(|name| (*name).into()).collect(),
    ));
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
        let script = format!(
            r#"
            const assert = require('node:assert/strict');
            const module = new WebAssembly.Module(require('node:fs').readFileSync(0));
            assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
                [['env', 'mark'], ['env', 'flag'], ['match', '_']]);
            const events = [];
            let flagValue = 0;
            const e = new WebAssembly.Instance(module, {{
                env: {{ mark(n) {{ events.push(n); return n; }},
                    flag() {{ events.push('flag'); return flagValue; }} }},
                match: {{ _() {{ throw new Error('unused import executed'); }} }}
            }}).exports;
            function descriptor(value, string = false) {{
                const payload = string ? Buffer.from(value, 'utf8') : value;
                const width = string ? 1 : 8;
                const h = e.__alloc(8 + payload.length * width) >>> 0;
                const view = new DataView(e.memory.buffer);
                view.setUint32(h, h + 8, true);
                view.setUint32(h + 4, payload.length, true);
                if (string) new Uint8Array(e.memory.buffer, h + 8, payload.length).set(payload);
                else payload.forEach((v, i) => view.setFloat64(h + 8 + 8 * i, v, true));
                return h;
            }}
            function contents(h, string = false) {{
                const view = new DataView(e.memory.buffer);
                const p = view.getUint32(h >>> 0, true), n = view.getUint32((h >>> 0) + 4, true);
                return string ? Buffer.from(e.memory.buffer, p, n).toString('utf8') :
                    Array.from({{length: n}}, (_, i) => view.getFloat64(p + 8 * i, true));
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
fn scalar_match_caches_scrutinee_and_uses_ordered_typed_regions_not_a_jump_table() {
    let source = "fn choose(x: number) -> number { return 10 + (match mark(x) { 1 => 11, 1 => 22, 0.5..=2.5 => 33, _ => 44 }); }";
    let mut compiler = compiler();
    let mut baseline = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        if let Some(expected) = &baseline {
            assert_eq!(&bytes, expected);
        } else {
            baseline = Some(bytes.clone());
        }
        let mut calls = Vec::new();
        let mut sets = 0;
        let mut typed_ifs = 0;
        let mut comparisons = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            if let Payload::CodeSectionEntry(body) = payload.unwrap() {
                let mut reader = body.get_operators_reader().unwrap();
                while !reader.eof() {
                    match reader.read().unwrap() {
                        Operator::Call { function_index } => calls.push(function_index),
                        Operator::LocalSet { .. } | Operator::LocalTee { .. } => sets += 1,
                        Operator::If {
                            blockty: BlockType::Type(ValType::F64),
                        } => typed_ifs += 1,
                        Operator::F64Eq => comparisons.push("eq"),
                        Operator::F64Ge => comparisons.push("ge"),
                        Operator::F64Le => comparisons.push("le"),
                        Operator::BrTable { .. } => panic!("WP9 must retain ordered comparisons"),
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(calls, [0], "scrutinee host call occurs once");
        assert_eq!(sets, 1, "one numeric scrutinee cache");
        assert_eq!(
            typed_ifs, 3,
            "each non-wildcard arm has a single-result region"
        );
        assert_eq!(comparisons, ["eq", "eq", "ge", "le"]);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn ordered_patterns_scrutinee_once_selected_effects_and_ieee_inputs() {
    run(
        r#"
        fn choose(x: number) {
            return mark(10) + (match mark(x) {
                1 => mark(101), 1 => mark(102),
                3..1 => mark(103), 2..2 => mark(104),
                0.5..2.5 => mark(105), 2.5..=2.5 => mark(106),
                2..=4 => mark(107), _ => mark(108)
            }) + mark(20);
        }
        fn relations(x: number) {
            return match x { < -2 => 1, <= -1 => 2, > 2 => 3, >= 1 => 4, _ => 5 };
        }
        fn zero(x: number) { return match x { -0 => 1, 0 => 2, _ => 3 }; }
        fn signed(x: number) { return match x { 0 => x, _ => -0 }; }
        "#,
        &["choose", "relations", "zero", "signed"],
        r#"
        for (const [x, arm] of [[1,101], [0.5,105], [1.25,105], [2,105], [2.5,106],
            [3,107], [4,107], [4.25,108], [0,108], [-0,108], [NaN,108],
            [Infinity,108], [-Infinity,108]]) {
            assert.equal(e.choose(x), 30 + arm);
            const observed = events.splice(0);
            assert.equal(observed.length, 4);
            assert.equal(observed[0], 10);
            assert.ok(Object.is(observed[1], x));
            assert.deepEqual(observed.slice(2), [arm,20]);
        }
        for (const [x, result] of [[-Infinity,1], [-2,2], [-1,2], [0,5], [1,4],
            [2,4], [2.1,3], [Infinity,3], [NaN,5]]) assert.equal(e.relations(x), result);
        assert.equal(e.zero(0), 1); assert.equal(e.zero(-0), 1); assert.equal(e.zero(NaN), 3);
        assert.ok(Object.is(e.signed(-0), -0)); assert.ok(Object.is(e.signed(0), 0));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn content_strings_normalized_bools_discard_and_all_value_categories() {
    run(
        r#"
        fn text(s: string) { return match s { "é🦀" => "hit", "é🦀" => "duplicate", _ => "miss" }; }
        fn boolean(b: bool) { return match b { false => false, true => true, true => false }; }
        fn host_bool() { return match flag() { true => mark(1), false => mark(2) }; }
        fn items(b: bool) { return match b { true => [1, 2], false => [3] }; }
        fn progression(b: bool) { return match b { true => 0..=4 by 2, false => 1..3 }; }
        fn ranges(b: bool) { let r = progression(b); let n = 0; for x in r { n += x; } return n; }
        fn discard(b: bool) {
            match b { true => { mark(3); "ignored" }, false => { mark(4); [5] } };
            match b { true => 1..2, false => false } -1;
            return { match b { _ => { mark(5); true } }; match b { true => 7, false => 8 } };
        }
        fn void_value(b: bool) { (match b { true => { mark(6); }, false => { mark(7); } }); }
        "#,
        &[
            "text",
            "boolean",
            "host_bool",
            "items",
            "ranges",
            "discard",
            "void_value",
        ],
        r#"
        const a = descriptor('é🦀', true), b = descriptor('é🦀', true);
        assert.notEqual(a, b);
        assert.equal(contents(e.text(a), true), 'hit');
        assert.equal(contents(e.text(b), true), 'hit');
        assert.equal(contents(e.text(descriptor('é🦀', true)), true), 'miss');
        assert.equal(contents(e.text(descriptor('', true)), true), 'miss');
        for (const value of [0, 1, -7, 42, -2147483648]) {
            assert.equal(e.boolean(value), value === 0 ? 0 : 1);
            flagValue = value;
            assert.equal(e.host_bool(), value === 0 ? 2 : 1);
            assert.deepEqual(events.splice(0), ['flag', value === 0 ? 2 : 1]);
        }
        const first = e.items(1), second = e.items(1);
        assert.notEqual(first, second); assert.deepEqual(contents(first), [1,2]);
        assert.deepEqual(contents(e.items(0)), [3]);
        assert.equal(e.ranges(1), 6); assert.equal(e.ranges(0), 3);
        assert.equal(e.discard(1), 7); assert.deepEqual(events.splice(0), [3,5]);
        assert.equal(e.discard(0), 8); assert.deepEqual(events.splice(0), [4,5]);
        assert.equal(e.void_value(1), undefined); assert.deepEqual(events.splice(0), [6]);
        assert.equal(e.void_value(0), undefined); assert.deepEqual(events.splice(0), [7]);
        trap(() => e.text(0));
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn abrupt_arms_and_scrutinees_cross_for_latches_nested_regions_and_live_prefixes() {
    run(
        r#"
        fn iterate(xs: number[]) {
            let sum = 0;
            for x in xs {
                sum += 100 + (if true { match x {
                    1 => { mark(1); continue; },
                    3 => { mark(3); break; },
                    _ => { mark(x); 5 }
                } } else { 0 });
            }
            for x in 0..5 {
                sum += 10 + (match x {
                    0 => { mark(10); continue; },
                    3 => { mark(13); break; },
                    _ => { mark(10 + x); x }
                });
            }
            return sum;
        }
        fn returning(x: number) {
            return mark(20) + (match x { 1 => { mark(21); return 7; }, _ => mark(22) }) + mark(23);
        }
        fn value_break(b: bool) {
            return mark(30) + (loop {
                let n = 100 + (match b { true => { break 9; }, false => 2 });
                break n;
            }) + mark(31);
        }
        fn never_scrutinee() -> number {
            return mark(40) + (match { mark(41); return 8; } { _ => mark(99) }) + mark(98);
        }
        fn all_never(b: bool) -> number {
            return mark(50) + (match b { true => { return 11; }, false => { return 12; } });
        }
        fn skipped_divergence() { return match true { true => 6, false => loop {} }; }
        fn scrutinee_continue() {
            let sum = 0;
            for x in 0..3 {
                sum += 100 + (match { if x == 1 { continue; } x } { _ => x });
            }
            return sum;
        }
        fn for_returns(xs: number[]) {
            for x in xs {
                let n = 100 + (match x { 2 => { return 7; }, _ => x });
                mark(n);
            }
            for x in 0..3 {
                let n = 200 + (match x { 1 => { return 8; }, _ => x });
                mark(n);
            }
            return 9;
        }
        fn never_for_scrutinee() {
            for x in 0..3 { (match { mark(x); continue; } { _ => mark(99) }); }
            return 4;
        }
        fn outer_targets() {
            let sum = 0;
            for x in [1, 2, 3] {
                for y in (match x { 1 => { continue; }, 3 => { break; }, _ => 0..2 }) { sum += y + x; }
            }
            return sum;
        }
        "#,
        &[
            "iterate",
            "returning",
            "value_break",
            "never_scrutinee",
            "all_never",
            "skipped_divergence",
            "scrutinee_continue",
            "for_returns",
            "never_for_scrutinee",
            "outer_targets",
        ],
        r#"
        assert.equal(e.iterate(descriptor([0,1,2,3,4])), 233);
        assert.deepEqual(events.splice(0), [0,1,2,3,10,11,12,13]);
        assert.equal(e.returning(1), 7); assert.deepEqual(events.splice(0), [20,21]);
        assert.equal(e.returning(0), 65); assert.deepEqual(events.splice(0), [20,22,23]);
        assert.equal(e.value_break(1), 70); assert.deepEqual(events.splice(0), [30,31]);
        assert.equal(e.value_break(0), 163); assert.deepEqual(events.splice(0), [30,31]);
        assert.equal(e.never_scrutinee(), 8); assert.deepEqual(events.splice(0), [40,41]);
        assert.equal(e.all_never(1), 11); assert.deepEqual(events.splice(0), [50]);
        assert.equal(e.all_never(0), 12); assert.deepEqual(events.splice(0), [50]);
        assert.equal(e.skipped_divergence(), 6);
        assert.equal(e.scrutinee_continue(), 202);
        assert.equal(e.for_returns(descriptor([1,2,3])), 7); assert.deepEqual(events.splice(0), [101]);
        assert.equal(e.for_returns(descriptor([])), 8); assert.deepEqual(events.splice(0), [200]);
        assert.equal(e.never_for_scrutinee(), 4); assert.deepEqual(events.splice(0), [0,1,2]);
        assert.equal(e.outer_targets(), 5);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn match_rhs_preserves_checked_places_old_values_aliases_and_scrutinee_traps() {
    run(
        r#"
        fn update(xs: number[], b: bool) {
            let alias = xs;
            xs[mark(0)] += match { mark(1); b } {
                true => { alias[0] = mark(100); mark(3) },
                false => { alias[0] = mark(200); mark(4) }
            };
            return xs[0];
        }
        fn invalid_place(xs: number[]) {
            xs[mark(99)] = match mark(1) { _ => mark(2) };
        }
        fn invalid_scrutinee(xs: number[]) {
            return mark(5) + (match xs[mark(99)] { _ => mark(6) }) + mark(7);
        }
        fn alias_result(xs: number[], b: bool) {
            let chosen = match b { true => xs, false => [9] };
            chosen[0] = 8;
            return xs[0];
        }
        "#,
        &[
            "update",
            "invalid_place",
            "invalid_scrutinee",
            "alias_result",
        ],
        r#"
        const a = descriptor([10]);
        assert.equal(e.update(a,1), 13); assert.deepEqual(events.splice(0), [0,1,100,3]);
        assert.deepEqual(contents(a), [13]);
        const b = descriptor([10]);
        assert.equal(e.update(b,0), 14); assert.deepEqual(events.splice(0), [0,1,200,4]);
        trap(() => e.invalid_place(a), [99]);
        trap(() => e.invalid_scrutinee(a), [5,99]);
        assert.equal(e.alias_result(a,0), 13); assert.deepEqual(contents(a), [13]);
        assert.equal(e.alias_result(a,1), 8); assert.deepEqual(contents(a), [8]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn named_defaults_inference_contracts_and_traps_preserve_effect_order() {
    run(
        r#"
        fn target(a: number, b: number = a + 1, c: number = b * 2)
            requires a > 0 requires b == a + 1 requires c > b {
            return match a { 3 => { mark(10); a * 100 + b * 10 + c }, _ => { mark(11); c } };
        }
        fn leaf(n: number) { return match n { 1 => 7, _ => 9 }; }
        fn left(n: number) { return leaf(n); }
        fn right(n: number) { return match n { 1 => leaf(n), _ => left(n) }; }
        fn diamond(n: number) { return left(n) + right(n); }
        fn named(n: number) {
            return mark(1) + (match mark(n) {
                0 => target(c: mark(9), a: mark(3)),
                1 => target(mark(3)),
                _ => target(c: mark(1), a: mark(3))
            }) + mark(2);
        }
        fn abrupt_argument() {
            return target(c: match mark(5) { _ => { return 17; } }, a: mark(99));
        }
        fn trapping(xs: number[], n: number) {
            return mark(20) + (match mark(n) {
                0 => { mark(21); xs[99] },
                1 => { mark(22); len(xs) },
                _ => { mark(23); 7 }
            }) + mark(24);
        }
        fn invalid_range() { (match mark(30) { _ => mark(31)..mark(32) by 0 }); mark(99); }
        "#,
        &[
            "target",
            "diamond",
            "named",
            "abrupt_argument",
            "trapping",
            "invalid_range",
        ],
        r#"
        assert.equal(e.named(0), 352); assert.deepEqual(events.splice(0), [1,0,9,3,10,2]);
        assert.equal(e.named(1), 351); assert.deepEqual(events.splice(0), [1,1,3,10,2]);
        trap(() => e.named(2), [1,2,1,3]);
        assert.equal(e.target.length, 3);
        trap(() => e.target(3)); trap(() => e.target(3,0,9));
        assert.equal(e.diamond(1), 14); assert.equal(e.diamond(0), 18);
        assert.equal(e.abrupt_argument(), 17); assert.deepEqual(events.splice(0), [5]);
        const xs = descriptor([1,2]);
        trap(() => e.trapping(xs,0), [20,0,21]);
        assert.equal(e.trapping(xs,1), 46); assert.deepEqual(events.splice(0), [20,1,22,24]);
        assert.equal(e.trapping(xs,2), 51); assert.deepEqual(events.splice(0), [20,2,23,24]);
        trap(() => e.invalid_range(), [30,31,32]);
        "#,
    );
}
