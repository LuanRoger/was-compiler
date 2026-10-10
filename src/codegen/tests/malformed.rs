use super::*;

#[test]
fn void_parameters_are_rejected_for_hosts_and_users() {
    for is_host in [false, true] {
        let program = if is_host {
            TypedProgram {
                functions: vec![],
                host_functions: vec![host(0, "bad", &[Type::Number, Type::Void], Type::Void)],
            }
        } else {
            TypedProgram {
                functions: vec![user(0, "bad", &[Type::Number, Type::Void], Type::Void)],
                host_functions: vec![],
            }
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::VoidParameter {
                function: "bad".to_owned(),
                index: 1,
            })
        );
    }
}

#[test]
fn empty_result_functions_are_rejected_without_placeholder_bodies() {
    for return_type in [Type::Number, Type::Bool, Type::String, Type::NumberArray] {
        let program = TypedProgram {
            functions: vec![user(0, "bad", &[], return_type)],
            host_functions: vec![],
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::MissingReturnValue {
                function: "bad".to_owned(),
                return_type,
            })
        );
    }
}

#[test]
fn malformed_memory_expression_statements_keep_their_span_even_after_return() {
    for after_return in [false, true] {
        for (expected, value) in [
            (
                Type::String,
                TypedExpr {
                    kind: TypedExprKind::String("bad".to_owned()),
                    ty: Type::Number.into(),
                },
            ),
            (
                Type::NumberArray,
                TypedExpr {
                    kind: TypedExprKind::Array(vec![number()]),
                    ty: Type::Number.into(),
                },
            ),
        ] {
            let kind = TypedStatementKind::Expression(value);
            let mut function = user(0, "bad", &[], Type::Void);

            if after_return {
                function.body.push(TypedStatement {
                    kind: TypedStatementKind::Return(None),
                    span: Span::new(0, 7),
                });
            }

            function.body.push(TypedStatement {
                kind,
                span: Span::new(12, 34),
            });

            let program = TypedProgram {
                functions: vec![function],
                host_functions: vec![],
            };

            assert_eq!(
                compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected,
                    found: Type::Number,
                    span: Span::new(12, 34),
                })
            );
        }
    }
}

#[test]
fn arithmetic_requires_numeric_roots_and_operands_even_in_nested_typed_ast() {
    let span = Span::new(12, 34);
    let mut roots = vec![unary(UnaryOp::Negate, number())];

    for op in [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
        BinaryOp::Remainder,
    ] {
        roots.push(binary(number(), op, number()));
    }

    for mut value in roots {
        value.ty = Type::Bool.into();

        assert_eq!(
            compile(&return_program(Some(value), Type::Bool, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            }),
        );
    }

    for found in [Type::Bool, Type::String, Type::NumberArray, Type::Void] {
        let mut operand = number();

        operand.ty = found.into();

        let mut expressions = vec![unary(UnaryOp::Negate, operand.clone())];

        for op in [
            BinaryOp::Add,
            BinaryOp::Subtract,
            BinaryOp::Multiply,
            BinaryOp::Divide,
            BinaryOp::Remainder,
        ] {
            expressions.push(binary(operand.clone(), op, number()));
            expressions.push(binary(number(), op, operand.clone()));
        }

        for expression in expressions {
            for value in [
                expression.clone(),
                binary(expression.clone(), BinaryOp::Add, number()),
                binary(number(), BinaryOp::Divide, expression.clone()),
                unary(UnaryOp::Negate, expression),
            ] {
                assert_eq!(
                    compile(&return_program(Some(value), Type::Number, span)),
                    Err(CodegenError::TypeMismatch {
                        function: "bad".to_owned(),
                        expected: Type::Number,
                        found,
                        span,
                    }),
                );
            }
        }
    }

    let mut nested_root = binary(number(), BinaryOp::Multiply, number());

    nested_root.ty = Type::Bool.into();

    for value in [
        unary(UnaryOp::Negate, nested_root.clone()),
        binary(nested_root.clone(), BinaryOp::Subtract, number()),
        binary(number(), BinaryOp::Divide, nested_root),
    ] {
        assert_eq!(
            compile(&return_program(Some(value), Type::Number, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            }),
        );
    }
}

#[test]
fn nested_numeric_literal_errors_preserve_the_enclosing_statement_span() {
    let span = Span::new(12, 34);
    let overflow = "9".repeat(400);

    for literal in ["-1", "NaN", "1e3", "1.", overflow.as_str()] {
        for value in [
            unary(UnaryOp::Negate, number_literal(literal)),
            binary(number_literal(literal), BinaryOp::Remainder, number()),
            binary(number(), BinaryOp::Remainder, number_literal(literal)),
            binary(
                unary(UnaryOp::Negate, number_literal(literal)),
                BinaryOp::Multiply,
                number(),
            ),
            binary(
                number(),
                BinaryOp::Divide,
                binary(number(), BinaryOp::Add, number_literal(literal)),
            ),
        ] {
            let expected = if literal == overflow {
                CodegenError::NumberLiteralOutOfRange {
                    function: "bad".to_owned(),
                    literal: literal.to_owned(),
                    span,
                }
            } else {
                CodegenError::InvalidNumberLiteral {
                    function: "bad".to_owned(),
                    literal: literal.to_owned(),
                    span,
                }
            };

            assert_eq!(
                compile(&return_program(Some(value), Type::Number, span)),
                Err(expected),
                "literal {literal:?}",
            );
        }
    }

    for expression in [
        format!("-({overflow})"),
        format!("{overflow} + 1"),
        format!("1 / (2 + {overflow})"),
        format!("{overflow} % 2"),
        format!("1 % {overflow}"),
    ] {
        let source = format!("fn bad() -> number {{ return {expression}; }}");
        let compiler = Compiler::new();
        let program = compiler.analyze(&source).expect("analyze nested overflow");

        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::NumberLiteralOutOfRange {
                function: "bad".to_owned(),
                literal: overflow.clone(),
                span: program.functions[0].body[0].span,
            }),
        );
    }
}

#[test]
fn remainder_checks_root_types_before_operands_and_operand_types_before_literals() {
    let span = Span::new(12, 34);

    for ty in [Type::Number, Type::Bool] {
        let mut operand = boolean();

        if ty == Type::Bool {
            operand.ty = Type::String.into();
        }

        let mut value = binary(operand, BinaryOp::Remainder, number_literal("NaN"));

        value.ty = ty.into();

        assert_eq!(
            compile(&return_program(Some(value), ty, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            }),
            "root type {ty:?}",
        );
    }
}

#[test]
fn nested_invalid_expressions_and_literals_propagate_in_left_to_right_order() {
    let span = Span::new(12, 34);
    let cases = vec![
        (
            TypedExpr {
                kind: TypedExprKind::String("bad".to_owned()),
                ty: Type::Number.into(),
            },
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::String,
                found: Type::Number,
                span,
            },
        ),
        (
            binary(number(), BinaryOp::Remainder, number_literal("1e3")),
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "1e3".to_owned(),
                span,
            },
        ),
        (
            TypedExpr {
                kind: TypedExprKind::Identifier {
                    id: VariableId(99),
                    name: "x".to_owned(),
                },
                ty: Type::Number.into(),
            },
            CodegenError::UnknownVariable {
                function: "bad".to_owned(),
                id: VariableId(99),
                span,
            },
        ),
        (
            TypedExpr {
                kind: TypedExprKind::Call {
                    func: FunctionRef::Host(HostFunctionId(1)),
                    name: "numeric".to_owned(),
                    args: vec![],
                },
                ty: Type::Number.into(),
            },
            CodegenError::UnknownFunctionReference {
                function: "bad".to_owned(),
                reference: FunctionRef::Host(HostFunctionId(1)),
                span,
            },
        ),
    ];

    for (expression, expected) in cases {
        for value in [
            unary(UnaryOp::Negate, expression.clone()),
            binary(expression.clone(), BinaryOp::Remainder, number()),
            binary(number(), BinaryOp::Remainder, expression.clone()),
            binary(expression.clone(), BinaryOp::Add, number()),
            binary(number(), BinaryOp::Subtract, expression.clone()),
            binary(
                binary(expression, BinaryOp::Multiply, number()),
                BinaryOp::Divide,
                number_literal("NaN"),
            ),
        ] {
            let mut program = return_program(Some(value), Type::Number, span);

            program
                .host_functions
                .push(host(0, "numeric", &[], Type::Number));
            program.functions[0].params.push(TypedParam {
                id: VariableId(0),
                name: "x".to_owned(),
                ty: Type::Number,
                span: Span::new(0, 1),
            });

            assert_eq!(
                compile(&program).expect_err("reject nested expression"),
                expected,
            );
        }
    }
}

#[test]
fn source_arithmetic_with_wrong_types_is_still_rejected_by_semantic_analysis() {
    for expression in [
        "-true",
        "1 + true",
        "true - 1",
        "1 * false",
        "false / 2",
        "1 % true",
        "false % 2",
        "[1] % 2",
        "1 % \"text\"",
        "1 + (2 * true)",
        "1 + \"text\"",
        "[1] / 2",
    ] {
        let source = format!("fn bad() -> number {{ return {expression}; }}");

        assert!(
            matches!(
                Compiler::new().compile_source(&source),
                Err(CompileError::Semantic(_))
            ),
            "expression {expression}",
        );
    }
}

#[test]
fn malformed_typed_numeric_literals_are_rejected_with_the_statement_span() {
    let span = Span::new(12, 34);

    for literal in [
        "", "+1", "-1", "-0", "1e3", "1E3", "1e-3", " 1", "1 ", "\t1", "1\n", "NaN", "nan", "inf",
        "Infinity", "-inf", ".25", "1.", ".", "1.2.3", "1_000", "0x10", "１２", "1\0",
    ] {
        let program = return_program(Some(number_literal(literal)), Type::Number, span);

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: literal.to_owned(),
                span,
            }),
            "literal {literal:?}",
        );
    }
}

#[test]
fn overflowing_numeric_literals_are_rejected_in_source_and_typed_ast() {
    let literal = "9".repeat(400);
    let span = Span::new(12, 34);
    let program = return_program(Some(number_literal(&literal)), Type::Number, span);

    assert_eq!(
        compile(&program),
        Err(CodegenError::NumberLiteralOutOfRange {
            function: "bad".to_owned(),
            literal: literal.clone(),
            span,
        }),
    );

    let source = format!("fn bad() -> number {{ return {literal}; }}");
    let compiler = Compiler::new();
    let program = compiler
        .analyze(&source)
        .expect("analyze overflowing literal");

    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::NumberLiteralOutOfRange {
            function: "bad".to_owned(),
            literal,
            span: program.functions[0].body[0].span,
        }),
    );
}

#[test]
fn malformed_memory_expression_kinds_are_rejected_with_the_statement_span() {
    let expressions = vec![
        (Type::String, TypedExprKind::String("text".to_owned())),
        (Type::NumberArray, TypedExprKind::Array(vec![number()])),
        (
            Type::NumberArray,
            TypedExprKind::Index {
                expr: Box::new(boolean()),
                index: Box::new(number_literal("0")),
            },
        ),
    ];
    let span = Span::new(12, 34);

    for (expected, kind) in expressions {
        let is_index = matches!(&kind, TypedExprKind::Index { .. });
        let value = TypedExpr {
            kind,
            ty: Type::Number.into(),
        };

        for is_statement in [false, true] {
            let mut program = if is_statement {
                local_program(vec![TypedStatement {
                    kind: TypedStatementKind::Expression(value.clone()),
                    span,
                }])
            } else {
                return_program(Some(value.clone()), Type::Number, span)
            };
            program
                .host_functions
                .push(host(0, "numeric", &[], Type::Number));

            assert_eq!(
                compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected,
                    found: if is_index { Type::Bool } else { Type::Number },
                    span,
                }),
            );
        }
    }
}

#[test]
fn source_builtin_len_propagates_overflowing_array_element_literals() {
    let compiler = Compiler::new();
    let literal = "9".repeat(400);
    let source = format!("fn bad() -> number {{ return len([{literal}]); }}");
    let program = compiler
        .analyze(&source)
        .expect("analyze builtin return expression");

    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::NumberLiteralOutOfRange {
            function: "bad".to_owned(),
            literal,
            span: program.functions[0].body[0].span,
        }),
    );
}

#[test]
fn invalid_unreachable_returns_are_not_skipped() {
    let span = Span::new(40, 60);
    let overflow = "9".repeat(400);
    let cases = vec![
        (
            Some(number_literal("-1")),
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "-1".to_owned(),
                span,
            },
        ),
        (
            Some(number_literal(&overflow)),
            CodegenError::NumberLiteralOutOfRange {
                function: "bad".to_owned(),
                literal: overflow,
                span,
            },
        ),
        (
            Some(boolean()),
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            Some(binary(number(), BinaryOp::Remainder, number_literal("NaN"))),
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            },
        ),
        (
            None,
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Void,
                span,
            },
        ),
    ];

    for (value, expected) in cases {
        let mut program = return_program(Some(number()), Type::Number, Span::new(12, 34));

        program.functions[0].body.push(TypedStatement {
            kind: TypedStatementKind::Return(value),
            span,
        });

        assert_eq!(compile(&program), Err(expected));
    }
}

#[test]
fn malformed_return_types_and_numeric_expression_types_are_checked() {
    let span = Span::new(12, 34);

    for return_type in [Type::Number, Type::Bool] {
        assert_eq!(
            compile(&return_program(None, return_type, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: return_type,
                found: Type::Void,
                span,
            }),
        );
    }

    for (value, return_type, expected, found) in [
        (boolean(), Type::Number, Type::Number, Type::Bool),
        (number(), Type::Bool, Type::Bool, Type::Number),
        (
            TypedExpr {
                kind: TypedExprKind::Number("42".to_owned()),
                ty: Type::Bool.into(),
            },
            Type::Bool,
            Type::Number,
            Type::Bool,
        ),
    ] {
        assert_eq!(
            compile(&return_program(Some(value), return_type, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected,
                found,
                span,
            }),
        );
    }
}

#[test]
fn void_returns_reject_any_value_including_void_typed_expressions() {
    let span = Span::new(12, 34);

    for value in [
        number(),
        boolean(),
        number_literal("NaN"),
        TypedExpr {
            kind: TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(0)),
                name: "noop".to_owned(),
                args: vec![],
            },
            ty: Type::Void.into(),
        },
    ] {
        let mut program = return_program(Some(value), Type::Void, span);

        program
            .host_functions
            .push(host(0, "noop", &[], Type::Void));

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnexpectedReturnValue {
                function: "bad".to_owned(),
                span,
            }),
        );
    }
}

#[test]
fn invalid_user_ids_are_rejected() {
    for ids in [vec![1], vec![0, 0], vec![0, 2], vec![1, 0]] {
        let expected = ids
            .iter()
            .enumerate()
            .find(|(expected, actual)| *expected != **actual)
            .unwrap()
            .0;
        let actual = ids[expected];
        let program = TypedProgram {
            functions: ids
                .iter()
                .enumerate()
                .map(|(position, id)| user(*id, &format!("u{position}"), &[], Type::Void))
                .collect(),
            host_functions: vec![],
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidFunctionId {
                function: format!("u{expected}"),
                expected,
                actual,
            })
        );
    }
}

#[test]
fn invalid_host_ids_are_rejected() {
    for ids in [vec![1], vec![0, 0], vec![0, 2], vec![1, 0]] {
        let expected = ids
            .iter()
            .enumerate()
            .find(|(expected, actual)| *expected != **actual)
            .unwrap()
            .0;
        let actual = ids[expected];
        let program = TypedProgram {
            functions: vec![],
            host_functions: ids
                .iter()
                .enumerate()
                .map(|(position, id)| host(*id, &format!("h{position}"), &[], Type::Void))
                .collect(),
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidHostFunctionId {
                function: format!("h{expected}"),
                expected,
                actual,
            })
        );
    }
}

#[test]
fn duplicate_export_names_are_rejected() {
    let program = TypedProgram {
        functions: vec![
            user(0, "duplicate", &[], Type::Void),
            user(1, "duplicate", &[], Type::Void),
        ],
        host_functions: vec![],
    };

    assert_eq!(
        compile(&program),
        Err(CodegenError::DuplicateExportName {
            name: "duplicate".to_owned()
        })
    );
}

#[test]
fn errors_from_later_functions_propagate_without_partial_module_output() {
    let program = TypedProgram {
        functions: vec![
            user(0, "valid", &[], Type::Void),
            user(1, "bad", &[], Type::Number),
        ],
        host_functions: vec![],
    };

    let error = compile(&program).expect_err("reject second function");

    assert_eq!(
        error,
        CodegenError::MissingReturnValue {
            function: "bad".to_owned(),
            return_type: Type::Number
        }
    );
    assert!(error.to_string().contains("bad"));
    assert!(error.to_string().contains("Number"));
}
