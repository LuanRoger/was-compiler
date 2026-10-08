use super::*;

#[test]
fn bare_void_returns_are_emitted_including_unreachable_returns() {
    let bytes = Compiler::new()
        .compile_source(
            "fn once() -> void { return; }
             fn twice(n: number) -> void { return; return; }",
        )
        .expect("compile bare void returns");

    assert_eq!(
        decode(&bytes).bodies,
        vec![
            vec![DecodedInstruction::Return, DecodedInstruction::End],
            vec![
                DecodedInstruction::Return,
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        ],
    );
}

#[test]
fn all_numeric_returns_are_emitted_in_source_order_even_when_unreachable() {
    let bytes = Compiler::new()
        .compile_source("fn value() -> number { return 42; return 0.25; return 0; }")
        .expect("compile multiple numeric returns");

    assert_eq!(
        decode(&bytes).bodies,
        vec![vec![
            DecodedInstruction::F64Const(42.0),
            DecodedInstruction::Return,
            DecodedInstruction::F64Const(0.25),
            DecodedInstruction::Return,
            DecodedInstruction::F64Const(0.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]],
    );
}

#[test]
fn if_else_complete_returns_emit_unreachable_after_end_and_validate() {
    let bytes = Compiler::new()
        .compile_source(
            "fn choose(flag: bool) -> number { if flag { return 1; } else { return 2; } }
         fn partial(flag: bool) -> number { if flag { return 1; } return 2; }
         fn noop(flag: bool) -> void { if flag { return; } else { return; } }",
        )
        .expect("compile conditional returns");
    let module = decode(&bytes);

    assert_eq!(
        module.bodies[0],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::If(BlockType::Empty),
            DecodedInstruction::F64Const(1.0),
            DecodedInstruction::Return,
            DecodedInstruction::Else,
            DecodedInstruction::F64Const(2.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
            DecodedInstruction::Unreachable,
            DecodedInstruction::End,
        ]
    );
    assert_eq!(
        module.bodies[1],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::If(BlockType::Empty),
            DecodedInstruction::F64Const(1.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
            DecodedInstruction::F64Const(2.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]
    );
    assert_eq!(
        module.bodies[2],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::If(BlockType::Empty),
            DecodedInstruction::Return,
            DecodedInstruction::Else,
            DecodedInstruction::Return,
            DecodedInstruction::End,
            DecodedInstruction::Unreachable,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn while_emits_empty_block_loop_exit_and_back_edge_with_body_locals() {
    let bytes = Compiler::new()
        .compile_source(
            "fn countdown(n: number) -> number {
             while n > 0 { let step: number = 1; n = n - step; }
             return n;
         }",
        )
        .expect("compile loop with declaration");
    let module = decode(&bytes);

    assert_eq!(module.locals[0], vec![ValType::F64]);
    assert_eq!(
        module.bodies[0],
        vec![
            DecodedInstruction::Block(BlockType::Empty),
            DecodedInstruction::Loop(BlockType::Empty),
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::F64Const(0.0),
            DecodedInstruction::F64Gt,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::BrIf(1),
            DecodedInstruction::F64Const(1.0),
            DecodedInstruction::LocalSet(1),
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::LocalGet(1),
            DecodedInstruction::F64Sub,
            DecodedInstruction::LocalSet(0),
            DecodedInstruction::Br(0),
            DecodedInstruction::End,
            DecodedInstruction::End,
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn nested_loops_keep_relative_exit_and_back_edge_depths() {
    let bytes = Compiler::new()
        .compile_source(
            "fn nested(a: bool, b: bool) -> void { while a { while b { b = false; } a = false; } }",
        )
        .expect("compile nested loops");
    let module = decode(&bytes);
    let body = &module.bodies[0];

    assert_eq!(
        body.iter()
            .filter(|op| **op == DecodedInstruction::Block(BlockType::Empty))
            .count(),
        2
    );
    assert_eq!(
        body.iter()
            .filter(|op| **op == DecodedInstruction::Loop(BlockType::Empty))
            .count(),
        2
    );
    assert_eq!(
        body.iter()
            .filter_map(|op| if let DecodedInstruction::BrIf(depth) = op {
                Some(*depth)
            } else {
                None
            })
            .collect::<Vec<_>>(),
        vec![1, 1]
    );
    assert_eq!(
        body.iter()
            .filter_map(|op| if let DecodedInstruction::Br(depth) = op {
                Some(*depth)
            } else {
                None
            })
            .collect::<Vec<_>>(),
        vec![0, 0]
    );
}

#[test]
fn forged_if_and_while_conditions_require_bool_and_preserve_statement_spans() {
    let span = Span::new(12, 34);

    for cond in [
        number(),
        call(
            FunctionRef::User(FunctionId(99)),
            "missing",
            vec![],
            Type::Void,
        ),
    ] {
        for statement in [
            conditional(cond.clone(), vec![], None, span),
            while_loop(cond.clone(), vec![], span),
        ] {
            assert_eq!(
                compile(&local_program(vec![statement])),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Bool,
                    found: cond.ty,
                    span,
                })
            );
        }
    }

    let forged = TypedExpr {
        kind: TypedExprKind::Number("1".to_owned()),
        ty: Type::Bool,
    };

    for statement in [
        conditional(forged.clone(), vec![], None, span),
        while_loop(forged, vec![], span),
    ] {
        assert_eq!(
            compile(&local_program(vec![statement])),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            })
        );
    }
}

#[test]
fn nested_complete_and_partial_returns_validate_with_fallbacks() {
    for source in [
        "fn complete(a: bool, b: bool) -> number { if a { if b { return 1; } else { return 2; } } else { return 3; } }",
        "fn complete(a: bool, b: bool) -> bool { if a { return b; } else { if b { return true; } else { return false; } } }",
        "fn partial(a: bool, b: bool) -> number { if a { if b { return 1; } return 2; } else { return 3; } }",
        "fn partial(a: bool, b: bool) -> number { if a { if b { return 1; } } else { return 2; } return 3; }",
        "fn loop_fallback(a: bool) -> number { while a { if a { return 1; } else { return 2; } } return 3; }",
        "fn already_returned(a: bool) -> number { return 1; while a { return 2; } if a { return 3; } }",
    ] {
        let bytes = Compiler::new()
            .compile_source(source)
            .expect("compile complete return paths");

        decode(&bytes);
    }
}

#[test]
fn missing_return_paths_include_partial_nested_conditionals_and_all_loops() {
    let span = Span::new(12, 34);

    for return_type in [Type::Number, Type::Bool] {
        let value = if return_type == Type::Number {
            number()
        } else {
            boolean()
        };
        let ret = returned(Some(value), span);
        let full_if = conditional(boolean(), vec![ret.clone()], Some(vec![ret.clone()]), span);
        let cases = vec![
            vec![conditional(boolean(), vec![ret.clone()], None, span)],
            vec![conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![]),
                span,
            )],
            vec![conditional(
                boolean(),
                vec![],
                Some(vec![ret.clone()]),
                span,
            )],
            vec![conditional(
                boolean(),
                vec![conditional(boolean(), vec![ret.clone()], None, span)],
                Some(vec![ret.clone()]),
                span,
            )],
            vec![conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![conditional(boolean(), vec![ret.clone()], None, span)]),
                span,
            )],
            vec![while_loop(boolean(), vec![ret.clone()], span)],
            vec![while_loop(
                TypedExpr {
                    kind: TypedExprKind::Bool(false),
                    ty: Type::Bool,
                },
                vec![ret.clone()],
                span,
            )],
            vec![while_loop(boolean(), vec![full_if], span)],
            vec![conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![while_loop(boolean(), vec![ret], span)]),
                span,
            )],
        ];

        for body in cases {
            let mut program = local_program(body);

            program.functions[0].return_type = return_type;

            assert_eq!(
                compile(&program),
                Err(CodegenError::MissingReturnValue {
                    function: "bad".to_owned(),
                    return_type,
                }),
                "body {:?}",
                program.functions[0].body
            );
        }
    }
}

#[test]
fn forged_branch_and_loop_local_reads_do_not_escape_their_scope() {
    let outer = Span::new(0, 100);
    let span = Span::new(40, 60);
    let local = declaration(7, Type::Bool, boolean(), Span::new(12, 34));
    let read = TypedStatement {
        kind: TypedStatementKind::Expression(identifier(7, "x", Type::Bool)),
        span,
    };
    let cases = vec![
        vec![conditional(
            boolean(),
            vec![local.clone()],
            Some(vec![read.clone()]),
            outer,
        )],
        vec![
            conditional(boolean(), vec![local.clone()], None, outer),
            read.clone(),
        ],
        vec![
            conditional(boolean(), vec![], Some(vec![local.clone()]), outer),
            read.clone(),
        ],
        vec![conditional(
            boolean(),
            vec![read.clone()],
            Some(vec![local.clone()]),
            outer,
        )],
        vec![
            while_loop(boolean(), vec![local.clone()], outer),
            read.clone(),
        ],
        vec![while_loop(
            identifier(7, "x", Type::Bool),
            vec![local.clone()],
            span,
        )],
        vec![
            conditional(boolean(), vec![local.clone()], None, outer),
            while_loop(identifier(7, "x", Type::Bool), vec![], span),
        ],
        vec![
            while_loop(boolean(), vec![local.clone()], outer),
            while_loop(identifier(7, "x", Type::Bool), vec![], span),
        ],
        vec![conditional(
            boolean(),
            vec![
                conditional(boolean(), vec![local.clone()], None, outer),
                read.clone(),
            ],
            None,
            outer,
        )],
        vec![
            conditional(boolean(), vec![local], None, outer),
            assignment(identifier(7, "x", Type::Bool), boolean(), span),
        ],
    ];

    for body in cases {
        assert_eq!(
            compile(&local_program(body)),
            Err(CodegenError::UninitializedVariable {
                function: "bad".to_owned(),
                id: VariableId(7),
                span,
            })
        );
    }
}

#[test]
fn scoped_branch_and_loop_declarations_preserve_initialized_outer_bindings() {
    let span = Span::new(12, 34);
    let outer = identifier(7, "same_name", Type::Number);
    let branch = vec![
        declaration(8, Type::Bool, boolean(), span),
        assignment(outer.clone(), number_literal("2"), span),
    ];
    let loop_body = vec![
        declaration(9, Type::Number, number_literal("3"), span),
        assignment(
            outer.clone(),
            identifier(9, "same_name", Type::Number),
            span,
        ),
    ];
    let program = local_program(vec![
        declaration(7, Type::Number, number(), span),
        conditional(
            boolean(),
            branch,
            Some(vec![assignment(outer.clone(), number_literal("4"), span)]),
            span,
        ),
        while_loop(
            TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool,
            },
            loop_body,
            span,
        ),
        TypedStatement {
            kind: TypedStatementKind::Expression(outer),
            span,
        },
    ]);
    let module = decode(&compile(&program).expect("outer initialization survives scopes"));

    assert_eq!(
        module.locals[0],
        vec![ValType::F64, ValType::I32, ValType::F64]
    );
}

#[test]
fn invalid_statements_in_unreachable_branches_loops_and_continuations_are_diagnosed() {
    let span = Span::new(40, 60);
    let outer = Span::new(0, 100);
    let invalid = TypedStatement {
        kind: TypedStatementKind::Expression(number_literal("NaN")),
        span,
    };
    let ret = returned(None, Span::new(0, 7));
    let complete = conditional(boolean(), vec![ret.clone()], Some(vec![ret.clone()]), outer);
    let cases = vec![
        vec![
            ret.clone(),
            conditional(boolean(), vec![invalid.clone()], None, outer),
        ],
        vec![conditional(
            boolean(),
            vec![ret.clone()],
            Some(vec![invalid.clone()]),
            outer,
        )],
        vec![
            ret.clone(),
            while_loop(boolean(), vec![invalid.clone()], outer),
        ],
        vec![while_loop(
            TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool,
            },
            vec![invalid.clone()],
            outer,
        )],
        vec![conditional(
            boolean(),
            vec![ret.clone(), invalid.clone()],
            Some(vec![ret.clone()]),
            outer,
        )],
        vec![while_loop(boolean(), vec![ret, invalid.clone()], outer)],
        vec![complete, invalid],
    ];

    for body in cases {
        assert_eq!(
            compile(&local_program(body)),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            })
        );
    }
}

#[test]
fn nested_return_type_errors_keep_inner_statement_spans() {
    let span = Span::new(40, 60);
    let outer = Span::new(0, 100);

    for value in [None, Some(boolean())] {
        let ret = returned(value.clone(), span);

        for statement in [
            conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![returned(Some(number()), outer)]),
                outer,
            ),
            while_loop(boolean(), vec![ret], outer),
        ] {
            let mut program = return_program(Some(number()), Type::Number, outer);

            program.functions[0].body.push(statement);

            assert_eq!(
                compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Number,
                    found: value.as_ref().map_or(Type::Void, |expr| expr.ty),
                    span,
                })
            );
        }
    }
}
