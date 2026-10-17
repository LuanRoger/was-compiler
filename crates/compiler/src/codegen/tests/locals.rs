use super::*;

#[test]
fn scalar_parameters_locals_and_mutations_emit_dense_slots_at_all_optimization_levels() {
    use DecodedInstruction::*;

    let source = "fn update(n: number, flag: bool) -> number {
                      let value: number = n * 2 + 1;
                      let ready: bool = flag;
                      n = value - 3;
                      ready = false;
                      value = -n / 2;
                      return value;
                  }";
    let mut expected_bytes = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });
        let program = compiler.analyze(source).expect("analyze scalar mutations");
        let original = program.clone();
        let bytes = compiler
            .compile(&program)
            .expect("compile scalar mutations");
        let module = decode(&bytes);

        assert_eq!(program, original);
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        if level == OptimizationLevel::None {
            assert_eq!(compile(&program).unwrap(), bytes);
        }
        assert_eq!(
            module.signatures,
            vec![(vec![ValType::F64, ValType::I32], vec![ValType::F64])],
        );
        assert_eq!(module.locals, vec![vec![ValType::F64, ValType::I32]]);
        let expected = if level == OptimizationLevel::None {
            vec![
                LocalGet(0),
                F64Const(2.0),
                F64Mul,
                F64Const(1.0),
                F64Add,
                LocalSet(2),
                LocalGet(1),
                LocalSet(3),
                LocalGet(2),
                F64Const(3.0),
                F64Sub,
                LocalSet(0),
                I32Const(0),
                LocalSet(3),
                LocalGet(0),
                F64Neg,
                F64Const(2.0),
                F64Div,
                LocalSet(2),
                LocalGet(2),
                Return,
                End,
            ]
        } else {
            vec![
                LocalGet(0),
                I32Const(2),
                Runtime("F64ConvertI32S".into()),
                F64Mul,
                I32Const(1),
                Runtime("F64ConvertI32S".into()),
                F64Add,
                LocalSet(2),
                LocalGet(1),
                LocalSet(3),
                LocalGet(2),
                I32Const(3),
                Runtime("F64ConvertI32S".into()),
                F64Sub,
                LocalSet(0),
                I32Const(0),
                LocalSet(3),
                LocalGet(0),
                F64Neg,
                I32Const(2),
                Runtime("F64ConvertI32S".into()),
                F64Div,
                LocalSet(2),
                LocalGet(2),
                Return,
                End,
            ]
        };
        assert_eq!(
            module.bodies,
            vec![expected],
            "optimization level {level:?}"
        );

        if level != OptimizationLevel::None {
            if let Some(expected) = &expected_bytes {
                assert_eq!(&bytes, expected, "optimization level {level:?}");
            } else {
                expected_bytes = Some(bytes);
            }
        }
    }
}

#[test]
fn bool_literals_variables_and_returns_preserve_mixed_local_order_and_reset_function_ids() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn first(flag: bool, n: number) -> bool {
                 let value: number = n;
                 let ready: bool = true;
                 let other: number = value;
                 let copy: bool = flag;
                 ready = copy;
                 flag = false;
                 return ready;
             }
             fn second(flag: bool) -> bool {
                 let ready: bool = flag;
                 ready = false;
                 return ready;
             }
             fn yes() -> bool { return true; }
             fn no() -> bool { return false; }",
        )
        .expect("compile bool bindings and returns");
    let module = decode(&bytes);

    assert_eq!(
        module.locals,
        vec![
            vec![ValType::F64, ValType::I32, ValType::F64, ValType::I32],
            vec![ValType::I32],
            vec![],
            vec![],
        ],
    );
    assert_eq!(
        module.bodies,
        vec![
            vec![
                LocalGet(1),
                LocalSet(2),
                I32Const(1),
                LocalSet(3),
                LocalGet(2),
                LocalSet(4),
                LocalGet(0),
                LocalSet(5),
                LocalGet(5),
                LocalSet(3),
                I32Const(0),
                LocalSet(0),
                LocalGet(3),
                Return,
                End,
            ],
            vec![
                LocalGet(0),
                LocalSet(1),
                I32Const(0),
                LocalSet(1),
                LocalGet(1),
                Return,
                End,
            ],
            vec![I32Const(1), Return, End],
            vec![I32Const(0), Return, End],
        ],
    );
}

#[test]
fn scalar_expression_statements_drop_values_even_after_return() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn discard(n: number, flag: bool) -> void {
                 n + 1;
                 true;
                 flag;
                 return;
                 let value: number = n;
                 value = 2;
                 value;
                 false;
             }",
        )
        .expect("compile discarded scalar expressions");
    let module = decode(&bytes);

    assert_eq!(module.locals, vec![vec![ValType::F64]]);
    assert_eq!(
        module.bodies,
        vec![vec![
            LocalGet(0),
            F64Const(1.0),
            F64Add,
            Drop,
            I32Const(1),
            Drop,
            LocalGet(1),
            Drop,
            Return,
            LocalGet(0),
            LocalSet(2),
            F64Const(2.0),
            LocalSet(2),
            LocalGet(2),
            Drop,
            I32Const(0),
            Drop,
            End,
        ]],
    );
}

#[test]
fn sparse_variable_ids_and_diagnostic_names_do_not_determine_wasm_indices() {
    use DecodedInstruction::*;

    let span = Span::new(12, 34);
    let mut function = user(0, "sparse", &[Type::Number, Type::Bool], Type::Number);

    function.params[0].id = VariableId(90);
    function.params[1].id = VariableId(3);
    function.body = vec![
        declaration(
            usize::MAX,
            Type::Bool,
            identifier(3, "renamed", Type::Bool),
            span,
        ),
        declaration(
            7,
            Type::Number,
            identifier(90, "not_p0", Type::Number),
            span,
        ),
        assignment(
            identifier(90, "renamed", Type::Number),
            identifier(7, "not_x", Type::Number),
            span,
        ),
        TypedStatement {
            kind: TypedStatementKind::Return(Some(identifier(7, "p0", Type::Number))),
            span,
        },
    ];

    let layout = LocalLayout::new(&function).expect("layout sparse variable IDs");

    assert_eq!(
        layout.types,
        vec![wasm_encoder::ValType::I32, wasm_encoder::ValType::F64]
    );

    for (id, index, ty) in [
        (90, 0, Type::Number),
        (3, 1, Type::Bool),
        (usize::MAX, 2, Type::Bool),
        (7, 3, Type::Number),
    ] {
        let local = layout.get(VariableId(id)).expect("mapped variable");
        let copied = local;

        assert_eq!((local.index.raw(), local.ty), (index, ty));
        assert_eq!((copied.index.raw(), copied.ty), (index, ty));
    }

    assert!(layout.get(VariableId(0)).is_none());

    let module = decode(
        &compile(&TypedProgram {
            functions: vec![function],
            host_functions: vec![],
        })
        .expect("compile sparse variable IDs"),
    );

    assert_eq!(module.locals, vec![vec![ValType::I32, ValType::F64]]);
    assert_eq!(
        module.bodies,
        vec![vec![
            LocalGet(1),
            LocalSet(2),
            LocalGet(0),
            LocalSet(3),
            LocalGet(3),
            LocalSet(0),
            LocalGet(3),
            Return,
            End,
        ]]
    );
}

#[test]
fn local_layout_collects_and_emits_nested_shadow_bindings_in_preorder() {
    fn check_binding(layout: &LocalLayout, expr: &TypedExpr, index: u32, ty: Type) {
        let TypedExprKind::Identifier { id, .. } = &expr.kind else {
            panic!("expected resolved identifier");
        };
        let local = layout.get(*id).expect("shadow binding");

        assert_eq!((local.index.raw(), local.ty), (index, ty));
    }

    fn collect(block: &[TypedStatement], ids: &mut Vec<(VariableId, Type)>) {
        for statement in block {
            match &statement.kind {
                TypedStatementKind::Let { id, ty, .. } => ids.push((*id, *ty)),
                TypedStatementKind::If { then, else_, .. } => {
                    collect(then, ids);

                    if let Some(else_) = else_ {
                        collect(else_, ids);
                    }
                }
                TypedStatementKind::While { body, .. } => collect(body, ids),
                _ => {}
            }
        }
    }

    let program = Compiler::new()
        .analyze(
            "fn shadow(value: number) -> number {
             let copy: number = value;
             if true {
                 let value: bool = true;
                 if value {
                     let copy: number = 2;
                     copy;
                 } else {
                     let copy: bool = false;
                     copy;
                 }
                 value;
             } else {
                 let value: number = 3;
                 value;
             }
             while false {
                 let value: bool = false;
                 value;
             }
             let tail: number = copy;
             return value;
         }",
        )
        .expect("analyze nested shadow bindings");
    let function = &program.functions[0];
    let layout = LocalLayout::new(function).expect("layout nested shadow bindings");
    let mut ids = vec![(function.params[0].id, Type::Number)];

    collect(&function.body, &mut ids);

    assert_eq!(
        layout.types,
        vec![
            wasm_encoder::ValType::F64,
            wasm_encoder::ValType::I32,
            wasm_encoder::ValType::F64,
            wasm_encoder::ValType::I32,
            wasm_encoder::ValType::F64,
            wasm_encoder::ValType::I32,
            wasm_encoder::ValType::F64,
        ]
    );
    assert_eq!(ids.len(), 8);

    for (index, (id, ty)) in ids.into_iter().enumerate() {
        let local = layout.get(id).expect("preorder local slot");

        assert_eq!((local.index.raw(), local.ty), (index as u32, ty));
    }

    let TypedStatementKind::If {
        then,
        else_: Some(else_),
        ..
    } = &function.body[1].kind
    else {
        panic!("expected outer if");
    };
    let TypedStatementKind::If {
        cond,
        then: inner_then,
        else_: Some(inner_else),
    } = &then[1].kind
    else {
        panic!("expected nested if");
    };

    check_binding(&layout, cond, 2, Type::Bool);

    for (statement, index, ty) in [
        (&inner_then[1], 3, Type::Number),
        (&inner_else[1], 4, Type::Bool),
        (&then[2], 2, Type::Bool),
        (&else_[1], 5, Type::Number),
    ] {
        let TypedStatementKind::Expression(expr) = &statement.kind else {
            panic!("expected shadow expression");
        };

        check_binding(&layout, expr, index, ty);
    }

    let TypedStatementKind::While { body, .. } = &function.body[2].kind else {
        panic!("expected while");
    };
    let TypedStatementKind::Expression(expr) = &body[1].kind else {
        panic!("expected loop shadow expression");
    };

    check_binding(&layout, expr, 6, Type::Bool);

    let TypedStatementKind::Let { value, .. } = &function.body[3].kind else {
        panic!("expected trailing declaration");
    };

    check_binding(&layout, value, 1, Type::Number);

    let TypedStatementKind::Return(Some(value)) = &function.body[4].kind else {
        panic!("expected restored parameter return");
    };

    check_binding(&layout, value, 0, Type::Number);

    let module = decode(&compile(&program).expect("compile nested shadow bindings"));

    assert_eq!(
        module.locals[0],
        vec![
            ValType::F64,
            ValType::I32,
            ValType::F64,
            ValType::I32,
            ValType::F64,
            ValType::I32,
            ValType::F64
        ]
    );
    assert!(module.bodies[0].contains(&DecodedInstruction::If(BlockType::Empty)));
    assert!(module.bodies[0].contains(&DecodedInstruction::Loop(BlockType::Empty)));
}

#[test]
fn duplicate_variable_ids_are_rejected_at_the_repeated_parameter_or_declaration() {
    let first = Span::new(1, 4);
    let repeated = Span::new(12, 34);
    let mut parameters = user(0, "bad", &[Type::Number, Type::Bool], Type::Void);

    parameters.params[0].id = VariableId(77);
    parameters.params[1].id = VariableId(77);
    parameters.params[1].span = repeated;

    let mut parameter_local = user(0, "bad", &[Type::Number], Type::Void);

    parameter_local.params[0].id = VariableId(77);
    parameter_local
        .body
        .push(declaration(77, Type::Bool, boolean(), repeated));

    let mut locals = user(0, "bad", &[], Type::Void);

    locals.body = vec![
        declaration(77, Type::Number, number(), first),
        TypedStatement {
            kind: TypedStatementKind::Return(None),
            span: first,
        },
        declaration(77, Type::Bool, boolean(), repeated),
    ];

    let mut nested = user(0, "bad", &[], Type::Void);

    nested.body = vec![TypedStatement {
        kind: TypedStatementKind::If {
            cond: Box::new(boolean()),
            then: vec![declaration(77, Type::Number, number(), first)],
            else_: Some(vec![TypedStatement {
                kind: TypedStatementKind::While {
                    id: LoopId(0),
                    cond: Box::new(boolean()),
                    body: vec![declaration(77, Type::Bool, boolean(), repeated)],
                },
                span: first,
            }]),
        },
        span: first,
    }];

    for function in [parameters, parameter_local, locals, nested] {
        let expected = CodegenError::DuplicateVariableId {
            function: "bad".to_owned(),
            id: VariableId(77),
            span: repeated,
        };

        assert_eq!(LocalLayout::new(&function).err(), Some(expected));
        assert_eq!(
            compile(&TypedProgram {
                functions: vec![function],
                host_functions: vec![],
            }),
            Err(CodegenError::DuplicateVariableId {
                function: "bad".to_owned(),
                id: VariableId(77),
                span: repeated,
            })
        );
    }
}

#[test]
fn unknown_ids_are_rejected_for_reads_and_assignments_without_name_or_cross_function_lookup() {
    let span = Span::new(12, 34);

    for ty in [Type::Number, Type::Bool, Type::String, Type::Void] {
        for kind in [
            TypedStatementKind::Expression(identifier(99, "p0", ty)),
            TypedStatementKind::Assignment {
                target: identifier(99, "p0", ty),
                value: number_literal("NaN"),
            },
        ] {
            let mut previous = user(0, "previous", &[], Type::Void);

            previous
                .body
                .push(declaration(99, Type::Number, number(), Span::new(0, 1)));

            let mut function = user(1, "bad", &[Type::Number], Type::Void);

            function.body.push(TypedStatement { kind, span });

            assert_eq!(
                compile(&TypedProgram {
                    functions: vec![previous, function],
                    host_functions: vec![],
                }),
                Err(CodegenError::UnknownVariable {
                    function: "bad".to_owned(),
                    id: VariableId(99),
                    span,
                })
            );
        }
    }
}

#[test]
fn forward_reads_self_initializers_and_assignments_before_declaration_are_uninitialized() {
    let span = Span::new(12, 34);
    let later = Span::new(40, 60);
    let cases = vec![
        vec![
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(7, "x", Type::Number)),
                span,
            },
            declaration(7, Type::Number, number(), later),
        ],
        vec![
            declaration(8, Type::Number, identifier(7, "x", Type::Number), span),
            declaration(7, Type::Number, number(), later),
        ],
        vec![declaration(
            7,
            Type::Number,
            identifier(7, "renamed", Type::Number),
            span,
        )],
        vec![
            assignment(
                identifier(7, "x", Type::Number),
                number_literal("NaN"),
                span,
            ),
            declaration(7, Type::Number, number(), later),
        ],
        vec![
            TypedStatement {
                kind: TypedStatementKind::Return(Some(identifier(7, "x", Type::Number))),
                span,
            },
            declaration(7, Type::Number, number(), later),
        ],
        vec![declaration(
            7,
            Type::Bool,
            identifier(7, "x", Type::Bool),
            span,
        )],
        vec![
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(7, "x", Type::Bool)),
                span,
            },
            declaration(7, Type::Bool, boolean(), later),
        ],
        vec![
            assignment(identifier(7, "x", Type::Bool), boolean(), span),
            declaration(7, Type::Bool, boolean(), later),
        ],
    ];

    for body in cases {
        for after_return in [false, true] {
            let mut program = local_program(body.clone());

            if matches!(body[0].kind, TypedStatementKind::Return(_)) {
                program.functions[0].return_type = Type::Number;
            }

            if after_return {
                let value = if program.functions[0].return_type == Type::Number {
                    Some(number())
                } else {
                    None
                };

                program.functions[0].body.insert(
                    0,
                    TypedStatement {
                        kind: TypedStatementKind::Return(value),
                        span: Span::new(0, 7),
                    },
                );
            }

            assert_eq!(
                compile(&program),
                Err(CodegenError::UninitializedVariable {
                    function: "bad".to_owned(),
                    id: VariableId(7),
                    span,
                })
            );
        }
    }
}

#[test]
fn scalar_local_types_are_checked_before_initialization_or_rhs_emission() {
    let span = Span::new(12, 34);
    let cases = vec![
        (
            declaration(8, Type::Number, boolean(), span),
            Type::Number,
            Type::Bool,
        ),
        (
            declaration(8, Type::Bool, number(), span),
            Type::Bool,
            Type::Number,
        ),
        (
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(0, "p0", Type::Bool)),
                span,
            },
            Type::Number,
            Type::Bool,
        ),
        (
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(7, "x", Type::Bool)),
                span,
            },
            Type::Number,
            Type::Bool,
        ),
        (
            assignment(identifier(0, "p0", Type::Number), boolean(), span),
            Type::Number,
            Type::Bool,
        ),
        (
            assignment(identifier(0, "p0", Type::Bool), boolean(), span),
            Type::Number,
            Type::Bool,
        ),
        (
            assignment(identifier(7, "x", Type::Bool), number_literal("NaN"), span),
            Type::Number,
            Type::Bool,
        ),
        (
            TypedStatement {
                kind: TypedStatementKind::Expression(TypedExpr {
                    kind: TypedExprKind::Bool(true),
                    ty: Type::Number.into(),
                }),
                span,
            },
            Type::Bool,
            Type::Number,
        ),
    ];

    for (statement, expected, found) in cases {
        let mut program = local_program(vec![
            statement,
            declaration(7, Type::Number, number(), Span::new(40, 60)),
        ]);

        program.functions[0].params = user(0, "bad", &[Type::Number], Type::Void).params;

        assert_eq!(
            compile(&program),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected,
                found,
                span,
            })
        );
    }
}

#[test]
fn non_identifier_assignment_targets_and_index_bases_are_rejected_before_rhs() {
    let span = Span::new(12, 34);
    let targets = vec![
        number(),
        boolean(),
        binary(number(), BinaryOp::Add, number()),
        TypedExpr {
            kind: TypedExprKind::Call {
                func: FunctionRef::User(FunctionId(0)),
                name: "bad".to_owned(),
                args: vec![],
            },
            ty: Type::Number.into(),
        },
        TypedExpr {
            kind: TypedExprKind::Index {
                expr: Box::new(TypedExpr {
                    kind: TypedExprKind::Array(
                        vec![number()],
                        crate::ast::ArrayElementType::Number,
                    ),
                    ty: Type::Array(crate::ast::ArrayElementType::Number).into(),
                }),
                index: Box::new(number_literal("0")),
            },
            ty: Type::Number.into(),
        },
    ];

    for target in targets {
        assert_eq!(
            compile(&local_program(vec![assignment(
                target,
                number_literal("NaN"),
                span
            )])),
            Err(CodegenError::InvalidAssignmentTarget {
                function: "bad".to_owned(),
                span,
            }),
        );
    }
}

#[test]
fn unsupported_local_types_are_rejected_before_body_emission_even_in_unreachable_nested_blocks() {
    let span = Span::new(40, 60);

    for ty in [Type::Void] {
        for nested in [false, true] {
            let local = declaration(7, ty, number(), span);
            let statement = if nested {
                TypedStatement {
                    kind: TypedStatementKind::If {
                        cond: Box::new(boolean()),
                        then: vec![],
                        else_: Some(vec![TypedStatement {
                            kind: TypedStatementKind::While {
                                id: LoopId(0),
                                cond: Box::new(boolean()),
                                body: vec![local],
                            },
                            span: Span::new(30, 70),
                        }]),
                    },
                    span: Span::new(20, 80),
                }
            } else {
                local
            };
            let program = local_program(vec![
                TypedStatement {
                    kind: TypedStatementKind::Expression(number_literal("NaN")),
                    span: Span::new(0, 7),
                },
                TypedStatement {
                    kind: TypedStatementKind::Return(None),
                    span: Span::new(8, 15),
                },
                statement,
            ]);

            assert_eq!(
                compile(&program),
                Err(CodegenError::UnsupportedLocalType {
                    function: "bad".to_owned(),
                    ty,
                    span,
                })
            );
        }
    }
}

#[test]
fn unreachable_local_initializers_and_assignments_are_still_validated() {
    let span = Span::new(40, 60);

    for kind in [
        declaration(8, Type::Number, number_literal("NaN"), span).kind,
        assignment(
            identifier(7, "x", Type::Number),
            number_literal("NaN"),
            span,
        )
        .kind,
    ] {
        let program = local_program(vec![
            declaration(7, Type::Number, number(), Span::new(0, 7)),
            TypedStatement {
                kind: TypedStatementKind::Return(None),
                span: Span::new(8, 15),
            },
            TypedStatement { kind, span },
        ]);

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            })
        );
    }
}
