use super::*;
use crate::{codegen::compile_with_exports, exports::ExportOptions};

const OPERATORS: [(BinaryOp, &str); 5] = [
    (BinaryOp::Add, "+="),
    (BinaryOp::Subtract, "-="),
    (BinaryOp::Multiply, "*="),
    (BinaryOp::Divide, "/="),
    (BinaryOp::Remainder, "%="),
];

fn compound(target: TypedExpr, op: BinaryOp, value: TypedExpr, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::CompoundAssignment { target, op, value },
        span,
    }
}

fn element(base: TypedExpr, index: TypedExpr) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Index {
            expr: Box::new(base),
            index: Box::new(index),
        },
        ty: Type::Number.into(),
    }
}

fn arithmetic(op: BinaryOp, remainder_index: u32) -> DecodedInstruction {
    match op {
        BinaryOp::Add => DecodedInstruction::F64Add,
        BinaryOp::Subtract => DecodedInstruction::F64Sub,
        BinaryOp::Multiply => DecodedInstruction::F64Mul,
        BinaryOp::Divide => DecodedInstruction::F64Div,
        BinaryOp::Remainder => DecodedInstruction::Call(remainder_index),
        _ => panic!("test requires arithmetic operator"),
    }
}

#[test]
fn scalar_compounds_keep_old_value_on_stack_and_need_no_memory_import_or_scratch() {
    use DecodedInstruction::*;

    for (op, token) in OPERATORS {
        let source = format!("fn update(n: number) -> number {{ n {token} n + 2; return n; }}");
        let mut snapshot = None;
        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let compiler = Compiler::with_options(CompilerOptions {
                optimization_level: level,
            });
            let bytes = compiler.compile_source(&source).unwrap();
            let module = decode(&bytes);
            assert_eq!(module.sections, [1, 2, 3, 7, 10]);
            assert!(module.imports.is_empty());
            assert!(module.locals[0].is_empty());
            assert_eq!(
                module.bodies[0],
                [
                    LocalGet(0),
                    LocalGet(0),
                    F64Const(2.0),
                    F64Add,
                    arithmetic(op, 1),
                    LocalSet(0),
                    LocalGet(0),
                    Return,
                    End
                ]
            );
            assert_eq!(
                module.functions.len(),
                if op == BinaryOp::Remainder { 2 } else { 1 }
            );
            if let Some(expected) = &snapshot {
                assert_eq!(&bytes, expected);
            } else {
                snapshot = Some(bytes);
            }
        }
    }
}

#[test]
fn local_compound_live_old_value_survives_array_construction_and_branching_callee() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn rhs(a: number[]) -> number {
            if len(a) == 1 { return a[0]; } else { return 0; }
        }
        fn update(n: number) -> number { n += rhs([n]); return n; }",
        )
        .unwrap();
    let module = decode(&bytes);
    assert_eq!(module.locals[1], [ValType::I32]); // only the array literal scratch
    assert_eq!(
        &module.bodies[1][..4],
        &[LocalGet(0), I32Const(16), Call(2), LocalSet(1)]
    );
    assert_eq!(
        &module.bodies[1][module.bodies[1].len() - 7..],
        &[
            LocalGet(1),
            Call(0),
            F64Add,
            LocalSet(0),
            LocalGet(0),
            Return,
            End
        ]
    );
}

#[test]
fn indexed_compounds_decode_once_only_address_and_old_load_before_rhs_for_all_operators() {
    use DecodedInstruction::*;

    let mut compiler = Compiler::new();
    compiler
        .register_host_function("index", "env", "index", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("rhs", "env", "rhs", vec![], Type::Number)
        .unwrap();
    for (op, token) in OPERATORS {
        let source = format!("fn update(a: number[]) -> void {{ a[index()] {token} rhs(); }}");
        let bytes = compiler.compile_source(&source).unwrap();
        let module = decode(&bytes);
        assert_eq!(module.locals[0], [ValType::I32, ValType::F64]);
        assert_eq!(module.imports.len(), 2);
        assert_eq!(
            module.functions.len(),
            if op == BinaryOp::Remainder { 9 } else { 8 }
        );
        assert_eq!(
            module.bodies[0],
            [
                LocalGet(0),
                Call(5),
                Drop, // parameter descriptor validation
                LocalGet(0),
                Call(0),
                Call(6), // index once, then checked address
                LocalSet(1),
                LocalGet(1),
                F64Load {
                    offset: 0,
                    align: 3,
                    memory: 0
                },
                LocalSet(2),
                LocalGet(1),
                LocalGet(2),
                Call(1),
                arithmetic(op, 10),
                F64Store {
                    offset: 0,
                    align: 3,
                    memory: 0
                },
                End,
            ]
        );
        if op == BinaryOp::Remainder {
            let ordinary = compiler
                .compile_source("fn update(a: number[]) -> void { a[index()] = a[0] % rhs(); }")
                .unwrap();
            decode(&ordinary);
            let last_body = |bytes: &[u8]| {
                Parser::new(0)
                    .parse_all(bytes)
                    .filter_map(|payload| match payload.unwrap() {
                        Payload::CodeSectionEntry(body) => Some(body.as_bytes().to_vec()),
                        _ => None,
                    })
                    .last()
                    .unwrap()
            };
            assert_eq!(last_body(&bytes), last_body(&ordinary));
        }
    }
}

#[test]
fn typed_scratch_and_array_temporaries_emit_in_planned_order_without_reuse() {
    let bytes = Compiler::new()
        .compile_source(
            "fn update(a: number[]) -> void {
            let first = [1];
            a[len([2]) - 1] += len([3]);
            if false { a[0] -= 4; }
            while false { a[0] *= 5; }
            return;
            a[0] /= 6;
            let last = [7];
        }",
        )
        .unwrap();
    let module = decode(&bytes);
    assert_eq!(
        module.locals[0],
        [
            ValType::I32,
            ValType::I32, // declarations first and last
            ValType::I32, // first literal
            ValType::I32,
            ValType::F64, // address and old value
            ValType::I32,
            ValType::I32, // target index and RHS literals
            ValType::I32,
            ValType::F64, // conditional scratch
            ValType::I32,
            ValType::F64, // loop scratch
            ValType::I32,
            ValType::F64, // unreachable scratch
            ValType::I32, // last literal
        ]
    );
    let sets: Vec<_> = module.bodies[0]
        .iter()
        .filter_map(|instruction| {
            if let DecodedInstruction::LocalSet(index) = instruction {
                Some(*index)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(sets, [3, 1, 6, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 2]);
}

#[test]
fn legacy_array_temporary_groups_and_preorder_slots_remain_byte_identical() {
    let program = Compiler::new()
        .analyze(
            "fn old() -> void {
            let a = [len([1]), 2];
            let b = [3];
            a[0] = len([4]);
        }",
        )
        .unwrap();
    assert_eq!(
        super::super::temporaries::plan(&program.functions[0]).unwrap(),
        [wasm_encoder::ValType::I32; 4]
    );
    let bytes = compile(&program).unwrap();
    let module = decode(&bytes);
    let sets: Vec<_> = module.bodies[0]
        .iter()
        .filter_map(|instruction| {
            if let DecodedInstruction::LocalSet(index) = instruction {
                Some(*index)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(sets, [2, 3, 0, 4, 1, 5]);
    let body = Parser::new(0)
        .parse_all(&bytes)
        .find_map(|payload| {
            if let Payload::CodeSectionEntry(body) = payload.unwrap() {
                Some(body)
            } else {
                None
            }
        })
        .unwrap();
    let groups = body
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        groups,
        [(1, ValType::I32), (1, ValType::I32), (4, ValType::I32)]
    );
    // Independent legacy local-declaration encoding: two declarations, then one
    // count-four i32 scratch group, without coalescing declaration groups.
    assert_eq!(&body.as_bytes()[..7], &[3, 1, 0x7f, 1, 0x7f, 4, 0x7f]);
}

#[test]
fn remainder_in_hidden_unreachable_and_runtime_skipped_compounds_plans_exact_helper() {
    for body in [
        "n %= 2;",
        "return; n %= 2;",
        "if false { n %= 2; }",
        "while false { n %= 2; }",
    ] {
        let source = format!("fn visible() -> void {{}} fn hidden(n: number) -> void {{ {body} }}");
        let program = Compiler::new().analyze(&source).unwrap();
        for exports in [
            ExportOptions::All,
            ExportOptions::None,
            ExportOptions::Only(vec!["visible".into()]),
        ] {
            let bytes = compile_with_exports(&program, &exports).unwrap();
            let module = decode(&bytes);
            assert_eq!(module.sections, [1, 2, 3, 7, 10]);
            assert_eq!(module.functions, [0, 1, 2]);
            assert_eq!(
                module.signatures[2],
                (vec![ValType::F64, ValType::F64], vec![ValType::F64])
            );
            assert!(module.bodies[1].contains(&DecodedInstruction::Call(2)));
            assert!(module.locals[1].is_empty());
        }
    }
}

#[test]
fn forged_non_arithmetic_compound_operators_are_rejected_with_statement_spans() {
    let span = Span::new(12, 34);
    for (op, operator) in [
        (BinaryOp::Equal, "=="),
        (BinaryOp::NotEqual, "!="),
        (BinaryOp::LessThan, "<"),
        (BinaryOp::LessThanOrEqual, "<="),
        (BinaryOp::GreaterThan, ">"),
        (BinaryOp::GreaterThanOrEqual, ">="),
        (BinaryOp::And, "&&"),
        (BinaryOp::Or, "||"),
    ] {
        let program = local_program(vec![
            declaration(0, Type::Number, number(), span),
            compound(identifier(0, "x", Type::Number), op, number(), span),
        ]);
        let expected = CodegenError::UnsupportedOperator {
            function: "bad".into(),
            operator,
            span,
        };
        assert_eq!(compile(&program), Err(expected));
    }
}

#[test]
fn forged_compound_places_types_ids_and_nested_expressions_are_checked() {
    let span = Span::new(12, 34);
    let mismatch = |expected, found| CodegenError::TypeMismatch {
        function: "bad".into(),
        expected,
        found,
        span,
    };
    let invalid_target = || CodegenError::InvalidAssignmentTarget {
        function: "bad".into(),
        span,
    };
    let array_local = || identifier(0, "a", Type::NumberArray);
    let indexed = || element(array_local(), number());
    let mut wrong_root = indexed();
    wrong_root.ty = Type::Bool.into();
    let invalid_number = || CodegenError::InvalidNumberLiteral {
        function: "bad".into(),
        literal: "invalid".into(),
        span,
    };
    let cases = [
        (number(), number(), invalid_target()),
        (element(number(), number()), number(), invalid_target()),
        (wrong_root, number(), mismatch(Type::Number, Type::Bool)),
        (
            element(identifier(0, "a", Type::Bool), number()),
            number(),
            mismatch(Type::NumberArray, Type::Bool),
        ),
        (
            element(array_local(), boolean()),
            number(),
            mismatch(Type::Number, Type::Bool),
        ),
        (indexed(), boolean(), mismatch(Type::Number, Type::Bool)),
        (
            element(
                identifier(0, "a", Type::NumberArray),
                number_literal("invalid"),
            ),
            number(),
            invalid_number(),
        ),
        (
            indexed(),
            binary(number(), BinaryOp::Add, number_literal("invalid")),
            invalid_number(),
        ),
        (
            element(identifier(99, "unknown", Type::NumberArray), number()),
            number(),
            CodegenError::UnknownVariable {
                function: "bad".into(),
                id: VariableId(99),
                span,
            },
        ),
        (
            element(identifier(1, "numeric", Type::NumberArray), number()),
            number(),
            mismatch(Type::Number, Type::NumberArray),
        ),
    ];
    for (target, value, expected) in cases {
        let mut function = user(0, "bad", &[Type::NumberArray, Type::Number], Type::Void);
        function
            .body
            .push(compound(target, BinaryOp::Add, value, span));
        let program = TypedProgram {
            functions: vec![function],
            host_functions: vec![],
        };
        assert_eq!(compile(&program), Err(expected));
    }

    for (target, value, expected) in [
        (
            identifier(99, "unknown", Type::Number),
            number(),
            CodegenError::UnknownVariable {
                function: "bad".into(),
                id: VariableId(99),
                span,
            },
        ),
        (
            identifier(0, "n", Type::Bool),
            number(),
            mismatch(Type::Number, Type::Bool),
        ),
        (
            identifier(0, "n", Type::Number),
            boolean(),
            mismatch(Type::Number, Type::Bool),
        ),
        (
            identifier(0, "n", Type::Number),
            TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Number.into(),
            },
            mismatch(Type::Bool, Type::Number),
        ),
    ] {
        let mut function = user(0, "bad", &[Type::Number], Type::Void);
        function
            .body
            .push(compound(target, BinaryOp::Multiply, value, span));
        assert_eq!(
            compile(&TypedProgram {
                functions: vec![function],
                host_functions: vec![]
            }),
            Err(expected)
        );
    }

    for ty in [Type::Bool, Type::String, Type::NumberArray] {
        let mut function = user(0, "bad", &[ty], Type::Void);
        function.body.push(compound(
            identifier(0, "x", ty),
            BinaryOp::Add,
            number(),
            span,
        ));
        assert_eq!(
            compile(&TypedProgram {
                functions: vec![function],
                host_functions: vec![]
            }),
            Err(mismatch(Type::Number, ty))
        );
    }
    for target in [
        identifier(0, "later", Type::Number),
        element(identifier(0, "later", Type::NumberArray), number()),
    ] {
        let ty = if matches!(target.kind, TypedExprKind::Index { .. }) {
            Type::NumberArray
        } else {
            Type::Number
        };
        let value = if ty == Type::NumberArray {
            TypedExpr {
                kind: TypedExprKind::Array(vec![]),
                ty: ty.into(),
            }
        } else {
            number()
        };
        let program = local_program(vec![
            compound(target, BinaryOp::Add, number(), span),
            declaration(0, ty, value, span),
        ]);
        assert_eq!(
            compile(&program),
            Err(CodegenError::UninitializedVariable {
                function: "bad".into(),
                id: VariableId(0),
                span,
            })
        );
    }
}

#[test]
fn checked_boundary_rejects_hidden_skipped_and_unreachable_compound_rhs_errors() {
    let span = Span::new(12, 34);
    let bad = || {
        compound(
            identifier(0, "n", Type::Number),
            BinaryOp::Remainder,
            number_literal("invalid"),
            span,
        )
    };
    for body in [
        vec![bad()],
        vec![returned(None, Span::new(0, 7)), bad()],
        vec![conditional(boolean(), vec![], Some(vec![bad()]), span)],
        vec![while_loop(boolean(), vec![bad()], span)],
    ] {
        let mut hidden = user(1, "bad", &[Type::Number], Type::Void);
        hidden.body = body;
        let program = TypedProgram {
            functions: vec![user(0, "visible", &[], Type::Void), hidden],
            host_functions: vec![],
        };
        for exports in [
            ExportOptions::All,
            ExportOptions::None,
            ExportOptions::Only(vec!["visible".into()]),
        ] {
            assert_eq!(
                compile_with_exports(&program, &exports),
                Err(CodegenError::InvalidNumberLiteral {
                    function: "bad".into(),
                    literal: "invalid".into(),
                    span,
                })
            );
        }
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_compound_preserves_checked_target_old_value_and_once_only_effects_across_growth() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "index",
            "env",
            "index",
            vec![Type::NumberArray],
            Type::Number,
        )
        .unwrap();
    compiler
        .register_host_function("rhs", "env", "rhs", vec![Type::NumberArray], Type::Number)
        .unwrap();
    let bytes = compiler
        .compile_source("fn update(a: number[]) -> void { a[index(a)] += rhs(a); }")
        .unwrap();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        let e, events = [], selected = 0;
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: {
            index(a) {
                events.push('index');
                const v = new DataView(e.memory.buffer);
                v.setFloat64(v.getUint32(a, true), 12, true);
                return selected;
            },
            rhs(a) {
                events.push('rhs');
                e.memory.grow(1);
                const v = new DataView(e.memory.buffer), p = v.getUint32(a, true);
                v.setFloat64(p, 99, true);
                v.setFloat64(p + 8, 200, true);
                selected = 1;
                return 3;
            }
        }});
        e = instance.exports;
        const a = e.__alloc(24);
        let v = new DataView(e.memory.buffer);
        v.setUint32(a, a + 8, true); v.setUint32(a + 4, 2, true);
        v.setFloat64(a + 8, 10, true); v.setFloat64(a + 16, 20, true);
        e.update(a);
        v = new DataView(e.memory.buffer);
        assert.deepEqual(events, ['index', 'rhs']);
        assert.equal(v.getFloat64(a + 8, true), 15);
        assert.equal(v.getFloat64(a + 16, true), 200);
        for (const bad of [2, -1, 0.5, NaN, Infinity]) {
            events = []; selected = bad;
            assert.throws(() => e.update(a), WebAssembly.RuntimeError);
            assert.deepEqual(events, ['index']);
        }
        events = [];
        assert.throws(() => e.update(0), WebAssembly.RuntimeError);
        assert.deepEqual(events, []);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_all_compound_operators_use_numeric_and_exact_remainder_semantics() {
    let mut source = String::from(
        "fn rhs(a: number[]) -> number {
            if len(a) == 1 { return a[0]; } else { return 0; }
        }
        fn stack(n: number) -> number { n += rhs([n]); return n; }",
    );
    for (index, (_, token)) in OPERATORS.into_iter().enumerate() {
        source.push_str(&format!(
            "fn scalar{index}(a: number, b: number) -> number {{ a {token} b; return a; }}
             fn array{index}(a: number[], b: number) -> number {{ a[0] {token} b; return a[0]; }}"
        ));
    }
    let bytes = Compiler::new().compile_source(&source).unwrap();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports;
        assert.equal(e.stack(5), 10);
        const a = e.__alloc(16), v = new DataView(e.memory.buffer);
        v.setUint32(a, a + 8, true); v.setUint32(a + 4, 1, true);
        const ops = [(a,b) => a+b, (a,b) => a-b, (a,b) => a*b, (a,b) => a/b, (a,b) => a%b];
        const cases = [[11, 3], [-11, 3], [-0, 2], [Infinity, 2], [2, Infinity],
            [NaN, 1], [1, 0], [Number.MAX_VALUE, Number.MIN_VALUE],
            [Number.MIN_VALUE, Number.MIN_VALUE * 2]];
        for (let i = 0; i < 5; i++) for (const [x,y] of cases) {
            const expected = ops[i](x,y);
            assert.ok(Object.is(e['scalar' + i](x,y), expected));
            v.setFloat64(a + 8, x, true);
            assert.ok(Object.is(e['array' + i](a,y), expected));
        }
    "#,
    );
}
