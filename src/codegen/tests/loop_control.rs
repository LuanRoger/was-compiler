use super::*;
use crate::exports::ExportOptions;

fn loop_with_id(
    id: usize,
    cond: TypedExpr,
    body: Vec<TypedStatement>,
    span: Span,
) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::While {
            id: LoopId(id),
            cond: Box::new(cond),
            body,
        },
        span,
    }
}

fn break_to(target: usize, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::Break {
            target: LoopId(target),
        },
        span,
    }
}

fn continue_to(target: usize, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::Continue {
            target: LoopId(target),
        },
        span,
    }
}

fn short_circuit_condition() -> TypedExpr {
    bool_binary(
        boolean(),
        BinaryOp::And,
        bool_binary(boolean(), BinaryOp::Or, boolean()),
    )
}

#[test]
fn nested_transfers_decode_relative_depths_with_statement_and_expression_ifs() {
    let span = Span::new(12, 34);
    let inner = loop_with_id(
        77,
        short_circuit_condition(),
        vec![conditional(
            boolean(),
            vec![conditional(
                short_circuit_condition(),
                vec![break_to(77, span)],
                Some(vec![continue_to(77, span)]),
                span,
            )],
            Some(vec![continue_to(77, span)]),
            span,
        )],
        span,
    );
    let outer = loop_with_id(
        900,
        short_circuit_condition(),
        vec![
            conditional(
                short_circuit_condition(),
                vec![
                    conditional(
                        boolean(),
                        vec![break_to(900, span)],
                        Some(vec![continue_to(900, span)]),
                        span,
                    ),
                    inner,
                    continue_to(900, span),
                ],
                Some(vec![break_to(900, span)]),
                span,
            ),
            break_to(900, span),
        ],
        span,
    );
    let program = local_program(vec![outer]);
    let original = program.clone();
    let bytes = compile(&program).unwrap();
    let module = decode(&bytes);
    let body = &module.bodies[0];

    assert_eq!(program, original);
    assert_eq!(compile(&program).unwrap(), bytes);
    assert_eq!(
        body.iter()
            .filter_map(|op| match op {
                DecodedInstruction::BrIf(depth) => Some(*depth),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![1, 1]
    );
    assert_eq!(
        body.iter()
            .filter_map(|op| match op {
                DecodedInstruction::Br(depth) => Some(*depth),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![3, 2, 3, 2, 1, 0, 1, 2, 1, 0]
    );
    assert!(!body.contains(&DecodedInstruction::Unreachable));
    assert!(module.locals[0].is_empty());
    assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
}

#[test]
fn forged_targets_reject_unknown_inactive_outer_and_cross_function_loops_with_spans() {
    let span = Span::new(40, 60);
    let outer = Span::new(0, 100);

    for transfer in [break_to, continue_to] {
        let cases = vec![
            vec![transfer(9, span)],
            vec![loop_with_id(9, boolean(), vec![], outer), transfer(9, span)],
            vec![loop_with_id(9, boolean(), vec![transfer(99, span)], outer)],
            vec![loop_with_id(
                9,
                boolean(),
                vec![loop_with_id(10, boolean(), vec![transfer(9, span)], outer)],
                outer,
            )],
            vec![loop_with_id(
                9,
                boolean(),
                vec![
                    loop_with_id(10, boolean(), vec![], outer),
                    transfer(10, span),
                ],
                outer,
            )],
        ];

        for (body, target) in cases.into_iter().zip([9, 9, 99, 9, 10]) {
            let error = compile(&local_program(body)).unwrap_err();
            assert_eq!(error.span(), Some(span));
            assert_eq!(
                error,
                CodegenError::InvalidLoopTarget {
                    function: "bad".into(),
                    target: LoopId(target),
                    span,
                }
            );
        }

        let mut first = user(0, "first", &[], Type::Void);
        first.body = vec![loop_with_id(9, boolean(), vec![], outer)];
        let mut second = user(1, "second", &[], Type::Void);
        second.body = vec![transfer(9, span)];
        let error = compile(&TypedProgram {
            functions: vec![first, second],
            host_functions: vec![],
        })
        .unwrap_err();
        assert_eq!(
            error,
            CodegenError::InvalidLoopTarget {
                function: "second".into(),
                target: LoopId(9),
                span
            }
        );
        assert_eq!(error.span(), Some(span));
    }
}

#[test]
fn duplicate_loop_ids_are_function_wide_even_in_nested_unreachable_and_sibling_arms() {
    let first = Span::new(1, 10);
    let repeated = Span::new(40, 60);
    let empty = loop_with_id(usize::MAX, boolean(), vec![], first);
    let duplicate = loop_with_id(usize::MAX, boolean(), vec![], repeated);
    let cases = vec![
        vec![empty.clone(), duplicate.clone()],
        vec![loop_with_id(
            usize::MAX,
            boolean(),
            vec![duplicate.clone()],
            first,
        )],
        vec![empty.clone(), returned(None, first), duplicate.clone()],
        vec![conditional(
            boolean(),
            vec![empty.clone()],
            Some(vec![duplicate.clone()]),
            first,
        )],
        vec![loop_with_id(
            usize::MAX,
            boolean(),
            vec![break_to(usize::MAX, first), duplicate],
            first,
        )],
    ];

    for body in cases {
        let error = compile(&local_program(body)).unwrap_err();
        assert_eq!(
            error,
            CodegenError::DuplicateLoopId {
                function: "bad".into(),
                id: LoopId(usize::MAX),
                span: repeated
            }
        );
        assert_eq!(error.span(), Some(repeated));
    }

    let mut first_function = user(0, "first", &[], Type::Void);
    first_function.body = vec![empty.clone()];
    let mut second_function = user(1, "second", &[], Type::Void);
    second_function.body = vec![empty];
    decode(
        &compile(&TypedProgram {
            functions: vec![first_function, second_function],
            host_functions: vec![],
        })
        .unwrap(),
    );
}

#[test]
fn transfers_do_not_make_unreachable_returns_count_as_returning_if_arms() {
    let span = Span::new(12, 34);

    for transfer in [break_to, continue_to] {
        let body = vec![loop_with_id(
            7,
            boolean(),
            vec![conditional(
                boolean(),
                vec![transfer(7, span), returned(Some(number()), span)],
                Some(vec![returned(Some(number()), span)]),
                span,
            )],
            span,
        )];
        let mut program = local_program(body);
        program.functions[0].return_type = Type::Number;

        assert_eq!(
            compile(&program),
            Err(CodegenError::MissingReturnValue {
                function: "bad".into(),
                return_type: Type::Number
            })
        );
        program.functions[0]
            .body
            .push(returned(Some(number_literal("2")), span));
        let module = decode(&compile(&program).unwrap());
        assert!(!module.bodies[0].contains(&DecodedInstruction::Unreachable));
        assert_eq!(
            module.bodies[0]
                .iter()
                .filter(|op| **op == DecodedInstruction::Return)
                .count(),
            3
        );
    }
}

#[test]
fn hidden_skipped_and_unreachable_loop_bodies_and_transfers_are_all_checked() {
    let span = Span::new(40, 60);
    let outer = Span::new(0, 100);
    let invalid = TypedStatement {
        kind: TypedStatementKind::Expression(number_literal("NaN")),
        span,
    };
    let false_cond = TypedExpr {
        kind: TypedExprKind::Bool(false),
        ty: Type::Bool,
    };

    for transfer in [break_to, continue_to] {
        let cases = vec![
            vec![returned(None, outer), transfer(99, span)],
            vec![conditional(
                false_cond.clone(),
                vec![transfer(99, span)],
                None,
                outer,
            )],
            vec![loop_with_id(
                7,
                false_cond.clone(),
                vec![transfer(99, span)],
                outer,
            )],
            vec![loop_with_id(
                7,
                boolean(),
                vec![transfer(7, outer), transfer(99, span)],
                outer,
            )],
        ];

        for body in cases {
            let program = local_program(body);
            for exports in [ExportOptions::All, ExportOptions::None] {
                assert_eq!(
                    super::super::compile_with_exports(&program, &exports),
                    Err(CodegenError::InvalidLoopTarget {
                        function: "bad".into(),
                        target: LoopId(99),
                        span
                    })
                );
            }
        }

        let program = local_program(vec![loop_with_id(
            7,
            false_cond.clone(),
            vec![transfer(7, outer), invalid.clone()],
            outer,
        )]);
        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".into(),
                literal: "NaN".into(),
                span
            })
        );
    }

    let mut forged_rhs = boolean();
    forged_rhs.kind = TypedExprKind::Number("NaN".into());
    let cond = bool_binary(false_cond, BinaryOp::And, forged_rhs);
    let program = local_program(vec![loop_with_id(7, cond, vec![break_to(7, outer)], span)]);
    assert_eq!(
        compile(&program),
        Err(CodegenError::TypeMismatch {
            function: "bad".into(),
            expected: Type::Number,
            found: Type::Bool,
            span
        })
    );
}

#[test]
fn transfers_preserve_planning_of_unreachable_locals_temporaries_strings_and_helpers() {
    let span = Span::new(12, 34);
    let array = TypedExpr {
        kind: TypedExprKind::Array(vec![number()]),
        ty: Type::NumberArray,
    };
    let string = TypedExpr {
        kind: TypedExprKind::String("after".into()),
        ty: Type::String,
    };
    let following = vec![
        declaration(99, Type::NumberArray, array, span),
        TypedStatement {
            kind: TypedStatementKind::Expression(string),
            span,
        },
        TypedStatement {
            kind: TypedStatementKind::Expression(binary(number(), BinaryOp::Remainder, number())),
            span,
        },
    ];
    let helper_bodies = |bytes: &[u8]| {
        Parser::new(0)
            .parse_all(bytes)
            .filter_map(|payload| match payload.unwrap() {
                Payload::CodeSectionEntry(body) => {
                    let range = body.range();
                    let start = usize::try_from(range.start).unwrap();
                    let end = usize::try_from(range.end).unwrap();
                    Some(bytes[start..end].to_vec())
                }
                _ => None,
            })
            .skip(1)
            .collect::<Vec<_>>()
    };
    let baseline = local_program(vec![loop_with_id(7, boolean(), following.clone(), span)]);
    let baseline_bytes = compile(&baseline).unwrap();
    let baseline = decode(&baseline_bytes);

    for transfer in [break_to, continue_to] {
        let mut body = vec![transfer(7, span)];
        body.extend(following.clone());
        let program = local_program(vec![loop_with_id(7, boolean(), body, span)]);
        let bytes = compile(&program).unwrap();
        let module = decode(&bytes);
        assert_eq!(module.signatures, baseline.signatures);
        assert_eq!(module.functions, baseline.functions);
        assert_eq!(module.exports, baseline.exports);
        assert_eq!(module.locals, baseline.locals);
        assert_eq!(module.locals[0], vec![ValType::I32, ValType::I32]);
        assert_eq!(module.functions.len(), 9);
        assert_eq!(helper_bodies(&bytes), helper_bodies(&baseline_bytes));
        let mut expected = baseline.bodies[0].clone();
        expected.insert(
            5,
            DecodedInstruction::Br(
                if matches!(transfer(7, span).kind, TypedStatementKind::Break { .. }) {
                    1
                } else {
                    0
                },
            ),
        );
        assert_eq!(module.bodies[0], expected);
    }
}

#[test]
fn partial_transfer_paths_do_not_turn_later_returns_into_guaranteed_if_returns() {
    let span = Span::new(12, 34);

    for transfer in [break_to, continue_to] {
        let partial = conditional(boolean(), vec![transfer(7, span)], None, span);
        let conditional_return = conditional(
            boolean(),
            vec![partial, returned(Some(number()), span)],
            Some(vec![returned(Some(number()), span)]),
            span,
        );
        let mut program = local_program(vec![
            loop_with_id(7, boolean(), vec![conditional_return], span),
            returned(Some(number()), span),
        ]);
        program.functions[0].return_type = Type::Number;
        let module = decode(&compile(&program).unwrap());
        assert!(!module.bodies[0].contains(&DecodedInstruction::Unreachable));
    }
}

#[test]
fn returns_before_unreachable_transfers_keep_the_existing_if_unreachable_marker() {
    let span = Span::new(12, 34);
    let program = local_program(vec![loop_with_id(
        7,
        boolean(),
        vec![conditional(
            boolean(),
            vec![returned(None, span), break_to(7, span)],
            Some(vec![returned(None, span), continue_to(7, span)]),
            span,
        )],
        span,
    )]);
    let module = decode(&compile(&program).unwrap());
    assert!(module.bodies[0].windows(3).any(|ops| ops
        == [
            DecodedInstruction::End,
            DecodedInstruction::Unreachable,
            DecodedInstruction::Br(0),
        ]));
}

#[test]
fn valid_unreachable_transfers_and_following_statements_are_still_emitted() {
    let span = Span::new(12, 34);
    let program = local_program(vec![loop_with_id(
        7,
        boolean(),
        vec![
            returned(None, span),
            break_to(7, span),
            continue_to(7, span),
            TypedStatement {
                kind: TypedStatementKind::Expression(number_literal("42")),
                span,
            },
        ],
        span,
    )]);

    assert_eq!(
        decode(&compile(&program).unwrap()).bodies[0],
        vec![
            DecodedInstruction::Block(BlockType::Empty),
            DecodedInstruction::Loop(BlockType::Empty),
            DecodedInstruction::I32Const(1),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::BrIf(1),
            DecodedInstruction::Return,
            DecodedInstruction::Br(1),
            DecodedInstruction::Br(0),
            DecodedInstruction::F64Const(42.0),
            DecodedInstruction::Drop,
            DecodedInstruction::Br(0),
            DecodedInstruction::End,
            DecodedInstruction::End,
            DecodedInstruction::End,
        ]
    );
}
