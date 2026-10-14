use was_compiler::{CodegenError, Compiler, Type, tokens::Span, typed_ast::*};
use wasmparser::{Operator, Parser, Payload, Validator};

fn span() -> Span {
    Span::new(31, 57)
}
fn number(value: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number(value.into()),
        ty: Type::Number.into(),
    }
}
fn statement(kind: TypedStatementKind) -> TypedStatement {
    TypedStatement { kind, span: span() }
}
fn closure(id: usize, owner: BindingOwner, body: TypedClosureBody, result: Type) -> TypedClosure {
    TypedClosure {
        id: CallableId(id),
        owner,
        name: format!("c{id}"),
        name_span: span(),
        params: vec![],
        return_type: result,
        body,
        captures: vec![],
        dependencies: vec![],
        span: span(),
        body_span: span(),
    }
}
fn lexical(id: usize, args: Vec<TypedExpr>, result: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::LexicalCall {
            callable: CallableId(id),
            args,
            span: span(),
        },
        ty: result.into(),
    }
}
fn program(body: Vec<TypedStatement>) -> TypedProgram {
    TypedProgram {
        host_functions: vec![],
        functions: vec![TypedFunction {
            id: FunctionId(0),
            name: "main".into(),
            params: vec![],
            return_type: Type::Number,
            contracts: vec![],
            body,
            span: span(),
        }],
    }
}
fn operators(bytes: &[u8], index: usize) -> Vec<Operator<'_>> {
    Validator::new().validate_all(bytes).unwrap();
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .nth(index)
        .unwrap()
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

fn block(statements: Vec<TypedStatement>, tail: Option<TypedExpr>) -> TypedClosureBody {
    TypedClosureBody::Block(TypedValueBlock {
        statements,
        tail: tail.map(Box::new),
        span: span(),
    })
}
fn assert_invalid(program: &TypedProgram, reason: &str, expected_span: Span) {
    let error = Compiler::new().compile(program).unwrap_err();
    assert!(
        matches!(&error, CodegenError::InvalidClosure { reason: actual, span } if *actual == reason && *span == expected_span),
        "{error:?}"
    );
}

#[test]
fn wide_independent_siblings_keep_direct_helpers_and_lexical_dependency_checks() {
    const WIDTH: usize = 2048;
    let mut statements = Vec::with_capacity(WIDTH + 1);
    for id in 0..WIDTH {
        statements.push(statement(TypedStatementKind::Closure(closure(
            id,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(number("7"))),
            Type::Number,
        ))));
    }
    statements.push(statement(TypedStatementKind::Return(Some(lexical(
        WIDTH - 1,
        vec![],
        Type::Number,
    )))));
    let mut typed = program(statements);
    let bytes = Compiler::new().compile(&typed).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let bodies = Parser::new(0)
        .parse_all(&bytes)
        .filter(|payload| matches!(payload, Ok(Payload::CodeSectionEntry(_))))
        .count();
    assert_eq!(bodies, WIDTH + 1);
    assert!(operators(&bytes, 0).iter().any(|op| matches!(op, Operator::Call { function_index } if *function_index == u32::try_from(WIDTH).unwrap())));
    for payload in Parser::new(0).parse_all(&bytes) {
        assert!(!matches!(
            payload.unwrap(),
            Payload::MemorySection(_)
                | Payload::GlobalSection(_)
                | Payload::DataSection(_)
                | Payload::TableSection(_)
        ));
    }

    let TypedStatementKind::Closure(last) = &mut typed.functions[0].body[WIDTH - 1].kind else {
        panic!()
    };
    last.body = TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number)));
    last.dependencies = vec![CallableId(0)];
    let bytes = Compiler::new().compile(&typed).unwrap();
    assert!(
        operators(&bytes, WIDTH)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 1 }))
    );
    let TypedStatementKind::Closure(last) = &mut typed.functions[0].body[WIDTH - 1].kind else {
        panic!()
    };
    last.dependencies.clear();
    last.span = Span::new(301, 351);
    assert_invalid(&typed, "invalid callable dependency", Span::new(301, 351));
}

#[test]
fn missing_dependency_metadata_preserves_body_checks_and_distinct_error_origins() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let call_span = Span::new(101, 107);
    let mut call = lexical(0, vec![], Type::Number);
    let TypedExprKind::LexicalCall { span, .. } = &mut call.kind else {
        panic!()
    };
    *span = call_span;
    let mut second = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(call.clone())),
        Type::Number,
    );
    second.span = Span::new(81, 109);
    let fixture = |second| {
        program(vec![
            statement(TypedStatementKind::Closure(first.clone())),
            statement(TypedStatementKind::Closure(second)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ])
    };
    assert_invalid(
        &fixture(second.clone()),
        "invalid callable dependency",
        second.span,
    );
    second.body = block(
        vec![statement(TypedStatementKind::Expression(call.clone()))],
        Some(number("invalid")),
    );
    assert!(matches!(
        Compiler::new().compile(&fixture(second.clone())),
        Err(CodegenError::InvalidNumberLiteral { .. })
    ));

    // An earlier declaration in an exited sibling scope is not an omitted permission.
    let mut hidden = first;
    hidden.span = Span::new(41, 79);
    second.body = TypedClosureBody::Expression(Box::new(call));
    let typed = program(vec![
        statement(TypedStatementKind::If {
            cond: Box::new(TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            }),
            then: vec![statement(TypedStatementKind::Closure(hidden))],
            else_: None,
        }),
        statement(TypedStatementKind::Closure(second)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&typed, "invalid callable call", call_span);
}

#[test]
fn nested_missing_dependencies_keep_the_parent_declaration_error_origin() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let mut child = closure(
        2,
        BindingOwner::Closure(CallableId(1)),
        TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
        Type::Number,
    );
    child.dependencies = vec![CallableId(0)];
    child.span = Span::new(201, 231);
    let mut parent = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(child))],
            Some(number("1")),
        ),
        Type::Number,
    );
    parent.span = Span::new(151, 251);
    let typed = program(vec![
        statement(TypedStatementKind::Closure(first)),
        statement(TypedStatementKind::Closure(parent)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&typed, "invalid callable dependency", Span::new(151, 251));
}

#[test]
fn nested_calls_preserve_transitive_external_dependencies_and_source_preorder() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let mut child = closure(
        2,
        BindingOwner::Closure(CallableId(1)),
        TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
        Type::Number,
    );
    child.dependencies = vec![CallableId(0)];
    let mut parent = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(child))],
            Some(lexical(2, vec![], Type::Number)),
        ),
        Type::Number,
    );
    parent.dependencies = vec![CallableId(0)];
    let program = program(vec![
        statement(TypedStatementKind::Closure(first)),
        statement(TypedStatementKind::Closure(parent)),
        statement(TypedStatementKind::Return(Some(lexical(
            1,
            vec![],
            Type::Number,
        )))),
    ]);
    let bytes = Compiler::new().compile(&program).unwrap();
    for (body, target) in [(0, 2), (2, 3), (3, 1)] {
        assert!(operators(&bytes, body).iter().any(
            |op| matches!(op, Operator::Call { function_index } if *function_index == target)
        ));
    }
    let mut forged = program.clone();
    let TypedStatementKind::Closure(parent) = &mut forged.functions[0].body[1].kind else {
        panic!()
    };
    parent.dependencies.clear();
    assert_invalid(&forged, "invalid callable dependency", span());
}

#[test]
fn forged_identity_owner_and_capture_metadata_are_rejected_even_when_unused() {
    let good = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    for (mut declaration, reason) in [
        (good.clone(), "invalid callable identity"),
        (good.clone(), "invalid closure ownership"),
        (good.clone(), "invalid capture metadata"),
    ] {
        match reason {
            "invalid callable identity" => declaration.id = CallableId(8),
            "invalid closure ownership" => declaration.owner = BindingOwner::Closure(CallableId(0)),
            _ => declaration.captures.push(TypedCapture {
                owner: BindingOwner::Function(FunctionId(0)),
                variable: VariableId(0),
                ty: Type::Number,
                mutability: CaptureMutability::Mutable,
            }),
        }
        let p = program(vec![
            statement(TypedStatementKind::Return(Some(number("1")))),
            statement(TypedStatementKind::Closure(declaration)),
        ]);
        assert_invalid(&p, reason, span());
    }
    let duplicate = program(vec![
        statement(TypedStatementKind::Closure(good.clone())),
        statement(TypedStatementKind::Closure(good)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&duplicate, "invalid callable identity", span());
}

#[test]
fn lexical_calls_require_declaration_time_scope_and_canonical_exact_dependencies() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let before = program(vec![
        statement(TypedStatementKind::Expression(lexical(
            0,
            vec![],
            Type::Number,
        ))),
        statement(TypedStatementKind::Closure(first.clone())),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&before, "invalid callable call", span());
    let escaped = program(vec![
        statement(TypedStatementKind::If {
            cond: Box::new(TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            }),
            then: vec![statement(TypedStatementKind::Closure(first.clone()))],
            else_: None,
        }),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![],
            Type::Number,
        )))),
    ]);
    assert_invalid(&escaped, "invalid callable call", span());
    let self_call = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
        Type::Number,
    );
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(self_call)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid callable call",
        span(),
    );
    for dependencies in [
        vec![],
        vec![CallableId(0), CallableId(0)],
        vec![CallableId(1)],
        vec![CallableId(77)],
    ] {
        let mut second = closure(
            1,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
            Type::Number,
        );
        second.dependencies = dependencies;
        assert_invalid(
            &program(vec![
                statement(TypedStatementKind::Closure(first.clone())),
                statement(TypedStatementKind::Closure(second)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]),
            "invalid callable dependency",
            span(),
        );
    }
    let mut unused = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    unused.dependencies = vec![CallableId(0)];
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(first)),
            statement(TypedStatementKind::Closure(unused)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid callable dependency",
        span(),
    );
}

#[test]
fn nested_helpers_plan_locals_and_scratch_without_polluting_the_creator() {
    let array = TypedExpr {
        kind: TypedExprKind::Array(vec![number("3")]),
        ty: Type::NumberArray.into(),
    };
    let nested = closure(
        1,
        BindingOwner::Closure(CallableId(0)),
        block(
            vec![statement(TypedStatementKind::Let {
                id: VariableId(0),
                name: "array".into(),
                ty: Type::NumberArray,
                value: array,
            })],
            Some(number("9")),
        ),
        Type::Number,
    );
    let outer = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(nested))],
            Some(lexical(1, vec![], Type::Number)),
        ),
        Type::Number,
    );
    let bytes = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(outer)),
            statement(TypedStatementKind::Return(Some(lexical(
                0,
                vec![],
                Type::Number,
            )))),
        ]))
        .unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let bodies: Vec<_> = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 10); // user, seven memory helpers, enclosing and nested helpers
    assert_eq!(bodies[0].get_locals_reader().unwrap().get_count(), 0);
    assert_eq!(bodies[8].get_locals_reader().unwrap().get_count(), 0);
    assert_eq!(
        bodies[9]
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
        [(1, wasmparser::ValType::I32), (1, wasmparser::ValType::I32)]
    );
    assert!(
        operators(&bytes, 8)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 9 }))
    );
    assert!(
        operators(&bytes, 9)
            .iter()
            .any(|op| matches!(op, Operator::LocalSet { local_index: 0 }))
    );
    assert!(
        operators(&bytes, 9)
            .iter()
            .any(|op| matches!(op, Operator::LocalSet { local_index: 1 }))
    );
}

#[test]
fn body_discovery_appends_helpers_after_memory_remainder_and_range_helpers() {
    use was_compiler::ast::BinaryOp;
    let remainder = TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(number("7")),
            op: BinaryOp::Remainder,
            right: Box::new(number("2")),
        },
        ty: Type::Number.into(),
    };
    let range = TypedExpr {
        kind: TypedExprKind::Range {
            start: Box::new(remainder),
            end: Box::new(number("3")),
            step: None,
            inclusive: true,
        },
        ty: Type::Range.into(),
    };
    let contains = TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(number("1")),
            op: BinaryOp::In,
            right: Box::new(range),
        },
        ty: Type::Bool.into(),
    };
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(contains)),
        Type::Bool,
    );
    let bytes = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(declaration)),
            statement(TypedStatementKind::Expression(lexical(
                0,
                vec![],
                Type::Bool,
            ))),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]))
        .unwrap();
    let calls: Vec<_> = operators(&bytes, 12)
        .iter()
        .filter_map(|op| match op {
            Operator::Call { function_index } => Some(*function_index),
            _ => None,
        })
        .collect();
    assert_eq!(calls, [8, 9, 11]); // remainder, range construction, membership
    assert!(
        operators(&bytes, 0)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 12 }))
    );
}

#[test]
fn unreachable_closure_tails_are_checked_against_the_signature() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Return(Some(number("7"))))],
            Some(TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool.into(),
            }),
        ),
        Type::Number,
    );
    let error = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(declaration)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]))
        .unwrap_err();
    assert!(
        matches!(error, CodegenError::TypeMismatch { function, expected: Type::Number, found: Type::Bool, span: actual } if function == "<closure>" && actual == span())
    );
}

#[test]
fn closure_signatures_validate_unused_handle_parameters_before_the_body() {
    for (ty, expected_calls) in [
        (Type::String, vec![2]),
        (Type::NumberArray, vec![3]),
        (Type::Range, vec![9]),
    ] {
        let mut declaration = closure(
            0,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(number("7"))),
            Type::Number,
        );
        declaration.params.push(TypedParam {
            id: VariableId(41),
            name: "unused".into(),
            ty,
            span: span(),
        });
        let bytes = Compiler::new()
            .compile(&program(vec![
                statement(TypedStatementKind::Closure(declaration)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]))
            .unwrap();
        let helper = if ty == Type::Range { 10 } else { 8 };
        let ops = operators(&bytes, helper);
        assert!(matches!(
            ops.first(),
            Some(Operator::LocalGet { local_index: 0 })
        ));
        assert_eq!(
            ops.iter()
                .filter_map(|op| match op {
                    Operator::Call { function_index } => Some(*function_index),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            expected_calls
        );
        assert!(ops.iter().any(|op| matches!(op, Operator::Drop)));
        let body = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|p| match p.unwrap() {
                Payload::CodeSectionEntry(body) => Some(body),
                _ => None,
            })
            .nth(helper)
            .unwrap();
        assert_eq!(body.get_locals_reader().unwrap().get_count(), 0);
    }
}

#[test]
fn every_helper_is_checked_with_independent_value_and_loop_contexts() {
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("invalid"))),
        Type::Number,
    );
    let error = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Return(Some(number("1")))),
            statement(TypedStatementKind::Closure(declaration.clone())),
        ]))
        .unwrap_err();
    assert!(
        matches!(error, CodegenError::InvalidNumberLiteral { function, .. } if function == "<closure>")
    );
    declaration.body = TypedClosureBody::Expression(Box::new(TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(8),
            name: "outer".into(),
        },
        ty: Type::Number.into(),
    }));
    let p = program(vec![
        statement(TypedStatementKind::Let {
            id: VariableId(8),
            name: "outer".into(),
            ty: Type::Number,
            value: number("1"),
        }),
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert!(matches!(
        Compiler::new().compile(&p),
        Err(CodegenError::UnknownVariable {
            id: VariableId(8),
            ..
        })
    ));
    for transfer in [
        TypedStatementKind::Break { target: LoopId(3) },
        TypedStatementKind::Continue { target: LoopId(3) },
    ] {
        let declaration = closure(
            0,
            BindingOwner::Function(FunctionId(0)),
            block(vec![statement(transfer)], None),
            Type::Void,
        );
        let p = program(vec![
            statement(TypedStatementKind::While {
                id: LoopId(3),
                cond: Box::new(TypedExpr {
                    kind: TypedExprKind::Bool(false),
                    ty: Type::Bool.into(),
                }),
                body: vec![statement(TypedStatementKind::Closure(declaration))],
            }),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]);
        assert!(matches!(
            Compiler::new().compile(&p),
            Err(CodegenError::InvalidLoopTarget {
                target: LoopId(3),
                ..
            })
        ));
    }
}

#[test]
fn lexical_call_checks_arity_result_and_all_arguments_after_divergence() {
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    declaration.params = vec![
        TypedParam {
            id: VariableId(31),
            name: "a".into(),
            ty: Type::Number,
            span: span(),
        },
        TypedParam {
            id: VariableId(55),
            name: "b".into(),
            ty: Type::Number,
            span: span(),
        },
    ];
    for call in [
        lexical(0, vec![], Type::Number),
        lexical(0, vec![number("1"), number("2")], Type::Bool),
    ] {
        assert_invalid(
            &program(vec![
                statement(TypedStatementKind::Closure(declaration.clone())),
                statement(TypedStatementKind::Expression(call)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]),
            "invalid callable call",
            span(),
        );
    }
    let divergent = TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![],
        },
        ty: ExprResult::Never,
    };
    let mut call = lexical(0, vec![divergent, number("invalid")], Type::Number);
    call.ty = ExprResult::Never;
    let p = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Expression(call)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert!(matches!(
        Compiler::new().compile(&p),
        Err(CodegenError::InvalidNumberLiteral { .. })
    ));
}

#[test]
fn closure_result_checks_returns_tails_fallthrough_and_never_only_bodies() {
    let divergent = TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![],
        },
        ty: ExprResult::Never,
    };
    for (body, result) in [
        (
            TypedClosureBody::Expression(Box::new(divergent)),
            Type::Number,
        ),
        (block(vec![], None), Type::Number),
    ] {
        let declaration = closure(0, BindingOwner::Function(FunctionId(0)), body, result);
        assert_invalid(
            &program(vec![
                statement(TypedStatementKind::Closure(declaration)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]),
            "invalid closure result",
            span(),
        );
    }
    for body in [
        block(
            vec![statement(TypedStatementKind::Return(Some(number("7"))))],
            None,
        ),
        block(
            vec![statement(TypedStatementKind::Return(Some(number("7"))))],
            Some(number("9")),
        ),
    ] {
        let declaration = closure(0, BindingOwner::Function(FunctionId(0)), body, Type::Number);
        let bytes = Compiler::new()
            .compile(&program(vec![
                statement(TypedStatementKind::Closure(declaration)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]))
            .unwrap();
        assert!(
            operators(&bytes, 1)
                .iter()
                .any(|op| matches!(op, Operator::Return))
        );
    }
    let void = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Loop {
                id: LoopId(0),
                body: vec![],
            })],
            None,
        ),
        Type::Void,
    );
    let bytes = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(void)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]))
        .unwrap();
    Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn integrated_source_retains_named_calls_normalized_defaults_contracts_and_owner_local_loops() {
    use was_compiler::{CompilerOptions, ExportOptions, OptimizationLevel};
    for source in [
        "fn named(a: number, b: number = a + 1) -> number requires a >= 0 { return a + b; } fn main() -> number { let f = (x: number) => named(a: x); return f(3); }",
        "fn main() -> number { let f = (n: number) => { let a = [1, 2]; let sum = 0; for x in a { sum += x; } for x in 1..=n { if x == 2 { continue; } sum += x; } sum }; return f(3); }",
        "fn main() -> number { let a = () => 1; let b = () => { let c = () => a(); c() }; return { let a = () => 2; b() }; }",
        "fn main() -> number { let f = () => loop { break { let g = () => 7; g() }; }; return f(); }",
        "fn main() -> number { let f = () => match true { true => { let g = () => 7; g() }, false => 9 }; return f(); }",
        "fn named(x: number) -> number { return x; } fn main() -> number { let f = (x: number) => x |> named; return f(3); }",
    ] {
        let typed = Compiler::new().analyze(source).unwrap();
        let baseline = Compiler::new().compile(&typed).unwrap();
        Validator::new().validate_all(&baseline).unwrap();
        for optimization in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let compiler = Compiler::with_options(CompilerOptions {
                optimization_level: optimization,
            });
            assert_eq!(compiler.compile(&typed).unwrap(), baseline, "{source}");
        }
        let hidden =
            was_compiler::codegen::compile_with_exports(&typed, &ExportOptions::None).unwrap();
        Validator::new().validate_all(&hidden).unwrap();
        let code_count = |bytes: &[u8]| {
            Parser::new(0)
                .parse_all(bytes)
                .filter(|p| matches!(p, Ok(Payload::CodeSectionEntry(_))))
                .count()
        };
        assert_eq!(code_count(&baseline), code_count(&hidden));
    }
}

#[test]
fn lexical_permission_is_not_global_existence_and_calls_keep_their_own_spans() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    let call_span = Span::new(101, 107);
    let call = TypedExpr {
        kind: TypedExprKind::LexicalCall {
            callable: CallableId(0),
            args: vec![],
            span: call_span,
        },
        ty: Type::Number.into(),
    };
    p.functions.push(TypedFunction {
        id: FunctionId(1),
        name: "other".into(),
        params: vec![],
        return_type: Type::Number,
        contracts: vec![],
        body: vec![statement(TypedStatementKind::Return(Some(call)))],
        span: span(),
    });
    assert_invalid(&p, "invalid callable call", call_span);
    let unknown = TypedExpr {
        kind: TypedExprKind::LexicalCall {
            callable: CallableId(77),
            args: vec![],
            span: call_span,
        },
        ty: Type::Number.into(),
    };
    assert_invalid(
        &program(vec![statement(TypedStatementKind::Return(Some(unknown)))]),
        "invalid callable call",
        call_span,
    );
}

#[test]
fn malformed_dependency_order_and_nested_ownership_have_stable_declaration_origins() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let second = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("8"))),
        Type::Number,
    );
    let mut third = closure(
        2,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("9"))),
        Type::Number,
    );
    third.span = Span::new(110, 149);
    third.dependencies = vec![CallableId(1), CallableId(0)];
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(first)),
            statement(TypedStatementKind::Closure(second)),
            statement(TypedStatementKind::Closure(third)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid callable dependency",
        Span::new(110, 149),
    );
    let mut nested = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    nested.span = Span::new(201, 231);
    let outer = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(nested))],
            Some(number("1")),
        ),
        Type::Number,
    );
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(outer)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid closure ownership",
        Span::new(201, 231),
    );
}

#[test]
fn lexical_calls_are_rejected_by_contract_and_default_whitelists() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(TypedExpr {
            kind: TypedExprKind::Bool(true),
            ty: Type::Bool.into(),
        })),
        Type::Bool,
    );
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    p.functions[0].contracts.push(TypedContract {
        value: lexical(0, vec![], Type::Bool),
        span: Span::new(81, 93),
    });
    assert!(
        matches!(Compiler::new().compile(&p), Err(CodegenError::InvalidContractExpression { reason: "contract expression is outside the pure-expression whitelist", span: actual }) if actual == Span::new(81, 93))
    );
    let mut typed = Compiler::new().analyze("fn named(a: number = 1) -> number { return a; } fn main() -> number { let f = () => 7; return named(); }").unwrap();
    let TypedStatementKind::Return(Some(TypedExpr {
        kind: TypedExprKind::NormalizedCall(sequence),
        ..
    })) = &mut typed.functions[1].body[1].kind
    else {
        panic!("normalized default")
    };
    sequence.bindings[0].value = lexical(0, vec![], Type::Number);
    assert!(matches!(
        Compiler::new().compile(&typed),
        Err(CodegenError::InvalidNormalizedCall {
            reason: "default initializer is outside the pure-expression whitelist",
            ..
        })
    ));
}

#[test]
fn helper_signatures_have_only_explicit_parameters_and_sparse_ids_are_owner_local() {
    let read = TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(40),
            name: "x".into(),
        },
        ty: Type::Number.into(),
    };
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(read)),
        Type::Number,
    );
    declaration.params = vec![
        TypedParam {
            id: VariableId(40),
            name: "x".into(),
            ty: Type::Number,
            span: span(),
        },
        TypedParam {
            id: VariableId(90),
            name: "flag".into(),
            ty: Type::Bool,
            span: span(),
        },
    ];
    let p = program(vec![
        statement(TypedStatementKind::Let {
            id: VariableId(40),
            name: "creator".into(),
            ty: Type::Number,
            value: number("3"),
        }),
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![
                number("8"),
                TypedExpr {
                    kind: TypedExprKind::Bool(true),
                    ty: Type::Bool.into(),
                },
            ],
            Type::Number,
        )))),
    ]);
    let bytes = Compiler::new().compile(&p).unwrap();
    let signatures: Vec<_> = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::TypeSection(types) => Some(types),
            _ => None,
        })
        .flat_map(|types| types.into_iter_err_on_gc_types().map(Result::unwrap))
        .collect();
    assert_eq!(signatures.len(), 2);
    assert_eq!(
        signatures[1].params(),
        [wasmparser::ValType::F64, wasmparser::ValType::I32]
    );
    assert_eq!(signatures[1].results(), [wasmparser::ValType::F64]);
    assert!(matches!(
        operators(&bytes, 1).as_slice(),
        [Operator::LocalGet { local_index: 0 }, Operator::End]
    ));
    let ops = operators(&bytes, 0);
    let call_position = ops
        .iter()
        .position(|op| matches!(op, Operator::Call { function_index: 1 }))
        .unwrap();
    assert!(
        matches!(&ops[call_position - 2], Operator::F64Const { value } if value.bits() == 8f64.to_bits())
    );
    assert!(matches!(
        &ops[call_position - 1],
        Operator::I32Const { value: 1 }
    ));
}

#[test]
fn every_concrete_body_result_and_void_compiles_without_exporting_helpers() {
    for body in [
        "1",
        "true",
        "\"é\"",
        "[1, 2]",
        "1..3",
        "{}",
        "{ return; }",
        "{ if true { return 1; } 2 }",
        "loop { break [1]; }",
        "if true { \"one\" } else { \"two\" }",
    ] {
        let source = format!("fn main() {{ let f = () => {body}; f(); }}");
        let typed = Compiler::new().analyze(&source).unwrap();
        let bytes = Compiler::new().compile(&typed).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        for payload in Parser::new(0).parse_all(&bytes) {
            if let Payload::ExportSection(exports) = payload.unwrap() {
                let names: Vec<_> = exports.into_iter().map(|e| e.unwrap().name).collect();
                assert!(
                    names
                        .iter()
                        .all(|name| ["main", "memory", "__alloc"].contains(name))
                );
            }
        }
    }
}

#[test]
fn helpers_reject_duplicate_parameters_uninitialized_locals_and_read_only_writes() {
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    declaration.params = vec![
        TypedParam {
            id: VariableId(3),
            name: "x".into(),
            ty: Type::Number,
            span: span(),
        },
        TypedParam {
            id: VariableId(3),
            name: "y".into(),
            ty: Type::Number,
            span: span(),
        },
    ];
    let fixture = |declaration| {
        program(vec![
            statement(TypedStatementKind::Closure(declaration)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ])
    };
    assert!(
        matches!(Compiler::new().compile(&fixture(declaration.clone())), Err(CodegenError::DuplicateVariableId { function, id: VariableId(3), .. }) if function == "<closure>")
    );
    declaration.params.truncate(1);
    declaration.params[0].ty = Type::Void;
    assert!(
        matches!(Compiler::new().compile(&fixture(declaration)), Err(CodegenError::VoidParameter { function, index: 0 }) if function == "<closure>")
    );
    let read = TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(8),
            name: "later".into(),
        },
        ty: Type::Number.into(),
    };
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![
                statement(TypedStatementKind::Expression(read)),
                statement(TypedStatementKind::Let {
                    id: VariableId(8),
                    name: "later".into(),
                    ty: Type::Number,
                    value: number("1"),
                }),
            ],
            Some(number("7")),
        ),
        Type::Number,
    );
    assert!(
        matches!(Compiler::new().compile(&fixture(declaration)), Err(CodegenError::UninitializedVariable { function, id: VariableId(8), .. }) if function == "<closure>")
    );
    let mut typed = Compiler::new()
        .analyze("fn main() { let f = () => { for x in [1] { let y = x; } }; }")
        .unwrap();
    let TypedStatementKind::Closure(declaration) = &mut typed.functions[0].body[0].kind else {
        panic!()
    };
    let TypedClosureBody::Block(block) = &mut declaration.body else {
        panic!()
    };
    let TypedStatementKind::For { variable, body, .. } = &mut block.statements[0].kind else {
        panic!()
    };
    body.push(statement(TypedStatementKind::Assignment {
        target: TypedExpr {
            kind: TypedExprKind::Identifier {
                id: *variable,
                name: "x".into(),
            },
            ty: Type::Number.into(),
        },
        value: number("2"),
    }));
    assert!(
        matches!(Compiler::new().compile(&typed), Err(CodegenError::ReadOnlyVariable { function, .. }) if function == "<closure>")
    );
}

#[test]
fn helper_normalized_bindings_keep_their_final_call_only_read_permission() {
    let mut typed = Compiler::new()
        .analyze(
            "fn named(a: number = 1) -> number { return a; } fn main() { let f = () => named(); }",
        )
        .unwrap();
    let TypedStatementKind::Closure(declaration) = &mut typed.functions[1].body[0].kind else {
        panic!()
    };
    let old = std::mem::replace(&mut declaration.body, block(vec![], None));
    let TypedClosureBody::Expression(call) = old else {
        panic!()
    };
    let TypedExprKind::NormalizedCall(sequence) = &call.kind else {
        panic!()
    };
    let id = sequence.bindings[0].id;
    declaration.body = block(
        vec![statement(TypedStatementKind::Expression(*call))],
        Some(TypedExpr {
            kind: TypedExprKind::Identifier {
                id,
                name: "forged".into(),
            },
            ty: Type::Number.into(),
        }),
    );
    assert!(matches!(
        Compiler::new().compile(&typed),
        Err(CodegenError::InvalidNormalizedCall {
            reason: "synthetic binding is only readable by its final call",
            ..
        })
    ));
}

#[test]
fn public_typed_callable_compiles_to_unexported_direct_helper_without_memory() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let program = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![],
            Type::Number,
        )))),
    ]);
    let original = program.clone();
    let bytes = Compiler::new().compile(&program).unwrap();
    assert_eq!(program, original);
    assert!(
        operators(&bytes, 0)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 1 }))
    );
    assert!(
        operators(&bytes, 1)
            .iter()
            .any(|op| matches!(op, Operator::F64Const { value } if value.bits() == 7f64.to_bits()))
    );
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::MemorySection(_)
            | Payload::GlobalSection(_)
            | Payload::DataSection(_)
            | Payload::TableSection(_) => panic!("callable-only storage"),
            Payload::ExportSection(exports) => assert_eq!(
                exports
                    .into_iter()
                    .map(|e| e.unwrap().name)
                    .collect::<Vec<_>>(),
                ["main"]
            ),
            _ => {}
        }
    }
}
