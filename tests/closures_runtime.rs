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

fn run(source: &str, selected: &[&str], assertions: &str) {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    compiler.set_exports(ExportOptions::Only(
        selected.iter().map(|name| (*name).into()).collect(),
    ));
    let mut baseline = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let program = compiler.analyze(source).unwrap();
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
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
            const events = [];
            let onMark = () => {{}};
            const e = new WebAssembly.Instance(module, {{env: {{mark(n) {{ events.push(n); onMark(n); return n; }} }} }}).exports;
            function contents(h, string = false) {{
                const v = new DataView(e.memory.buffer);
                const p = v.getUint32(h >>> 0, true), n = v.getUint32((h >>> 0) + 4, true);
                return string ? Buffer.from(e.memory.buffer, p, n).toString('utf8') :
                    Array.from({{length:n}}, (_, i) => v.getFloat64(p + 8*i, true));
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
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn declarations_do_not_execute_bodies_and_nested_dependencies_remain_lexical() {
    run(
        r#"
        fn named(x: number) -> number { return x + 1000; }
        fn main() -> number {
            let named = (x: number) => { mark(3); x * 2 };
            let wrapper = () => { let nested = () => named(mark(4)); nested() };
            let unused = () => mark(99);
            mark(1);
            let answer = named(mark(2)) + wrapper();
            { let named = () => 100; mark(named()); };
            mark(5);
            return answer;
        }
    "#,
        &["main"],
        r#"
        assert.equal(e.main(), 12);
        assert.deepEqual(events.splice(0), [1,2,3,4,3,100,5]);
        assert.equal(e.memory, undefined);
        assert.deepEqual(Object.keys(e), ['main']);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn arguments_execute_once_left_to_right_and_transfers_stay_in_the_caller() {
    run(
        r#"
        fn ordered() -> number {
            let pair = (a: number, b: number) => { mark(3); a * 10 + b };
            return pair(mark(1), mark(2));
        }
        fn returning() -> number {
            let triple = (a: number, b: number, c: number) => { mark(99); a + b + c };
            triple(mark(4), { return 40; }, mark(97));
            mark(98); return 0;
        }
        fn breaking() -> number {
            let pair = (a: number, b: number) => { mark(99); a + b };
            return loop { pair({ mark(5); break 50; }, mark(98)); };
        }
        fn continuing() -> number {
            let pair = (a: number, b: number) => { mark(99); a + b };
            let i = 0;
            while i < 2 { i += 1; pair(mark(i), { continue; }); mark(98); }
            return i;
        }
        fn argument_loop() -> number {
            let outer = () => {
                let pair = (a: number, b: number) => { mark(8); a + b };
                pair(loop { break mark(6); }, mark(7))
            };
            return outer();
        }
        "#,
        &[
            "ordered",
            "returning",
            "breaking",
            "continuing",
            "argument_loop",
        ],
        r#"
        assert.equal(e.ordered(), 12); assert.deepEqual(events.splice(0), [1,2,3]);
        assert.equal(e.returning(), 40); assert.deepEqual(events.splice(0), [4]);
        assert.equal(e.breaking(), 50); assert.deepEqual(events.splice(0), [5]);
        assert.equal(e.continuing(), 2); assert.deepEqual(events.splice(0), [1,2]);
        assert.equal(e.argument_loop(), 13); assert.deepEqual(events.splice(0), [6,7,8]);
        assert.equal(e.memory, undefined);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn zero_argument_expression_block_returns_and_own_loops_keep_independent_contexts() {
    run(
        r#"
        fn main(flag: bool) -> number {
            let zero = (() => 7);
            let choose = (early: bool) => {
                if early { mark(1); return zero(); }
                mark(2); zero() + 1
            };
            let sink = () => { mark(3); return; mark(99); };
            let empty = () => {};
            let loops = (n: number) => {
                let i = 0; let total = 0;
                while i < n {
                    i += 1;
                    if i == 2 { continue; }
                    loop { total += i; break; }
                }
                for x in [1, 2] { total += x; }
                total
            };
            sink(); empty();
            return choose(flag) + loops(3);
        }
        fn scalar() -> number {
            let own = () => { let i = 0; loop { i += 1; if i == 2 { break; } } i };
            return own();
        }
        "#,
        &["main", "scalar"],
        r#"
        assert.equal(e.main(1), 14); assert.deepEqual(events.splice(0), [3,1]);
        assert.equal(e.main(0), 15); assert.deepEqual(events.splice(0), [3,2]);
        assert.equal(e.scalar(), 2);
        "#,
    );
    run(
        "fn main() -> number { let empty = () => {}; let value = () => { return 9; }; empty(); return value(); }",
        &["main"],
        "assert.equal(e.main(), 9); assert.equal(e.memory, undefined); assert.deepEqual(Object.keys(e), ['main']);",
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn named_defaults_contracts_traps_and_bounded_creator_recursion_work_inside_helpers() {
    run(
        r#"
        fn checked(a: number, b: number = a + 1, c: number = b + 1) -> number
            requires a > 0 requires b > a { mark(90); return a * 100 + b * 10 + c; }
        fn entry(ok: bool) -> number {
            let invoke = (good: bool) => {
                if good { return checked(c: mark(3), a: mark(1)); }
                checked(c: mark(4), a: mark(0))
            };
            return invoke(ok);
        }
        fn defaulted(xs: number[], value: number = len(xs)) -> number
            requires value > 0 { mark(91); return value; }
        fn default_trap(empty: bool) -> number {
            let invoke = (bad: bool) => {
                let xs = if bad { [] } else { [5] };
                defaulted(xs)
            };
            return invoke(empty);
        }
        fn argument_trap() -> number {
            let pair = (a: number, b: number) => { mark(99); a + b };
            let xs = [1];
            return pair({ mark(6); xs[2] }, mark(98));
        }
        fn creator(n: number) -> number {
            let recurse = (m: number) => {
                mark(m);
                if m == 0 { return 0; }
                creator(m - 1) + m
            };
            return recurse(n);
        }
        "#,
        &["entry", "default_trap", "argument_trap", "creator"],
        r#"
        for (let i = 0; i < 3; i++) {
            assert.equal(e.entry(1), 123); assert.deepEqual(events.splice(0), [3,1,90]);
            assert.throws(() => e.entry(0), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [4,0]);
            assert.throws(() => e.default_trap(1), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), []);
            assert.equal(e.default_trap(0), 1); assert.deepEqual(events.splice(0), [91]);
            assert.throws(() => e.argument_trap(), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [6]);
            assert.equal(e.creator(3), 6); assert.deepEqual(events.splice(0), [3,2,1,0]);
            assert.equal(e.creator(0), 0); assert.deepEqual(events.splice(0), [0]);
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn helpers_carry_all_concrete_types_and_discover_body_only_memory_ranges_and_remainder() {
    run(
        r#"
        fn text() -> string { let make = () => "hé😀"; let id = (s: string) => s; return id(make()); }
        fn array() -> number[] { let make = () => [1, 2, 3]; let id = (a: number[]) => a; return id(make()); }
        fn mutate(a: number[]) -> number {
            let change = (xs: number[]) => { xs[1] += 5; len(xs) + xs[1] };
            return change(a);
        }
        fn equal(s: string, a: number[]) -> bool {
            let compare = (text: string, xs: number[]) => text == "hé😀" && xs == [1, 7, 3];
            let id = (b: bool) => b;
            return id(compare(s, a));
        }
        fn ranges(step: number) -> number {
            let make = (s: number) => mark(1)..=mark(5) by mark(s);
            let id = (r: range) => r;
            let sum = (r: range) => {
                let total = 0;
                for x in r { total += x; }
                if 3 in r { total += 10; }
                total
            };
            return sum(id(make(step)));
        }
        fn remainder(a: number, b: number) -> number { let rem = (x: number, y: number) => x % y; return rem(a, b); }
        "#,
        &["text", "array", "mutate", "equal", "ranges", "remainder"],
        r#"
        const s = e.text(), a = e.array(), b = e.array();
        assert.notEqual(a, b);
        assert.equal(contents(s, true), 'hé😀');
        assert.deepEqual(contents(a), [1,2,3]);
        assert.equal(e.mutate(a), 10); assert.deepEqual(contents(a), [1,7,3]);
        assert.deepEqual(contents(b), [1,2,3]);
        assert.equal(e.equal(s, a), 1); assert.equal(e.equal(s, b), 0);
        assert.equal(e.ranges(2), 19); assert.deepEqual(events.splice(0), [1,5,2]);
        assert.throws(() => e.ranges(0), WebAssembly.RuntimeError);
        assert.deepEqual(events.splice(0), [1,5,0]);
        assert.equal(e.ranges(2), 19); assert.deepEqual(events.splice(0), [1,5,2]);
        assert.equal(e.remainder(7, 2), 1); assert.equal(e.remainder(-7, 2), -1);
        assert.ok(Object.is(e.remainder(-4, 2), -0));
        assert.ok(Number.isNaN(e.remainder(Infinity, 2)));
        assert.ok(Number.isNaN(e.remainder(NaN, 2)));
        assert.equal(e.remainder(5, Infinity), 5);
        assert.equal(e.remainder(Number.MIN_VALUE, 2 * Number.MIN_VALUE), Number.MIN_VALUE);
        for (let i = 0; i < 3; i++) { e.array(); e.text(); }
        assert.deepEqual(contents(a), [1,7,3]); assert.equal(contents(s, true), 'hé😀');
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn helper_parameter_validation_precedes_body_even_for_unused_malformed_handles() {
    run(
        r#"
        fn text() -> string { let make = () => "guard"; return make(); }
        fn array() -> number[] { let make = () => [1]; return make(); }
        fn invoke(xs: number[], s: string) -> number {
            let unused = (a: number[], b: string, c: number) => { mark(90); c };
            return unused(xs, s, mark(1));
        }
        "#,
        &["text", "array", "invoke"],
        r#"
        const a = e.array() >>> 0, s = e.text() >>> 0;
        function length(h, n) { new DataView(e.memory.buffer).setUint32(h + 4, n, true); }
        assert.equal(e.invoke(a, s), 1); assert.deepEqual(events.splice(0), [1,90]);
        for (const [h, original] of [[a,1], [s,5]]) {
            onMark = n => { if (n === 1) length(h, 0xffffffff); };
            assert.throws(() => e.invoke(a, s), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [1]);
            length(h, original); onMark = () => {};
            assert.equal(e.invoke(a, s), 1); assert.deepEqual(events.splice(0), [1,90]);
        }
        for (const [xs, text] of [[0,s], [a,0], [-8,s], [a,-8]]) {
            assert.throws(() => e.invoke(xs, text), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), []);
        }
        assert.deepEqual(contents(a), [1]); assert.equal(contents(s, true), 'guard');
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn unused_range_parameters_validate_records_before_helper_body_and_recover_after_traps() {
    run(
        r#"
        fn invoke() -> number {
            let unused = (r: range, n: number) => { mark(90); n };
            return unused(17..=23 by 2, mark(1));
        }
        "#,
        &["invoke"],
        r#"
        assert.equal(e.invoke(), 1); assert.deepEqual(events.splice(0), [1,90]);
        for (const damage of ['step', 'bound', 'inclusive']) {
            onMark = n => {
                if (n !== 1) return;
                const v = new DataView(e.memory.buffer);
                let record;
                // Find the newest independently specified private range record;
                // no private handle is exposed through the host signature.
                for (let p = 8; p + 32 <= v.byteLength; p += 8) {
                    if (v.getBigInt64(p, true) === 17n && v.getBigInt64(p + 8, true) === 23n &&
                        v.getBigInt64(p + 16, true) === 2n && v.getUint32(p + 24, true) === 1) record = p;
                }
                assert.notEqual(record, undefined);
                if (damage === 'step') v.setBigInt64(record + 16, 0n, true);
                if (damage === 'bound') v.setBigInt64(record, 9007199254740992n, true);
                if (damage === 'inclusive') v.setUint32(record + 24, 2, true);
            };
            assert.throws(() => e.invoke(), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [1]);
            onMark = () => {};
            assert.equal(e.invoke(), 1); assert.deepEqual(events.splice(0), [1,90]);
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn shared_cells_preserve_all_types_rebinding_aliases_and_original_shadowed_bindings() {
    run(
        r#"
        fn main(n: number, flag: bool) -> number {
            let text = "old";
            let array = [1, 2];
            let alias = array;
            let interval = 1..3;
            let read = () => if flag && text == "new" && 5 in interval { n + array[0] } else { 0 };
            let change = () => {
                n = n + 1; flag = true; text = "new";
                array[0] = 7; array = [20]; interval = 5..=7;
            };
            change();
            n = n + 10; array[0] = array[0] + 2;
            { let n = 900; let text = "shadow"; mark(read()); };
            return read() + alias[0];
        }
        fn identity(n: number) -> number { let read = () => n; return read(); }
        fn nested(n: number) -> number {
            let outer = (seed: number) => {
                let local = seed;
                let first = () => { local = local + 2; local };
                let second = () => local;
                first(); local = local + 3; second()
            };
            return outer(n);
        }
        "#,
        &["main", "identity", "nested"],
        r#"
        assert.equal(e.main(1, 0), 41); assert.deepEqual(events.splice(0), [34]);
        assert.equal(e.main(10, 1), 50); assert.deepEqual(events.splice(0), [43]);
        assert.equal(e.nested(7), 12);
        for (const n of [NaN, Infinity, -Infinity, -0, Number.MIN_VALUE, Number.MAX_VALUE]) {
            assert.ok(Object.is(e.identity(n), n));
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn captured_scalar_compounds_save_old_values_and_write_the_original_shared_cell() {
    for (operator, initial, rhs, first, second) in [
        ("+=", 20, 3, 23.0, 26.0),
        ("-=", 20, 3, 17.0, 14.0),
        ("*=", 20, 3, 60.0, 180.0),
        ("/=", 20, 4, 5.0, 1.25),
        ("%=", 20, 6, 2.0, 2.0),
    ] {
        let source = format!(
            r#"
            fn main(n: number, operand: number) -> number {{
                let rhs = () => {{ mark(2); n = 100; operand }};
                mark(1); n {operator} rhs(); mark(n);
                let update = () => {{
                    n {operator} {{ mark(3); n = 200; operand }};
                    n
                }};
                mark(update());
                return n;
            }}
            fn before(n: number, operand: number) -> number {{
                n {operator} {{ mark(4); n = 300; operand }};
                let read = () => n;
                return read();
            }}
            "#
        );
        run(
            &source,
            &["main", "before"],
            &format!(
                r#"
                assert.equal(e.main({initial}, {rhs}), {second});
                assert.deepEqual(events.splice(0), [1,2,{first},3,{second}]);
                assert.equal(e.before({initial}, {rhs}), {first});
                assert.deepEqual(events.splice(0), [4]);
                "#
            ),
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn captured_indexed_places_keep_original_handles_addresses_and_old_elements_across_rhs_effects() {
    for (operator, operand, expected) in [
        ("=", 3, 3.0),
        ("+=", 3, 23.0),
        ("-=", 3, 17.0),
        ("*=", 3, 60.0),
        ("/=", 4, 5.0),
        ("%=", 6, 2.0),
    ] {
        let source = format!(
            r#"
            fn main(operand: number) -> number[] {{
                let array = [20, 21];
                let alias = array;
                let index = () => {{ mark(1); array = [60]; 0 }};
                let rhs = () => {{
                    mark(2); alias[0] = 100; array[0] = 101;
                    array = [80]; operand
                }};
                array[index()] {operator} rhs();
                mark(alias[0]); mark(array[0]);
                return alias;
            }}
            fn helper(array: number[], operand: number) -> number[] {{
                let alias = array;
                let update = () => {{
                    array[{{ mark(1); array = [60]; 0 }}] {operator} {{
                        mark(2); alias[0] = 100; array[0] = 101;
                        array = [80]; operand
                    }};
                    mark(array[0]); alias
                }};
                return update();
            }}
            fn make() -> number[] {{ return [20, 21]; }}
            "#
        );
        run(
            &source,
            &["main", "helper", "make"],
            &format!(
                r#"
                const first = e.main({operand});
                assert.deepEqual(contents(first), [{expected},21]);
                assert.deepEqual(events.splice(0), [1,2,{expected},80]);
                const original = e.make();
                assert.equal(e.helper(original, {operand}), original);
                assert.deepEqual(contents(original), [{expected},21]);
                assert.deepEqual(events.splice(0), [1,2,80]);
                assert.deepEqual(contents(first), [{expected},21]);
                "#
            ),
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn captured_indexed_places_trap_on_invalid_descriptors_and_bounds_before_rhs_events() {
    for operator in ["=", "+=", "-=", "*=", "/=", "%="] {
        let source = format!(
            r#"
            fn make() -> number[] {{ return [20, 21]; }}
            fn fail(array: number[], index: number, inside: bool) -> number {{
                let update = () => {{
                    mark(0); array[mark(index)] {operator} mark(9);
                }};
                if inside {{ update(); }} else {{
                    mark(0); array[mark(index)] {operator} mark(9);
                }}
                return 0;
            }}
            "#
        );
        run(
            &source,
            &["make", "fail"],
            r#"
            for (const inside of [0,1]) {
                for (const index of [-1, 0.5, 2, NaN, Infinity, -Infinity]) {
                    const h = e.make();
                    assert.throws(() => e.fail(h, index, inside), WebAssembly.RuntimeError);
                    assert.deepEqual(events.splice(0), [0,index]);
                    assert.deepEqual(contents(h), [20,21]);
                }
                for (const malformed of ['null', 'unaligned', 'overrun']) {
                    const h = e.make();
                    onMark = n => {
                        if (n !== 0) return;
                        const view = new DataView(e.memory.buffer);
                        if (malformed === 'null') view.setUint32(h, 0, true);
                        if (malformed === 'unaligned') view.setUint32(h, 9, true);
                        if (malformed === 'overrun') view.setUint32(h + 4, 0xffffffff, true);
                    };
                    assert.throws(() => e.fail(h, 1, inside), WebAssembly.RuntimeError);
                    assert.deepEqual(events.splice(0), [0,1]);
                    onMark = () => {};
                }
                const h = e.make();
                assert.equal(e.fail(h, -0, inside), 0);
                assert.deepEqual(events.splice(0), [0,-0,9]);
            }
            "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn captured_compounds_preserve_independent_ieee754_results_for_cells_and_elements() {
    for (column, operator) in ["+=", "-=", "*=", "/=", "%="].into_iter().enumerate() {
        let source = format!(
            r#"
            fn scalar(n: number, d: number) -> number {{
                let rhs = () => {{ mark(1); n = 99; d }};
                n {operator} rhs();
                let read = () => n;
                return read();
            }}
            fn scalar_helper(n: number, d: number) -> number {{
                let update = () => {{ n {operator} {{ mark(1); n = 99; d }}; n }};
                return update();
            }}
            fn element(n: number, d: number) -> number {{
                let array = [n];
                let alias = array;
                let rhs = () => {{ mark(1); alias[0] = 99; array = [88]; d }};
                array[0] {operator} rhs();
                return alias[0];
            }}
            fn element_helper(n: number, d: number) -> number {{
                let array = [n];
                let alias = array;
                let update = () => {{
                    array[0] {operator} {{ mark(1); alias[0] = 99; array = [88]; d }};
                }};
                update(); return alias[0];
            }}
            "#
        );
        run(
            &source,
            &["scalar", "scalar_helper", "element", "element_helper"],
            &format!(
                r#"
                // Literal expectations, not a copy of the guest remainder algorithm.
                const cases = [
                    [20, 6, [26,14,120,3.3333333333333335,2]],
                    [5.5, 2, [7.5,3.5,11,2.75,1.5]],
                    [-5.5, 2, [-3.5,-7.5,-11,-2.75,-1.5]],
                    [-0, 3, [3,-3,-0,-0,-0]],
                    [0, -3, [-3,3,-0,-0,0]],
                    [7, 0, [7,7,0,Infinity,NaN]],
                    [7, -0, [7,7,-0,-Infinity,NaN]],
                    [NaN, 2, [NaN,NaN,NaN,NaN,NaN]],
                    [2, NaN, [NaN,NaN,NaN,NaN,NaN]],
                    [Infinity, 2, [Infinity,Infinity,Infinity,Infinity,NaN]],
                    [-Infinity, 2, [-Infinity,-Infinity,-Infinity,-Infinity,NaN]],
                    [7, Infinity, [Infinity,-Infinity,Infinity,0,7]],
                    [-7, Infinity, [Infinity,-Infinity,-Infinity,-0,-7]],
                    [Infinity, Infinity, [Infinity,NaN,Infinity,NaN,NaN]],
                    [Number.MIN_VALUE, 2, [2,-2,1e-323,0,Number.MIN_VALUE]],
                    [-Number.MIN_VALUE, 2, [2,-2,-1e-323,-0,-Number.MIN_VALUE]],
                    [Number.MAX_VALUE, 2, [Number.MAX_VALUE,Number.MAX_VALUE,Infinity,8.988465674311579e307,0]],
                    [1, Number.MIN_VALUE, [1,1,Number.MIN_VALUE,Infinity,0]],
                ];
                for (const name of ['scalar','scalar_helper','element','element_helper']) {{
                    for (const [n,d,expected] of cases) {{
                        const actual = e[name](n,d);
                        assert.ok(Object.is(actual, expected[{column}]),
                            `${{name}}(${{n}},${{d}}): ${{actual}}`);
                        assert.deepEqual(events.splice(0), [1]);
                    }}
                }}
                "#
            ),
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn captured_compound_rhs_transfers_preserve_stack_prefixes_and_skip_stores() {
    for operator in ["+=", "-=", "*=", "/=", "%="] {
        for target in ["n", "array[mark(0)]"] {
            let source = format!(
                r#"
                fn returning() -> number {{
                    let n = 20; let array = [20];
                    let read = () => n + array[0];
                    return 10 + {{ {target} {operator} {{ mark(2); return read(); }}; 0 }};
                }}
                fn helper_return() -> number {{
                    let n = 20; let array = [20];
                    let update = () => {{
                        {target} {operator} {{ mark(2); return 40; }};
                        mark(99); 0
                    }};
                    return 10 + update() + n + array[0];
                }}
                fn breaking() -> number {{
                    let n = 20; let array = [20];
                    let read = () => n + array[0];
                    return 10 + loop {{
                        {target} {operator} {{ mark(2); break read(); }};
                        mark(99);
                    }};
                }}
                fn continuing() -> number {{
                    let n = 20; let array = [20];
                    let read = () => n + array[0];
                    let i = 0;
                    while i < 2 {{
                        i += 1;
                        {target} {operator} {{ mark(2); continue; }};
                        mark(99);
                    }}
                    if false {{
                        {target} {operator} loop {{}};
                    }}
                    return read();
                }}
                "#
            );
            let prefix = if target == "n" { "" } else { "0," };
            run(
                &source,
                &["returning", "helper_return", "breaking", "continuing"],
                &format!(
                    r#"
                    assert.equal(e.returning(), 40);
                    assert.deepEqual(events.splice(0), [{prefix}2]);
                    assert.equal(e.helper_return(), 90);
                    assert.deepEqual(events.splice(0), [{prefix}2]);
                    assert.equal(e.breaking(), 50);
                    assert.deepEqual(events.splice(0), [{prefix}2]);
                    assert.equal(e.continuing(), 40);
                    assert.deepEqual(events.splice(0), [{prefix}2,{prefix}2]);
                    "#
                ),
            );
        }
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn allocation_timing_preserves_parameter_contract_initializer_and_declaration_effects() {
    run(
        r#"
        fn array() -> number[] { return [1]; }
        fn entry(n: number, xs: number[]) -> number requires n > 0 {
            let read = () => { mark(90); n + len(xs) };
            mark(1); return read();
        }
        fn skipped(flag: bool) -> number {
            let n = mark(2);
            if flag { let unused = () => { mark(99); n }; }
            return n;
        }
        fn initialized() -> number {
            let n = { mark(3); mark(4) };
            let unused = () => { mark(99); n };
            return n;
        }
        fn early() -> number {
            let n: number = { mark(5); return 50; };
            let unused = () => n;
            return 0;
        }
        fn ordered() -> number {
            let n = 0;
            let set = (a: number, b: number) => { mark(8); n = a * 10 + b; n };
            return set(mark(6), mark(7));
        }
        "#,
        &[
            "array",
            "entry",
            "skipped",
            "initialized",
            "early",
            "ordered",
        ],
        r#"
        const xs = e.array();
        function allocated(action) {
            const before = e.__alloc(8) >>> 0;
            action();
            return (e.__alloc(8) >>> 0) - before - 8;
        }
        // Invalid explicit handles and failed contracts allocate no parameter cells.
        assert.equal(allocated(() => assert.throws(() => e.entry(1, 0), WebAssembly.RuntimeError)), 0);
        assert.equal(allocated(() => assert.throws(() => e.entry(0, xs), WebAssembly.RuntimeError)), 0);
        assert.deepEqual(events.splice(0), []);
        // Two cells plus a two-slot environment; declaration still executes no body.
        assert.equal(allocated(() => assert.equal(e.entry(2, xs), 3)), 32);
        assert.deepEqual(events.splice(0), [1,90]);
        assert.equal(allocated(() => assert.equal(e.skipped(0), 2)), 8);
        assert.deepEqual(events.splice(0), [2]);
        assert.equal(allocated(() => assert.equal(e.skipped(1), 2)), 16);
        assert.deepEqual(events.splice(0), [2]);
        let start;
        onMark = n => { if (n === 3) start = e.__alloc(8) >>> 0;
                         if (n === 4) assert.equal((e.__alloc(8) >>> 0) - start, 8); };
        assert.equal(e.initialized(), 4); assert.deepEqual(events.splice(0), [3,4]);
        onMark = () => {};
        assert.equal(allocated(() => assert.equal(e.early(), 50)), 0);
        assert.deepEqual(events.splice(0), [5]);
        assert.equal(e.ordered(), 67); assert.deepEqual(events.splice(0), [6,7,8]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn capture_storage_survives_recursive_reentrant_and_post_trap_invocations_and_memory_growth() {
    run(
        r#"
        fn recursive(n: number) -> number {
            let state = n;
            let update = () => { state = state + 10; state };
            if n > 0 { recursive(n - 1); }
            return update();
        }
        fn reenter(n: number) -> number {
            let state = n;
            let read = () => state;
            mark(n); state = state + 1; return read();
        }
        fn trapped(fail: bool) -> number[] {
            let array = [3, 4];
            let read = () => array;
            if fail { mark(7); array[9]; }
            array[0] = 8; return read();
        }
        fn text() -> string {
            let value = "retained hé😀";
            let read = () => value;
            return read();
        }
        fn grow(count: number) -> number {
            let i = 0;
            while i < count {
                let value = i;
                let read = () => value;
                read(); i += 1;
            }
            return i;
        }
        "#,
        &["recursive", "reenter", "trapped", "text", "grow"],
        r#"
        assert.equal(e.recursive(4), 14); assert.equal(e.recursive(0), 10);
        const retained = e.trapped(0), text = e.text();
        onMark = n => { if (n === 2) assert.equal(e.reenter(20), 21); };
        assert.equal(e.reenter(2), 3); assert.deepEqual(events.splice(0), [2,20]);
        onMark = () => {};
        for (let i = 0; i < 3; i++) {
            assert.throws(() => e.trapped(1), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [7]);
            assert.deepEqual(contents(e.trapped(0)), [8,4]);
        }
        const previous = e.memory.buffer, oldView = new DataView(previous);
        assert.equal(e.grow(9000), 9000);
        assert.notEqual(e.memory.buffer, previous);
        assert.equal(previous.byteLength, 0);
        assert.throws(() => oldView.getUint32(0, true), TypeError);
        assert.deepEqual(contents(retained), [8,4]);
        assert.equal(contents(text, true), 'retained hé😀');
        assert.equal(e.recursive(3), 13);
        "#,
    );
}
