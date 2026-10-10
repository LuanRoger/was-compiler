use super::*;
use crate::typed_ast::{ExprResult, TypedValueBlock};

#[test]
fn source_numeric_literals_emit_f64_const_return_and_end() {
    for (literal, expected) in [
        ("42", 42.0),
        ("0.25", 0.25),
        ("0", 0.0),
        ("0.0", 0.0),
        ("00042.00", 42.0),
        ("9007199254740993", 9007199254740992.0),
        ("100000000000000000000", 1e20),
    ] {
        let source = format!("fn value() -> number {{ return {literal}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile numeric return");
        let module = decode(&bytes);

        assert_eq!(module.signatures, vec![(vec![], vec![ValType::F64])]);
        assert_eq!(module.functions, vec![0]);
        assert_eq!(
            module.exports,
            vec![("value".to_owned(), ExternalKind::Func, 0)],
        );
        assert_eq!(
            module.bodies,
            vec![vec![
                DecodedInstruction::F64Const(expected),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ]],
            "literal {literal}",
        );
        assert_eq!(
            Compiler::new().compile_source(&source).unwrap(),
            bytes,
            "deterministic literal {literal}",
        );
    }
}

#[test]
fn source_arithmetic_emits_operands_before_each_opcode_or_helper_call() {
    use DecodedInstruction::*;

    for (expression, instruction) in [
        ("8 + 2", F64Add),
        ("8 - 2", F64Sub),
        ("8 * 2", F64Mul),
        ("8 / 2", F64Div),
        ("8 % 2", Call(1)),
    ] {
        let source = format!("fn value() -> number {{ return {expression}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile arithmetic return");
        let module = decode(&bytes);

        let remainder = expression.contains('%');
        let mut signatures = vec![(vec![], vec![ValType::F64])];
        let mut functions = vec![0];

        if remainder {
            signatures.push((vec![ValType::F64, ValType::F64], vec![ValType::F64]));
            functions.push(1);
        }

        assert_eq!(module.signatures, signatures);
        assert_eq!(module.functions, functions);
        assert_eq!(module.bodies.len(), if remainder { 2 } else { 1 });
        assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
        assert!(module.imports.is_empty());
        assert_eq!(
            module.exports,
            vec![("value".to_owned(), ExternalKind::Func, 0)],
        );
        assert_eq!(
            module.bodies[0],
            vec![F64Const(8.0), F64Const(2.0), instruction, Return, End],
            "expression {expression}",
        );
    }
}

#[test]
fn source_unary_negation_emits_f64_neg_without_folding() {
    use DecodedInstruction::*;

    for (expression, instructions) in [
        ("-42", vec![F64Const(42.0), F64Neg]),
        ("-0", vec![F64Const(0.0), F64Neg]),
        ("-(-2)", vec![F64Const(2.0), F64Neg, F64Neg]),
        (
            "-(1 + 2)",
            vec![F64Const(1.0), F64Const(2.0), F64Add, F64Neg],
        ),
        (
            "-(5 % 2)",
            vec![F64Const(5.0), F64Const(2.0), Call(1), F64Neg],
        ),
    ] {
        let source = format!("fn value() -> number {{ return {expression}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile unary negation");
        let mut expected = instructions;

        expected.extend([Return, End]);

        let module = decode(&bytes);

        assert_eq!(module.bodies[0], expected, "{expression}");
        assert_eq!(
            module.bodies.len(),
            if expression.contains('%') { 2 } else { 1 }
        );
    }
}

#[test]
fn nested_arithmetic_preserves_precedence_associativity_and_operand_order() {
    use DecodedInstruction::*;

    for (expression, instructions) in [
        (
            "1 + 2 * 3",
            vec![F64Const(1.0), F64Const(2.0), F64Const(3.0), F64Mul, F64Add],
        ),
        (
            "(1 + 2) * 3",
            vec![F64Const(1.0), F64Const(2.0), F64Add, F64Const(3.0), F64Mul],
        ),
        (
            "10 - 3 - 2",
            vec![F64Const(10.0), F64Const(3.0), F64Sub, F64Const(2.0), F64Sub],
        ),
        (
            "20 / 2 / 5",
            vec![F64Const(20.0), F64Const(2.0), F64Div, F64Const(5.0), F64Div],
        ),
        (
            "20 / (2 + 3)",
            vec![F64Const(20.0), F64Const(2.0), F64Const(3.0), F64Add, F64Div],
        ),
        (
            "1 / (2 / 4)",
            vec![F64Const(1.0), F64Const(2.0), F64Const(4.0), F64Div, F64Div],
        ),
        (
            "10 - (3 - 2)",
            vec![F64Const(10.0), F64Const(3.0), F64Const(2.0), F64Sub, F64Sub],
        ),
        (
            "20 % 6 % 3",
            vec![
                F64Const(20.0),
                F64Const(6.0),
                Call(1),
                F64Const(3.0),
                Call(1),
            ],
        ),
        (
            "20 % (6 % 3)",
            vec![
                F64Const(20.0),
                F64Const(6.0),
                F64Const(3.0),
                Call(1),
                Call(1),
            ],
        ),
        (
            "1 + 8 % 3 * 2",
            vec![
                F64Const(1.0),
                F64Const(8.0),
                F64Const(3.0),
                Call(1),
                F64Const(2.0),
                F64Mul,
                F64Add,
            ],
        ),
        (
            "(8 - 2) / (1 + 2)",
            vec![
                F64Const(8.0),
                F64Const(2.0),
                F64Sub,
                F64Const(1.0),
                F64Const(2.0),
                F64Add,
                F64Div,
            ],
        ),
    ] {
        let source = format!("fn value() -> number {{ return {expression}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile nested arithmetic");
        let mut expected = instructions;

        expected.extend([Return, End]);

        let module = decode(&bytes);

        assert_eq!(module.bodies[0], expected, "{expression}");
        assert_eq!(
            module.bodies.len(),
            if expression.contains('%') { 2 } else { 1 }
        );
    }
}

#[test]
fn arithmetic_preserves_import_indices_and_is_not_folded_at_any_optimization_level() {
    use DecodedInstruction::*;

    let mut expected_bytes = None;

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

        let source = "fn value(n: number, flag: bool) -> number { return -(1 + 2 * 3); }
                      fn quotient() -> number { return 1 / (2 / 4); }";
        let program = compiler.analyze(source).expect("analyze arithmetic");
        let original = program.clone();
        let bytes = compiler.compile(&program).expect("compile arithmetic");
        let module = decode(&bytes);

        assert_eq!(program, original);
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
        assert_eq!(
            module.signatures,
            vec![
                (vec![], vec![ValType::F64]),
                (vec![], vec![ValType::I32]),
                (vec![ValType::F64, ValType::I32], vec![ValType::F64]),
            ],
        );
        assert_eq!(
            module.imports,
            vec![
                ("env".to_owned(), "numeric".to_owned(), 0),
                ("provider".to_owned(), "ready".to_owned(), 1),
            ],
        );
        assert_eq!(module.functions, vec![2, 0]);
        assert_eq!(
            module.exports,
            vec![
                ("value".to_owned(), ExternalKind::Func, 2),
                ("quotient".to_owned(), ExternalKind::Func, 3),
            ],
        );
        assert_eq!(
            module.bodies,
            vec![
                vec![
                    F64Const(1.0),
                    F64Const(2.0),
                    F64Const(3.0),
                    F64Mul,
                    F64Add,
                    F64Neg,
                    Return,
                    End,
                ],
                vec![
                    F64Const(1.0),
                    F64Const(2.0),
                    F64Const(4.0),
                    F64Div,
                    F64Div,
                    Return,
                    End
                ],
            ],
            "optimization level {level:?}",
        );

        if let Some(expected) = &expected_bytes {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            expected_bytes = Some(bytes);
        }
    }
}

#[test]
fn source_returns_emit_remainder_helper_calls_after_literal_or_parameter_operands() {
    use DecodedInstruction::*;

    for (expression, operands) in [
        ("1 % 2", vec![F64Const(1.0), F64Const(2.0)]),
        ("flag_number % 2", vec![LocalGet(0), F64Const(2.0)]),
    ] {
        let source = format!("fn value(flag_number: number) -> number {{ return {expression}; }}");
        let compiler = Compiler::new();
        let program = compiler.analyze(&source).expect("analyze remainder");
        let original = program.clone();
        let bytes = compiler.compile(&program).expect("compile remainder");
        let module = decode(&bytes);
        let mut expected = operands;

        expected.extend([Call(1), Return, End]);

        assert_eq!(module.bodies[0], expected, "expression {expression}");
        assert_eq!(module.bodies.len(), 2);
        assert_eq!(
            module.exports,
            vec![("value".into(), ExternalKind::Func, 0)]
        );
        assert_eq!(compiler.compile_source(&source).unwrap(), bytes);
        assert_eq!(program, original);
    }
}

#[test]
fn valid_underflowing_literals_may_round_to_zero() {
    let literal = format!("0.{}1", "0".repeat(400));
    let source = format!("fn value() -> number {{ return {literal}; }}");
    let program = return_program(
        Some(number_literal(&literal)),
        Type::Number,
        Span::new(12, 34),
    );

    for bytes in [
        compile(&program).expect("compile underflowing typed literal"),
        Compiler::new()
            .compile_source(&source)
            .expect("compile underflowing source literal"),
    ] {
        assert_eq!(
            decode(&bytes).bodies,
            vec![vec![
                DecodedInstruction::F64Const(0.0),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ]],
        );
    }
}

#[test]
fn numeric_comparisons_emit_f64_operands_and_bool_results_at_all_optimization_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        for (operator, instruction) in [
            ("==", DecodedInstruction::F64Eq),
            ("!=", DecodedInstruction::F64Ne),
            ("<", DecodedInstruction::F64Lt),
            ("<=", DecodedInstruction::F64Le),
            (">", DecodedInstruction::F64Gt),
            (">=", DecodedInstruction::F64Ge),
        ] {
            let source = format!(
                "fn compare(a: number, b: number) -> bool {{ return a + 1 {operator} b * 2; }}"
            );
            let module = decode(
                &compiler
                    .compile_source(&source)
                    .expect("compile comparison"),
            );

            assert_eq!(
                module.signatures,
                vec![(vec![ValType::F64, ValType::F64], vec![ValType::I32])]
            );
            assert_eq!(
                module.bodies[0],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::LocalGet(1),
                    DecodedInstruction::F64Const(2.0),
                    DecodedInstruction::F64Mul,
                    instruction,
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                "operator {operator}, level {level:?}"
            );
        }
    }
}

#[test]
fn bool_equality_normalizes_both_operands_and_not_emits_eqz() {
    let bytes = Compiler::new()
        .compile_source(
            "fn equal(a: bool, b: bool) -> bool { return a == b; }
         fn unequal(a: bool, b: bool) -> bool { return a != b; }
         fn invert(a: bool) -> bool { return !a; }",
        )
        .expect("compile bool operators");
    let module = decode(&bytes);

    for (index, instruction) in [DecodedInstruction::I32Eq, DecodedInstruction::I32Ne]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            module.bodies[index],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::LocalGet(1),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                instruction,
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ]
        );
    }

    assert_eq!(
        module.bodies[2],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn logical_operators_guard_rhs_calls_inside_i32_result_ifs() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("rhs", "env", "rhs", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn both(a: bool) -> bool { return a && rhs(); }
         fn either(a: bool) -> bool { return a || rhs(); }
         fn nested(a: bool, b: bool) -> bool { return a && (b || rhs()); }",
        )
        .expect("compile short circuit");
    let module = decode(&bytes);

    for (index, rhs_in_then, constant) in [(0, true, 0), (1, false, 1)] {
        let body = &module.bodies[index];
        let if_index = body
            .iter()
            .position(|op| *op == DecodedInstruction::If(BlockType::Type(ValType::I32)))
            .unwrap();
        let else_index = body
            .iter()
            .position(|op| *op == DecodedInstruction::Else)
            .unwrap();
        let call_index = body
            .iter()
            .position(|op| *op == DecodedInstruction::Call(0))
            .unwrap();

        assert!(if_index < call_index);
        assert_eq!(call_index < else_index, rhs_in_then);
        assert!(body.contains(&DecodedInstruction::I32Const(constant)));
        assert_eq!(
            body.iter()
                .filter(|op| **op == DecodedInstruction::Call(0))
                .count(),
            1
        );
        assert_eq!(
            &body[call_index + 1..call_index + 3],
            &[DecodedInstruction::I32Eqz, DecodedInstruction::I32Eqz]
        );
        assert_eq!(
            &body[body.len() - 3..],
            &[
                DecodedInstruction::End,
                DecodedInstruction::Return,
                DecodedInstruction::End
            ]
        );
    }

    assert_eq!(
        module.bodies[2]
            .iter()
            .filter(|op| **op == DecodedInstruction::If(BlockType::Type(ValType::I32)))
            .count(),
        2
    );
    assert_eq!(
        module.bodies[2]
            .iter()
            .filter(|op| **op == DecodedInstruction::Call(0))
            .count(),
        1
    );
}

#[test]
fn logical_branches_preserve_live_equality_operands_and_call_arguments() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn compare(a: bool, b: bool, c: bool) -> bool { return a == (b && c); }
             fn invoke(a: bool, b: bool) -> number { return take(7, a || b); }
             fn take(value: number, flag: bool) -> number {
                 if flag { return value; } else { return -value; }
             }",
        )
        .expect("compile logical branches with live stack prefixes");
    let module = decode(&bytes);

    assert_eq!(
        &module.bodies[0][..5],
        &[
            LocalGet(0),
            I32Eqz,
            I32Eqz,
            LocalGet(1),
            If(BlockType::Type(ValType::I32))
        ]
    );
    assert_eq!(
        &module.bodies[1][..3],
        &[
            F64Const(7.0),
            LocalGet(0),
            If(BlockType::Type(ValType::I32))
        ]
    );
    assert_eq!(
        &module.bodies[1][module.bodies[1].len() - 3..],
        &[Call(2), Return, End]
    );
}

#[test]
fn forged_bool_operator_roots_and_operands_are_type_checked() {
    let span = Span::new(12, 34);
    let mut cases = vec![(bool_not(number()), Type::Bool, Type::Number)];
    let mut wrong_not_root = bool_not(boolean());

    wrong_not_root.ty = Type::Number.into();
    cases.push((wrong_not_root, Type::Bool, Type::Number));

    for op in [
        BinaryOp::Equal,
        BinaryOp::NotEqual,
        BinaryOp::LessThan,
        BinaryOp::LessThanOrEqual,
        BinaryOp::GreaterThan,
        BinaryOp::GreaterThanOrEqual,
        BinaryOp::And,
        BinaryOp::Or,
    ] {
        let logical = matches!(op, BinaryOp::And | BinaryOp::Or);
        let operand = if logical { boolean() } else { number() };
        let mut wrong_root = bool_binary(operand.clone(), op, operand);

        wrong_root.ty = Type::Number.into();
        cases.push((wrong_root, Type::Bool, Type::Number));

        if logical {
            cases.push((
                bool_binary(number(), op, boolean()),
                Type::Bool,
                Type::Number,
            ));
            cases.push((
                bool_binary(boolean(), op, number()),
                Type::Bool,
                Type::Number,
            ));
        } else {
            cases.push((
                bool_binary(number(), op, boolean()),
                Type::Number,
                Type::Bool,
            ));

            if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
                cases.push((
                    bool_binary(boolean(), op, number()),
                    Type::Bool,
                    Type::Number,
                ));
            } else {
                cases.push((
                    bool_binary(boolean(), op, number()),
                    Type::Number,
                    Type::Bool,
                ));
            }
        }
    }

    for (value, expected, found) in cases {
        assert_eq!(
            compile(&return_program(
                Some(value.clone()),
                value.ty.value_type().unwrap(),
                span
            )),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected,
                found,
                span,
            }),
            "expression {value:?}"
        );
    }
}

#[test]
fn equality_checks_parent_result_before_rejecting_void_left_operand() {
    let span = Span::new(12, 34);

    for op in [BinaryOp::Equal, BinaryOp::NotEqual] {
        let left = TypedExpr {
            kind: TypedExprKind::Block(TypedValueBlock {
                statements: vec![],
                tail: None,
                span,
            }),
            ty: ExprResult::Void,
        };
        let mut value = bool_binary(left, op, number());

        value.ty = ExprResult::Value(Type::Number);

        assert_eq!(
            compile(&local_program(vec![TypedStatement {
                kind: TypedStatementKind::Expression(value),
                span,
            }])),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Bool,
                found: Type::Number,
                span,
            }),
            "operator {op:?}"
        );
    }
}

#[test]
fn equality_with_correct_parent_result_retains_number_expected_void_left_error() {
    let span = Span::new(12, 34);
    let never = TypedExpr {
        kind: TypedExprKind::Block(TypedValueBlock {
            statements: vec![TypedStatement {
                kind: TypedStatementKind::Return(None),
                span,
            }],
            tail: None,
            span,
        }),
        ty: ExprResult::Never,
    };

    for op in [BinaryOp::Equal, BinaryOp::NotEqual] {
        for right in [number(), boolean(), never.clone()] {
            let left = TypedExpr {
                kind: TypedExprKind::Block(TypedValueBlock {
                    statements: vec![],
                    tail: None,
                    span,
                }),
                ty: ExprResult::Void,
            };
            let result = if right.ty == ExprResult::Never {
                ExprResult::Never
            } else {
                ExprResult::Value(Type::Bool)
            };
            let mut value = bool_binary(left, op, right);

            value.ty = result;

            assert_eq!(
                compile(&local_program(vec![TypedStatement {
                    kind: TypedStatementKind::Expression(value),
                    span,
                }])),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Number,
                    found: Type::Void,
                    span,
                }),
                "operator {op:?}, result {result:?}"
            );
        }
    }
}

#[test]
fn logical_rhs_is_validated_even_when_runtime_short_circuit_would_skip_it() {
    let span = Span::new(12, 34);

    for (op, left) in [
        (
            BinaryOp::And,
            TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool.into(),
            },
        ),
        (BinaryOp::Or, boolean()),
    ] {
        let right = bool_binary(number_literal("NaN"), BinaryOp::Equal, number());
        let value = bool_binary(left.clone(), op, right);

        assert_eq!(
            compile(&return_program(Some(value), Type::Bool, span)),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            })
        );
        assert_eq!(
            compile(&return_program(
                Some(bool_binary(left, op, number())),
                Type::Bool,
                span
            )),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Bool,
                found: Type::Number,
                span,
            })
        );
    }
}
