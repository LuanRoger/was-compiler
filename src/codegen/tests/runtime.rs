use super::*;

#[test]
#[ignore = "requires Node.js"]
fn node_executes_numeric_literal_returns() {
    let bytes = Compiler::new()
        .compile_source(
            "fn answer() -> number { return 42; return 0; }
             fn decimal() -> number { return 0.25; }",
        )
        .expect("compile runtime numeric returns");

    decode(&bytes);
    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));

        assert.equal(instance.exports.answer(), 42);
        assert.equal(instance.exports.decimal(), 0.25);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_numeric_exports_with_unused_provider_imports() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function(
            "numeric",
            "env",
            "numeric",
            vec![Type::Number, Type::Bool],
            Type::Number,
        )
        .unwrap();
    compiler
        .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn answer(n: number, flag: bool) -> number { return 42; }
             fn decimal(flag: bool, n: number) -> number { return 0.25; }
             fn noop() -> void { return; }",
        )
        .expect("compile runtime imports and numeric exports");
    let module = decode(&bytes);

    assert_eq!(
        module.exports,
        vec![
            ("answer".to_owned(), ExternalKind::Func, 2),
            ("decimal".to_owned(), ExternalKind::Func, 3),
            ("noop".to_owned(), ExternalKind::Func, 4),
        ],
    );
    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        let calls = 0;
        const imports = {
            env: { numeric: (n, flag) => { calls += 1; return n + flag; } },
            provider: { ready: () => { calls += 1; return 1; } },
        };
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), imports);

        assert.equal(instance.exports.answer(100, 1), 42);
        assert.equal(instance.exports.decimal(0, 100), 0.25);
        assert.equal(instance.exports.noop(), undefined);
        assert.equal(calls, 0);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_nested_arithmetic_with_unused_provider_imports() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn precedence(n: number, flag: bool) -> number { return 1 + 2 * 3; }
             fn parentheses() -> number { return (1 + 2) * 3; }
             fn negate() -> number { return -(1 + 2); }
             fn subtract() -> number { return 10 - 3 - 2; }
             fn divide() -> number { return 20 / (2 + 3); }
             fn rightdivide() -> number { return 1 / (2 / 4); }
             fn rightsubtract() -> number { return 10 - (3 - 2); }
             fn noop() -> void {}",
        )
        .expect("compile runtime nested arithmetic");
    let module = decode(&bytes);

    assert_eq!(
        module.exports[0],
        ("precedence".to_owned(), ExternalKind::Func, 2)
    );
    assert_eq!(
        module.exports[7],
        ("noop".to_owned(), ExternalKind::Func, 9)
    );

    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        let calls = 0;
        const imports = {
            env: { numeric: () => { calls += 1; return 99; } },
            provider: { ready: () => { calls += 1; return 1; } },
        };
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), imports);

        assert.equal(instance.exports.precedence(100, 1), 7);
        assert.equal(instance.exports.parentheses(), 9);
        assert.equal(instance.exports.negate(), -3);
        assert.equal(instance.exports.subtract(), 5);
        assert.equal(instance.exports.divide(), 4);
        assert.equal(instance.exports.rightdivide(), 2);
        assert.equal(instance.exports.rightsubtract(), 9);
        assert.equal(instance.exports.noop(), undefined);
        assert.equal(calls, 0);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_arithmetic_with_ieee754_rounding_and_special_results() {
    let finite = format!("1{}", "0".repeat(200));
    let source = format!(
        "fn decimal() -> number {{ return 0.1 + 0.2; }}
         fn fraction() -> number {{ return 1 / 10; }}
         fn rounded() -> number {{ return 9007199254740993 + 1; }}
         fn nan() -> number {{ return 0 / 0; }}
         fn positiveinfinity() -> number {{ return 1 / 0; }}
         fn negativeinfinity() -> number {{ return -1 / 0; }}
         fn negativezero() -> number {{ return -0; }}
         fn dividedzero() -> number {{ return 0 / -1; }}
         fn positivezero() -> number {{ return -0 * -1; }}
         fn overflow() -> number {{ return {finite} * {finite}; }}
         fn remainder(left: number, right: number) -> number {{ return left % right; }}
         fn remainderarithmetic() -> number {{ return 2 + 17 % 5 * 3; }}
         fn remaindernegativezero() -> number {{ return -6 % 3; }}
         fn remaindernan() -> number {{ return 1 % 0; }}
         fn remainderinfinity() -> number {{ return -3 % (1 / 0); }}",
    );
    let bytes = Compiler::new()
        .compile_source(&source)
        .expect("compile runtime IEEE 754 arithmetic");
    let module = decode(&bytes);

    assert_eq!(
        module.bodies[9],
        vec![
            DecodedInstruction::F64Const(1e200),
            DecodedInstruction::F64Const(1e200),
            DecodedInstruction::F64Mul,
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ],
    );

    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));

        assert.equal(instance.exports.decimal(), 0.30000000000000004);
        assert.equal(instance.exports.fraction(), 0.1);
        assert.equal(instance.exports.rounded(), 9007199254740992);
        assert.ok(Number.isNaN(instance.exports.nan()));
        assert.equal(instance.exports.positiveinfinity(), Infinity);
        assert.equal(instance.exports.negativeinfinity(), -Infinity);
        assert.ok(Object.is(instance.exports.negativezero(), -0));
        assert.ok(Object.is(instance.exports.dividedzero(), -0));
        assert.ok(Object.is(instance.exports.positivezero(), 0));
        assert.equal(instance.exports.overflow(), Infinity);
        assert.equal(instance.exports.remainderarithmetic(), 8);
        assert.ok(Object.is(instance.exports.remaindernegativezero(), -0));
        assert.ok(Number.isNaN(instance.exports.remaindernan()));
        assert.equal(instance.exports.remainderinfinity(), -3);
        for (const [left, right] of [
            [5.5, 2], [-5.5, 2], [5.5, -2], [-5.5, -2],
            [-0, 3], [-6, 3], [Infinity, 3], [3, Infinity],
            [NaN, 3], [3, NaN], [1, 0],
            [Number.MAX_VALUE, 3], [Number.MAX_VALUE, Number.MIN_VALUE],
            [-Number.MIN_VALUE, Number.MAX_VALUE], [1e300, 1e-300],
        ]) {
            assert.ok(Object.is(instance.exports.remainder(left, right), left % right));
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_parameterized_arithmetic_and_numeric_mutations_at_all_optimization_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
            .unwrap();
        compiler
            .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn calculate(n: number, scale: number) -> number {
                 let value: number = n * scale + 1;
                 n = value - 3;
                 value = -n / 2;
                 value + 99;
                 return value;
             }
             fn passthrough(n: number) -> number { return n; }",
            )
            .expect("compile parameterized numeric runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("calculate".to_owned(), ExternalKind::Func, 2)
        );
        assert_eq!(module.locals, vec![vec![ValType::F64], vec![]]);

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            let calls = 0;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: { numeric: () => { calls += 1; return 99; } },
                provider: { ready: () => { calls += 1; return 1; } },
            });

            assert.equal(instance.exports.calculate(4, 3), -5);
            assert.equal(instance.exports.calculate(2, 0.5), 0.5);
            assert.equal(instance.exports.calculate(-2, 4), 5);
            assert.equal(instance.exports.passthrough(0.25), 0.25);
            assert.ok(Object.is(instance.exports.passthrough(-0), -0));
            assert.equal(calls, 0);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_bool_assignments_returns_and_drops_at_all_optimization_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("unused", "env", "unused", vec![Type::Bool], Type::Bool)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn saved(flag: bool, n: number) -> bool {
                 let value: number = n + 1;
                 let copy: bool = flag;
                 let ready: bool = false;
                 ready = copy;
                 flag = true;
                 value = value * 2;
                 value;
                 flag;
                 false;
                 return ready;
             }
             fn reset(flag: bool) -> bool { flag = false; return flag; }
             fn yes() -> bool { return true; }
             fn discard(n: number, flag: bool) -> void {
                 let copy: bool = flag;
                 copy = true;
                 n = n + 2;
                 n;
                 copy;
             }",
            )
            .expect("compile bool assignment runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("saved".to_owned(), ExternalKind::Func, 1)
        );
        assert_eq!(
            module.locals,
            vec![
                vec![ValType::F64, ValType::I32, ValType::I32],
                vec![],
                vec![],
                vec![ValType::I32],
            ]
        );

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            let calls = 0;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: { unused: flag => { calls += 1; return flag; } },
            });

            assert.equal(instance.exports.saved(0, 7), 0);
            assert.equal(instance.exports.saved(1, 7), 1);
            assert.equal(instance.exports.reset(1), 0);
            assert.equal(instance.exports.yes(), 1);
            assert.equal(instance.exports.discard(3, 0), undefined);
            assert.equal(instance.exports.discard(9, 1), undefined);
            assert.equal(calls, 0);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_nested_user_host_calls_left_to_right_with_void_side_effects() {
    let mut snapshot = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        for (name, field, params, result) in [
            ("unused", "unused", vec![], Type::Void),
            ("mark", "mark", vec![Type::Number], Type::Number),
            ("join", "join", vec![Type::Number; 3], Type::Number),
            ("print", "log", vec![Type::Number], Type::Void),
        ] {
            compiler
                .register_host_function(name, "env", field, params, result)
                .unwrap();
        }

        let bytes = compiler
            .compile_source(
                "fn entry() -> number {
                    let result: number = combine(mark(1), identity(mark(2)), mark(identity(mark(3))));
                    relay(result);
                    combine(mark(4), mark(5), mark(6));
                    return result;
                }
                fn identity(n: number) -> number { return n; }
                fn combine(a: number, b: number, c: number) -> number { return join(a, b, c); }
                fn relay(n: number) -> void { print(n); }",
            )
            .expect("compile nested user and host runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("entry".to_owned(), ExternalKind::Func, 4)
        );
        assert_eq!(
            module.bodies[0],
            vec![
                DecodedInstruction::F64Const(1.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::F64Const(2.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(5),
                DecodedInstruction::F64Const(3.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(5),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(6),
                DecodedInstruction::LocalSet(0),
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(7),
                DecodedInstruction::F64Const(4.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::F64Const(5.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::F64Const(6.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(6),
                DecodedInstruction::Drop,
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const events = [];
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    unused: () => assert.fail('unused import called'),
                    mark: n => { events.push(['mark', n]); return n + 10; },
                    join: (a, b, c) => { events.push(['join', a, b, c]); return a * 100 + b * 10 + c; },
                    log: n => { events.push(['log', n]); return 999; },
                },
            });

            assert.equal(instance.exports.entry(), 1243);
            assert.deepEqual(events, [
                ['mark', 1], ['mark', 2], ['mark', 3], ['mark', 13],
                ['join', 11, 12, 23], ['log', 1243],
                ['mark', 4], ['mark', 5], ['mark', 6], ['join', 14, 15, 16],
            ]);
            events.length = 0;
            assert.equal(instance.exports.relay(7), undefined);
            assert.deepEqual(events, [['log', 7]]);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_host_bool_normalization_ieee754_numbers_and_import_link_errors() {
    let mut snapshot = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function(
                "host_bool",
                "env",
                "predicate",
                vec![Type::Bool, Type::Number, Type::Bool],
                Type::Bool,
            )
            .unwrap();
        compiler
            .register_host_function(
                "host_number",
                "env",
                "numeric",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn bool_bridge(flag: bool, n: number, other: bool) -> bool {
                    return guest_bool(host_bool(flag, n, other));
                }
                fn guest_bool(flag: bool) -> bool { return flag; }
                fn raw_bool(flag: bool) -> bool { return guest_bool(flag); }
                fn number_bridge(n: number) -> number { return guest_number(host_number(n)); }
                fn guest_number(n: number) -> number { return n; }",
            )
            .expect("compile host ABI runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("bool_bridge".to_owned(), ExternalKind::Func, 2)
        );
        assert_eq!(
            module.bodies[0],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::LocalGet(1),
                DecodedInstruction::LocalGet(2),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::Call(0),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::Call(3),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        );
        assert_eq!(
            module.bodies[2],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(3),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const module = new WebAssembly.Module(bytes);
            let result = 0;
            const booleans = [];
            const numbers = [];
            const imports = {
                env: {
                    predicate: (flag, n, other) => { booleans.push([flag, n, other]); return result; },
                    numeric: n => { numbers.push(n); return n; },
                },
            };
            const instance = new WebAssembly.Instance(module, imports);

            for (const raw of [2, -2, 0]) {
                result = raw;
                assert.equal(instance.exports.bool_bridge(2, 7.5, -2), raw === 0 ? 0 : 1);
                assert.equal(instance.exports.bool_bridge(0, 9, 0), raw === 0 ? 0 : 1);
            }
            assert.deepEqual(booleans, [
                [1, 7.5, 1], [0, 9, 0], [1, 7.5, 1], [0, 9, 0], [1, 7.5, 1], [0, 9, 0],
            ]);
            for (const raw of [2, -2, 0]) {
                assert.equal(instance.exports.raw_bool(raw), raw);
            }
            for (const n of [NaN, Infinity, -Infinity, -0, 0.25]) {
                assert.ok(Object.is(instance.exports.number_bridge(n), n));
                assert.ok(Object.is(numbers.at(-1), n));
            }
            for (const field of ['predicate', 'numeric']) {
                for (const value of [undefined, null, 7, {}]) {
                    const env = { ...imports.env, [field]: value };
                    assert.throws(() => new WebAssembly.Instance(module, { env }), WebAssembly.LinkError);
                }
                const env = { ...imports.env };
                delete env[field];
                assert.throws(() => new WebAssembly.Instance(module, { env }), WebAssembly.LinkError);
            }
            assert.throws(() => new WebAssembly.Instance(module, {}), TypeError);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_self_and_mutual_recursion_with_a_bounded_host_tick() {
    let mut snapshot = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("unused", "env", "unused", vec![], Type::Void)
            .unwrap();
        compiler
            .register_host_function("tick", "env", "tick", vec![Type::Number], Type::Void)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn recurse(n: number) -> number { tick(n); return recurse(n + 1); }
                fn left(n: number) -> void { tick(n); right(n + 1); }
                fn right(n: number) -> void { tick(n); left(n + 1); }",
            )
            .expect("compile bounded recursive runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports,
            vec![
                ("recurse".to_owned(), ExternalKind::Func, 2),
                ("left".to_owned(), ExternalKind::Func, 3),
                ("right".to_owned(), ExternalKind::Func, 4),
            ],
        );
        assert_eq!(
            module.bodies,
            vec![
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(1),
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(2),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(1),
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(4),
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(1),
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(3),
                    DecodedInstruction::End,
                ],
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const sentinel = new Error('bounded tick sentinel');
            const seen = [];
            let remaining = 0;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    unused: () => assert.fail('unused import called'),
                    tick: n => {
                        seen.push(n);
                        remaining -= 1;
                        if (remaining === 0) throw sentinel;
                    },
                },
            });

            for (const name of ['recurse', 'left', 'right']) {
                for (const bound of [1, 2, 7, 12]) {
                    remaining = bound;
                    seen.length = 0;
                    assert.throws(() => instance.exports[name](10), error => error === sentinel);
                    assert.equal(remaining, 0);
                    assert.deepEqual(seen, Array.from({ length: bound }, (_, i) => 10 + i));
                }
            }
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_comparisons_ieee754_and_noncanonical_bool_operators_at_all_levels() {
    run_guest_at_all_optimization_levels(
        "fn eq(a: number, b: number) -> bool { return a == b; }
         fn ne(a: number, b: number) -> bool { return a != b; }
         fn lt(a: number, b: number) -> bool { return a < b; }
         fn le(a: number, b: number) -> bool { return a <= b; }
         fn gt(a: number, b: number) -> bool { return a > b; }
         fn ge(a: number, b: number) -> bool { return a >= b; }
         fn bool_eq(a: bool, b: bool) -> bool { return a == b; }
         fn bool_ne(a: bool, b: bool) -> bool { return a != b; }
         fn invert(a: bool) -> bool { return !a; }
         fn both(a: bool, b: bool) -> bool { return a && b; }
         fn either(a: bool, b: bool) -> bool { return a || b; }
         fn nested(a: bool, b: bool) -> bool { return (a && !b) || (a != b); }
         fn precedence(a: bool, b: bool, c: bool) -> bool { return a || b && !c; }
         fn compare_and(a: bool, b: bool, c: bool) -> bool { return a == (b && c); }
         fn invoke_or(a: bool, b: bool) -> number { return take(7, a || b); }
         fn take(value: number, flag: bool) -> number {
             if flag { return value; } else { return -value; }
         }",
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes));
        const comparisons = {
            eq: (a, b) => a === b,
            ne: (a, b) => a !== b,
            lt: (a, b) => a < b,
            le: (a, b) => a <= b,
            gt: (a, b) => a > b,
            ge: (a, b) => a >= b,
        };
        const numbers = [NaN, Infinity, -Infinity, -0, 0, -1, 0.25, 1, Number.MIN_VALUE, Number.MAX_VALUE];

        for (const a of numbers) {
            for (const b of numbers) {
                for (const [name, compare] of Object.entries(comparisons)) {
                    assert.equal(guest[name](a, b), Number(compare(a, b)), `${name}(${a}, ${b})`);
                }
            }
        }

        const booleans = [0, 1, 2, -2, 2147483647, -2147483648];

        for (const a of booleans) {
            assert.equal(guest.invert(a), Number(!a));

            for (const b of booleans) {
                assert.equal(guest.bool_eq(a, b), Number(Boolean(a) === Boolean(b)));
                assert.equal(guest.bool_ne(a, b), Number(Boolean(a) !== Boolean(b)));
                assert.equal(guest.both(a, b), Number(Boolean(a) && Boolean(b)));
                assert.equal(guest.either(a, b), Number(Boolean(a) || Boolean(b)));
                assert.equal(guest.invoke_or(a, b), a || b ? 7 : -7);
                assert.equal(guest.nested(a, b), Number((Boolean(a) && !b) || (Boolean(a) !== Boolean(b))));

                for (const c of booleans) {
                    assert.equal(guest.precedence(a, b, c), Number(Boolean(a) || (Boolean(b) && !c)));
                    assert.equal(guest.compare_and(a, b, c), Number(Boolean(a) === (Boolean(b) && Boolean(c))));
                }
            }
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_short_circuit_host_effects_in_order_at_all_levels() {
    let mut snapshot = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("probe", "env", "probe", vec![Type::Number], Type::Bool)
            .unwrap();
        compiler
            .register_host_function(
                "numeric",
                "env",
                "numeric",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn both() -> bool { return probe(1) && probe(2); }
             fn either() -> bool { return probe(1) || probe(2); }
             fn nested_or() -> bool { return (probe(1) && probe(2)) || (probe(3) && probe(4)); }
             fn nested_and() -> bool { return probe(1) && (probe(2) || probe(3)); }
             fn negated() -> bool { return !probe(1) || probe(2); }
             fn equal() -> bool { return probe(1) == probe(2); }
             fn unequal() -> bool { return probe(1) != probe(2); }
             fn numeric_compare() -> bool { return numeric(1) < numeric(2); }
             fn guarded_compare() -> bool { return probe(1) && numeric(2) >= numeric(3); }",
            )
            .expect("compile host effects runtime");

        decode(&bytes);

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const seen = [];
            const sentinel = new Error('evaluated host call');
            let values = [];
            const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    probe: id => {
                        seen.push(`p${id}`);
                        const value = values[id - 1];
                        if (value === sentinel) throw sentinel;
                        return value;
                    },
                    numeric: id => { seen.push(`n${id}`); return id; },
                },
            });
            const expressions = {
                both: p => p(1) && p(2),
                either: p => p(1) || p(2),
                nested_or: p => (p(1) && p(2)) || (p(3) && p(4)),
                nested_and: p => p(1) && (p(2) || p(3)),
                negated: p => !p(1) || p(2),
                equal: p => p(1) === p(2),
                unequal: p => p(1) !== p(2),
            };

            for (const a of [0, 2, -2]) {
                for (const b of [0, 2, -2]) {
                    for (const c of [0, 2, -2]) {
                        for (const d of [0, 2, -2]) {
                            values = [a, b, c, d];

                            for (const [name, expression] of Object.entries(expressions)) {
                                const expectedSeen = [];
                                const result = expression(id => { expectedSeen.push(`p${id}`); return Boolean(values[id - 1]); });
                                seen.length = 0;
                                assert.equal(guest[name](), Number(result), name);
                                assert.deepEqual(seen, expectedSeen, name);
                            }
                        }
                    }
                }
            }

            for (const [name, skip] of [['both', 0], ['either', -2], ['negated', 0]]) {
                values = [skip, sentinel];
                seen.length = 0;
                assert.equal(guest[name](), name === 'both' ? 0 : 1);
                assert.deepEqual(seen, ['p1']);
                values[0] = skip === 0 ? -2 : 0;
                seen.length = 0;
                assert.throws(() => guest[name](), error => error === sentinel);
                assert.deepEqual(seen, ['p1', 'p2']);
            }

            seen.length = 0;
            assert.equal(guest.numeric_compare(), 1);
            assert.deepEqual(seen, ['n1', 'n2']);

            for (const raw of [0, 2, -2]) {
                values = [raw];
                seen.length = 0;
                assert.equal(guest.guarded_compare(), 0);
                assert.deepEqual(seen, raw === 0 ? ['p1'] : ['p1', 'n2', 'n3']);
            }
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_host_if_and_repeated_loop_conditions_at_all_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("probe", "env", "probe", vec![], Type::Bool)
            .unwrap();
        compiler
            .register_host_function(
                "numeric",
                "env",
                "numeric",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();

        let bytes = compiler.compile_source(
            "fn choose() -> number { if probe() { return numeric(2); } else { return numeric(3); } }
             fn repeat() -> number { let n: number = 0; while probe() { n = n + 1; } return n; }",
        ).expect("compile host control flow runtime");

        decode(&bytes);
        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const seen = [];
            let queue = [];
            const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    probe: () => {
                        assert.ok(queue.length > 0, 'condition evaluated too many times');
                        seen.push('probe');
                        return queue.shift();
                    },
                    numeric: n => { seen.push(n); return n; },
                },
            });

            for (const raw of [0, 2, -2]) {
                queue = [raw];
                seen.length = 0;
                assert.equal(guest.choose(), raw === 0 ? 3 : 2);
                assert.deepEqual(seen, ['probe', raw === 0 ? 3 : 2]);
                assert.deepEqual(queue, []);
            }

            for (const conditions of [[0], [2, 0], [2, -2, 0]]) {
                queue = [...conditions];
                seen.length = 0;
                assert.equal(guest.repeat(), conditions.length - 1);
                assert.deepEqual(seen, conditions.map(() => 'probe'));
                assert.deepEqual(queue, []);
            }
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_if_loops_early_returns_and_nested_shadowing_at_all_levels() {
    run_guest_at_all_optimization_levels(
        "fn choose(flag: bool) -> number { if flag { return 1; } else { return 2; } }
         fn partial(a: bool, b: bool) -> number { if a { if b { return 1; } } else { return 2; } return 3; }
         fn sum(n: number) -> number {
             let total: number = 0;
             while n > 0 { let current: number = n; total = total + current; n = n - 1; }
             return total;
         }
         fn nested(n: number, m: number) -> number {
             let total: number = 0;
             let i: number = 0;
             while i < n {
                 let j: number = 0;
                 while j < m { let term: number = i * 10 + j; total = total + term; j = j + 1; }
                 i = i + 1;
             }
             return total;
         }
         fn find(n: number, m: number, target: number) -> number {
             let i: number = 0;
             while i < n {
                 let j: number = 0;
                 while j < m {
                     let term: number = i * 10 + j;
                     if term == target { return term; }
                     j = j + 1;
                 }
                 i = i + 1;
             }
             return -1;
         }
         fn early(n: number) -> number { while n > 0 { if n == 3 { return n; } n = n - 1; } return 0; }
         fn loop_flag(flag: bool) -> number { let n: number = 0; while flag { n = n + 1; flag = false; } return n; }
         fn shadow(flag: bool, n: number) -> number {
             let result: number = n;
             if flag {
                 let n: bool = true;
                 if n { let result: number = 99; result; }
                 result = result + 1;
             } else { let n: number = 10; result = result + n; }
             let remaining: number = n;
             while remaining > 0 {
                 let n: bool = false;
                 let result: number = 5;
                 if !n { result = result + 1; }
                 remaining = remaining - 1;
             }
             return result + n;
         }",
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes));

        for (const a of [0, 1, 2, -2]) {
            assert.equal(guest.choose(a), a === 0 ? 2 : 1);
            assert.equal(guest.loop_flag(a), a === 0 ? 0 : 1);

            for (const b of [0, 1, 2, -2]) {
                assert.equal(guest.partial(a, b), a === 0 ? 2 : b === 0 ? 3 : 1);
            }

            for (const n of [0, 1, 2, 5]) {
                assert.equal(guest.shadow(a, n), 2 * n + (a === 0 ? 10 : 1));
            }
        }

        for (const n of [0, 1, 2, 5, 10]) {
            assert.equal(guest.sum(n), n * (n + 1) / 2);
            assert.equal(guest.early(n), n >= 3 ? 3 : 0);

            for (const m of [0, 1, 3, 5]) {
                const terms = [];

                for (let i = 0; i < n; i += 1) {
                    for (let j = 0; j < m; j += 1) terms.push(i * 10 + j);
                }

                assert.equal(guest.nested(n, m), terms.reduce((total, term) => total + term, 0));

                for (const target of [-1, 0, 2, 10, 21, 44, 99]) {
                    assert.equal(guest.find(n, m, target), terms.includes(target) ? target : -1);
                }
            }
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_guest_only_factorial_and_mutual_recursion_base_cases_at_all_levels() {
    run_guest_at_all_optimization_levels(
        "fn factorial(n: number) -> number {
             if n <= 1 { return 1; } else { return n * factorial(n - 1); }
         }
         fn even(n: number) -> bool { if n <= 0 { return true; } else { return odd(n - 1); } }
         fn odd(n: number) -> bool { if n <= 0 { return false; } else { return even(n - 1); } }",
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes));
        let factorial = 1;

        for (let n = 0; n <= 12; n += 1) {
            if (n > 0) factorial *= n;
            assert.equal(guest.factorial(n), factorial);
        }

        for (let n = 0; n <= 30; n += 1) {
            assert.equal(guest.even(n), Number(n % 2 === 0));
            assert.equal(guest.odd(n), Number(n % 2 !== 0));
        }

        assert.equal(guest.factorial(-1), 1);
        assert.equal(guest.even(-1), 1);
        assert.equal(guest.odd(-1), 0);
        "#,
    );
}
