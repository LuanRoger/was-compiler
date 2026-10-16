use super::*;

#[test]
fn host_call_statements_drop_only_scalar_results_and_normalize_bool_returns() {
    let expressions = [Type::Number, Type::Bool, Type::Void].map(|ty| TypedExpr {
        kind: TypedExprKind::Call {
            func: FunctionRef::Host(HostFunctionId(0)),
            name: "host".to_owned(),
            args: vec![],
        },
        ty: ty.into(),
    });

    for expression in expressions {
        let expression_type = expression.ty.value_type().unwrap_or(Type::Void);
        let mut function = user(0, "bad", &[], Type::Void);

        function.body.push(TypedStatement {
            kind: TypedStatementKind::Expression(expression),
            span: Span::new(5, 9),
        });

        let program = TypedProgram {
            functions: vec![function],
            host_functions: vec![host(0, "host", &[], expression_type)],
        };

        let mut expected = vec![DecodedInstruction::Call(0)];

        if expression_type == Type::Bool {
            expected.extend([DecodedInstruction::I32Eqz, DecodedInstruction::I32Eqz]);
        }

        if expression_type != Type::Void {
            expected.push(DecodedInstruction::Drop);
        }

        expected.push(DecodedInstruction::End);

        assert_eq!(decode(&compile(&program).unwrap()).bodies, vec![expected]);
    }
}

#[test]
fn forward_nested_scalar_and_void_calls_preserve_import_indices_at_all_optimization_levels() {
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
            .register_host_function("seed", "env", "seed", vec![Type::Number], Type::Number)
            .unwrap();
        compiler
            .register_host_function("ready", "env", "ready", vec![Type::Bool], Type::Bool)
            .unwrap();
        compiler
            .register_host_function("print", "env", "log", vec![Type::Number], Type::Void)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn run(n: number, flag: bool) -> number {
                    let value: number = identity(seed(n) + 2);
                    let copy: bool = pass(ready(flag));
                    copy;
                    relay(value);
                    identity(value);
                    return identity(seed(value));
                }
                fn identity(n: number) -> number { return n; }
                fn pass(flag: bool) -> bool { return flag; }
                fn relay(n: number) -> void { print(n); }",
            )
            .expect("compile forward nested calls");
        let module = decode(&bytes);

        assert_eq!(
            module.exports,
            vec![
                ("run".to_owned(), ExternalKind::Func, 3),
                ("identity".to_owned(), ExternalKind::Func, 4),
                ("pass".to_owned(), ExternalKind::Func, 5),
                ("relay".to_owned(), ExternalKind::Func, 6),
            ],
        );
        assert_eq!(
            module.imports,
            vec![
                ("env".to_owned(), "seed".to_owned(), 0),
                ("env".to_owned(), "ready".to_owned(), 1),
                ("env".to_owned(), "log".to_owned(), 2),
            ],
        );
        assert_eq!(
            module.bodies,
            vec![
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(0),
                    DecodedInstruction::F64Const(2.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(4),
                    DecodedInstruction::LocalSet(2),
                    DecodedInstruction::LocalGet(1),
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::Call(1),
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::Call(5),
                    DecodedInstruction::LocalSet(3),
                    DecodedInstruction::LocalGet(3),
                    DecodedInstruction::Drop,
                    DecodedInstruction::LocalGet(2),
                    DecodedInstruction::Call(6),
                    DecodedInstruction::LocalGet(2),
                    DecodedInstruction::Call(4),
                    DecodedInstruction::Drop,
                    DecodedInstruction::LocalGet(2),
                    DecodedInstruction::Call(0),
                    DecodedInstruction::Call(4),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(2),
                    DecodedInstruction::End,
                ],
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes);
        }
    }
}

#[test]
fn call_ids_are_authoritative_even_when_names_match_other_users_or_hosts() {
    let span = Span::new(12, 34);
    let mut program = call_program(
        binary(
            call(
                FunctionRef::User(FunctionId(1)),
                "host_only",
                vec![],
                Type::Number,
            ),
            BinaryOp::Add,
            call(
                FunctionRef::Host(HostFunctionId(0)),
                "callee",
                vec![],
                Type::Number,
            ),
        ),
        &[],
        Type::Number,
        span,
    );

    program.host_functions[0].name = "host_only".to_owned();
    program.functions[0].body.push(TypedStatement {
        kind: TypedStatementKind::Expression(call(
            FunctionRef::User(FunctionId(1)),
            "not_registered_anywhere",
            vec![],
            Type::Number,
        )),
        span,
    });

    assert_eq!(
        decode(&compile(&program).unwrap()).bodies[0],
        vec![
            DecodedInstruction::Call(2),
            DecodedInstruction::Call(0),
            DecodedInstruction::F64Add,
            DecodedInstruction::Drop,
            DecodedInstruction::Call(2),
            DecodedInstruction::Drop,
            DecodedInstruction::End,
        ],
    );
}

#[test]
fn host_bool_arguments_are_normalized_immediately_between_nested_scalar_calls() {
    let mut compiler = Compiler::new();

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
            "fn bad(a: bool, n: number, b: bool) -> bool {
                return host_bool(user_bool(a), host_number(n), user_bool(b));
            }
            fn user_bool(flag: bool) -> bool { return flag; }",
        )
        .expect("compile mixed host arguments");

    assert_eq!(
        decode(&bytes).bodies[0],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::Call(3),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::LocalGet(1),
            DecodedInstruction::Call(1),
            DecodedInstruction::LocalGet(2),
            DecodedInstruction::Call(3),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::Call(0),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ],
    );
}

#[test]
fn self_and_mutual_calls_link_without_executing_unbounded_guest_recursion() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn recurse(n: number) -> number { return recurse(n); }
            fn left(flag: bool) -> bool { return right(flag); }
            fn right(flag: bool) -> bool { return left(flag); }
            fn spin() -> void { spin(); }",
        )
        .expect("compile recursive references");
    let module = decode(&bytes);

    assert_eq!(
        module.bodies,
        vec![
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(3),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(2),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![DecodedInstruction::Call(4), DecodedInstruction::End],
        ],
    );
}

#[test]
fn calls_reject_argument_counts_before_emitting_any_argument() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for args in [vec![], vec![number_literal("NaN"), number()]] {
            let found = args.len();
            let program = call_program(
                call(reference, "ignored", args, Type::Number),
                &[Type::Number],
                Type::Number,
                span,
            );

            assert_eq!(
                compile(&program),
                Err(CodegenError::ArgumentCountMismatch {
                    function: "bad".to_owned(),
                    reference,
                    expected: 1,
                    found,
                    span,
                }),
            );
        }
    }

    let program = return_program(
        Some(call(
            FunctionRef::User(FunctionId(0)),
            "bad",
            vec![number()],
            Type::Number,
        )),
        Type::Number,
        span,
    );

    assert_eq!(
        compile(&program),
        Err(CodegenError::ArgumentCountMismatch {
            function: "bad".to_owned(),
            reference: FunctionRef::User(FunctionId(0)),
            expected: 0,
            found: 1,
            span,
        }),
    );
}

#[test]
fn calls_check_real_return_types_including_void_before_argument_errors() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for expected in [Type::Number, Type::Bool, Type::Void] {
            for found in [Type::Number, Type::Bool, Type::Void] {
                if expected == found {
                    continue;
                }

                let program = call_program(
                    call(reference, "ignored", vec![number_literal("NaN")], found),
                    &[Type::Number],
                    expected,
                    span,
                );

                assert_eq!(
                    compile(&program),
                    Err(CodegenError::TypeMismatch {
                        function: "bad".to_owned(),
                        expected,
                        found,
                        span,
                    }),
                );
            }
        }
    }

    let mut program = return_program(Some(number()), Type::Number, Span::new(0, 7));

    program.functions[0].body.push(TypedStatement {
        kind: TypedStatementKind::Expression(call(
            FunctionRef::User(FunctionId(0)),
            "bad",
            vec![],
            Type::Void,
        )),
        span,
    });

    assert_eq!(
        compile(&program),
        Err(CodegenError::TypeMismatch {
            function: "bad".to_owned(),
            expected: Type::Number,
            found: Type::Void,
            span,
        }),
    );
}

#[test]
fn calls_require_exact_argument_types_at_first_and_higher_positions() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for expected in [Type::Number, Type::Bool] {
            for found in [
                Type::Number,
                Type::Bool,
                Type::Void,
                Type::String,
                Type::Array(crate::ast::ArrayElementType::Number),
            ] {
                if expected == found {
                    continue;
                }

                for position in [0, 2] {
                    let valid = if expected == Type::Bool {
                        boolean()
                    } else {
                        number()
                    };
                    let mut args = vec![valid; 3];

                    args[position] = TypedExpr {
                        kind: TypedExprKind::Number("NaN".to_owned()),
                        ty: found.into(),
                    };

                    let program = call_program(
                        call(reference, "ignored", args, Type::Void),
                        &[expected; 3],
                        Type::Void,
                        span,
                    );

                    assert_eq!(
                        compile(&program),
                        Err(CodegenError::TypeMismatch {
                            function: "bad".to_owned(),
                            expected,
                            found,
                            span,
                        }),
                    );
                }
            }
        }
    }
}

#[test]
fn call_arguments_are_type_checked_and_emitted_left_to_right() {
    let span = Span::new(12, 34);
    let cases = vec![
        (
            vec![number_literal("NaN"), boolean()],
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            },
        ),
        (
            vec![boolean(), number_literal("NaN")],
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            vec![number(), number_literal("NaN")],
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            },
        ),
        (
            vec![identifier(99, "unknown", Type::Number), boolean()],
            CodegenError::UnknownVariable {
                function: "bad".to_owned(),
                id: VariableId(99),
                span,
            },
        ),
    ];

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for (args, expected) in &cases {
            let value = call(reference, "ignored", args.clone(), Type::Number);
            let statements = vec![
                TypedStatement {
                    kind: TypedStatementKind::Expression(value.clone()),
                    span,
                },
                TypedStatement {
                    kind: TypedStatementKind::Return(Some(value.clone())),
                    span,
                },
                declaration(7, Type::Number, value.clone(), span),
                assignment(identifier(0, "p0", Type::Number), value, span),
            ];

            for statement in statements {
                let mut program =
                    call_program(number(), &[Type::Number, Type::Number], Type::Number, span);

                program.functions[0].params = user(0, "bad", &[Type::Number], Type::Number).params;
                program.functions[0].return_type = Type::Number;
                program.functions[0].body = vec![statement];

                assert_eq!(
                    &compile(&program).expect_err("reject call argument"),
                    expected
                );
            }
        }
    }
}

#[test]
fn unknown_call_references_do_not_spill_between_host_and_user_tables_or_use_names() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(2)),
        FunctionRef::User(FunctionId(usize::MAX)),
        FunctionRef::Host(HostFunctionId(1)),
        FunctionRef::Host(HostFunctionId(2)),
        FunctionRef::Host(HostFunctionId(usize::MAX)),
    ] {
        let program = call_program(
            call(reference, "callee", vec![number_literal("NaN")], Type::Bool),
            &[],
            Type::Number,
            span,
        );

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnknownFunctionReference {
                function: "bad".to_owned(),
                reference,
                span,
            }),
        );
    }
}

#[test]
fn nested_builtin_argument_types_are_checked_before_emission_in_all_statement_contexts() {
    let span = Span::new(40, 60);
    let builtin = call(
        FunctionRef::Builtin(BuiltinFunction::Len),
        "not_a_lookup",
        vec![number_literal("NaN")],
        Type::Number,
    );
    let value = call(
        FunctionRef::Host(HostFunctionId(0)),
        "callee",
        vec![builtin],
        Type::Number,
    );
    let statements = vec![
        TypedStatement {
            kind: TypedStatementKind::Expression(value.clone()),
            span,
        },
        TypedStatement {
            kind: TypedStatementKind::Return(Some(value.clone())),
            span,
        },
        declaration(7, Type::Number, value.clone(), span),
        assignment(identifier(0, "p0", Type::Number), value, span),
    ];

    for statement in statements {
        let mut function = user(0, "bad", &[Type::Number], Type::Number);

        function.body = vec![statement];

        assert_eq!(
            compile(&TypedProgram {
                functions: vec![function],
                host_functions: vec![host(0, "callee", &[Type::Number], Type::Number)],
            }),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Array(crate::ast::ArrayElementType::Number),
                found: Type::Number,
                span,
            }),
        );
    }
}

#[test]
fn void_calls_are_rejected_in_scalar_returns_initializers_assignments_and_arguments() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        let value = call(reference, "callee", vec![], Type::Void);
        let statements = vec![
            TypedStatement {
                kind: TypedStatementKind::Return(Some(value.clone())),
                span,
            },
            declaration(7, Type::Number, value.clone(), span),
            assignment(identifier(0, "p0", Type::Number), value.clone(), span),
            TypedStatement {
                kind: TypedStatementKind::Expression(call(
                    FunctionRef::Host(HostFunctionId(1)),
                    "numeric",
                    vec![value],
                    Type::Number,
                )),
                span,
            },
        ];

        for statement in statements {
            let mut program = call_program(number(), &[], Type::Void, span);

            program.functions[0].params = user(0, "bad", &[Type::Number], Type::Number).params;
            program.functions[0].return_type = Type::Number;
            program.functions[0].body = vec![statement];
            program
                .host_functions
                .push(host(1, "numeric", &[Type::Number], Type::Number));

            assert_eq!(
                compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Number,
                    found: Type::Void,
                    span,
                }),
            );
        }
    }
}

#[test]
fn call_statements_preserve_drop_cleanup_after_return_for_host_and_user_results() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("predicate", "env", "predicate", vec![], Type::Bool)
        .unwrap();
    compiler
        .register_host_function("noop", "env", "noop", vec![], Type::Void)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn discard() -> void {
                numeric(); predicate(); noop(); number_user(); bool_user(); void_user();
                return;
                numeric(); predicate(); noop(); number_user(); bool_user(); void_user();
            }
            fn number_user() -> number { return 1; }
            fn bool_user() -> bool { return true; }
            fn void_user() -> void {}",
        )
        .expect("compile call statement cleanup");
    let sequence = vec![
        DecodedInstruction::Call(0),
        DecodedInstruction::Drop,
        DecodedInstruction::Call(1),
        DecodedInstruction::I32Eqz,
        DecodedInstruction::I32Eqz,
        DecodedInstruction::Drop,
        DecodedInstruction::Call(2),
        DecodedInstruction::Call(4),
        DecodedInstruction::Drop,
        DecodedInstruction::Call(5),
        DecodedInstruction::Drop,
        DecodedInstruction::Call(6),
    ];
    let mut expected = sequence.clone();

    expected.push(DecodedInstruction::Return);
    expected.extend(sequence);
    expected.push(DecodedInstruction::End);

    assert_eq!(decode(&bytes).bodies[0], expected);
}

#[test]
fn invalid_unreachable_calls_abort_without_partial_output_and_keep_statement_spans() {
    let span = Span::new(40, 60);
    let cases = vec![
        (
            call(
                FunctionRef::Host(HostFunctionId(1)),
                "callee",
                vec![],
                Type::Number,
            ),
            CodegenError::UnknownFunctionReference {
                function: "bad".to_owned(),
                reference: FunctionRef::Host(HostFunctionId(1)),
                span,
            },
        ),
        (
            call(
                FunctionRef::Host(HostFunctionId(0)),
                "callee",
                vec![],
                Type::Number,
            ),
            CodegenError::ArgumentCountMismatch {
                function: "bad".to_owned(),
                reference: FunctionRef::Host(HostFunctionId(0)),
                expected: 1,
                found: 0,
                span,
            },
        ),
        (
            call(
                FunctionRef::Host(HostFunctionId(0)),
                "callee",
                vec![number()],
                Type::Bool,
            ),
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            call(
                FunctionRef::User(FunctionId(1)),
                "callee",
                vec![boolean()],
                Type::Number,
            ),
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            call(
                FunctionRef::Builtin(BuiltinFunction::Len),
                "len",
                vec![],
                Type::Number,
            ),
            CodegenError::ArgumentCountMismatch {
                function: "bad".to_owned(),
                reference: FunctionRef::Builtin(BuiltinFunction::Len),
                expected: 1,
                found: 0,
                span,
            },
        ),
    ];

    for (value, expected) in cases {
        let mut program = call_program(value, &[Type::Number], Type::Number, span);

        program.functions[0].body.insert(
            0,
            TypedStatement {
                kind: TypedStatementKind::Return(None),
                span: Span::new(0, 7),
            },
        );

        assert_eq!(compile(&program), Err(expected));
    }
}
