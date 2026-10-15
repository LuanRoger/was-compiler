use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::{BinaryOp, MatchPattern, MatchPatternKind},
    codegen,
    tokens::Span,
    typed_ast::{
        CallBindingOrigin, ExprResult, FunctionId, FunctionRef, LoopId, TypedCallBinding,
        TypedContract, TypedExpr, TypedExprKind, TypedFunction, TypedMatchArm, TypedNormalizedCall,
        TypedParam, TypedProgram, TypedStatement, TypedStatementKind, TypedValueBlock, VariableId,
    },
};
use wasmparser::{BlockType, FunctionBody, Operator, Parser, Payload, ValType, Validator};

#[path = "../../../tests/support/process.rs"]
mod test_process;

fn origin() -> Span {
    Span::new(31, 97)
}

fn pattern_span() -> Span {
    Span::new(40, 45)
}

fn arm_span() -> Span {
    Span::new(40, 61)
}

fn expr(kind: TypedExprKind, ty: impl Into<ExprResult>) -> TypedExpr {
    TypedExpr {
        kind,
        ty: ty.into(),
    }
}

fn number(value: &str) -> TypedExpr {
    expr(TypedExprKind::Number(value.into()), Type::Number)
}

fn boolean(value: bool) -> TypedExpr {
    expr(TypedExprKind::Bool(value), Type::Bool)
}

fn string(value: &str) -> TypedExpr {
    expr(TypedExprKind::String(value.into()), Type::String)
}

fn read(id: usize, ty: Type) -> TypedExpr {
    expr(
        TypedExprKind::Identifier {
            id: VariableId(id),
            name: "ignored".into(),
        },
        ty,
    )
}

fn arm(kind: MatchPatternKind, body: TypedExpr) -> TypedMatchArm {
    TypedMatchArm {
        pattern: MatchPattern {
            kind,
            span: pattern_span(),
        },
        body,
        span: arm_span(),
    }
}

fn wildcard(body: TypedExpr) -> TypedMatchArm {
    arm(MatchPatternKind::Wildcard, body)
}

fn match_value(
    scrutinee: TypedExpr,
    arms: Vec<TypedMatchArm>,
    ty: impl Into<ExprResult>,
) -> TypedExpr {
    expr(
        TypedExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        },
        ty,
    )
}

fn statement(kind: TypedStatementKind) -> TypedStatement {
    TypedStatement {
        kind,
        span: origin(),
    }
}

fn program(result: Type, body: Vec<TypedStatement>) -> TypedProgram {
    TypedProgram {
        host_functions: vec![],
        functions: vec![TypedFunction {
            id: FunctionId(0),
            name: "main".into(),
            params: vec![],
            return_type: result,
            contracts: vec![],
            body,
            span: origin(),
        }],
    }
}

fn returning(value: TypedExpr, result: Type) -> TypedProgram {
    program(
        result,
        vec![statement(TypedStatementKind::Return(Some(value)))],
    )
}

fn discarded(scrutinee: TypedExpr, arms: Vec<TypedMatchArm>) -> TypedProgram {
    program(
        Type::Void,
        vec![statement(TypedStatementKind::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        })],
    )
}

fn divergent(id: usize) -> TypedExpr {
    expr(
        TypedExprKind::Loop {
            id: LoopId(id),
            body: vec![],
        },
        ExprResult::Never,
    )
}

fn block(
    statements: Vec<TypedStatement>,
    tail: Option<TypedExpr>,
    ty: impl Into<ExprResult>,
) -> TypedExpr {
    expr(
        TypedExprKind::Block(TypedValueBlock {
            statements,
            tail: tail.map(Box::new),
            span: arm_span(),
        }),
        ty,
    )
}

fn compiled(program: &TypedProgram) -> Vec<u8> {
    let bytes = codegen::compile(program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(program).unwrap(), bytes);
    }
    bytes
}

fn body(bytes: &[u8]) -> FunctionBody<'_> {
    Parser::new(0)
        .parse_all(bytes)
        .find_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .unwrap()
}

fn ops(bytes: &[u8]) -> Vec<Operator<'_>> {
    body(bytes)
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

fn locals(bytes: &[u8]) -> Vec<ValType> {
    body(bytes)
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .flat_map(|entry| {
            let (count, ty) = entry.unwrap();
            std::iter::repeat_n(ty, count as usize)
        })
        .collect()
}

fn rejected(program: &TypedProgram) -> CodegenError {
    let error = codegen::compile_with_exports(program, &ExportOptions::None).unwrap_err();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::None);
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(program).unwrap_err(), error);
    }
    error
}

#[test]
fn pattern_validation_retains_pattern_and_whole_match_origins() {
    let cases = vec![
        (number("1"), vec![], origin()),
        (
            number("1"),
            vec![arm(MatchPatternKind::Number("1".into()), number("2"))],
            origin(),
        ),
        (
            number("1"),
            vec![wildcard(number("2")), wildcard(number("3"))],
            pattern_span(),
        ),
        (
            number("1"),
            vec![
                arm(MatchPatternKind::Bool(true), number("2")),
                wildcard(number("3")),
            ],
            pattern_span(),
        ),
        (
            number("1"),
            vec![
                arm(MatchPatternKind::Number("--1".into()), number("2")),
                wildcard(number("3")),
            ],
            pattern_span(),
        ),
        (
            number("1"),
            vec![
                arm(
                    MatchPatternKind::Relational {
                        op: BinaryOp::Add,
                        value: "1".into(),
                    },
                    number("2"),
                ),
                wildcard(number("3")),
            ],
            pattern_span(),
        ),
        (
            number("1"),
            vec![
                arm(
                    MatchPatternKind::Interval {
                        start: "0".into(),
                        end: "NaN".into(),
                        inclusive: true,
                    },
                    number("2"),
                ),
                wildcard(number("3")),
            ],
            pattern_span(),
        ),
        (
            expr(TypedExprKind::Array(vec![]), Type::NumberArray),
            vec![wildcard(number("2"))],
            origin(),
        ),
        (
            boolean(true),
            vec![arm(MatchPatternKind::Bool(true), number("2"))],
            origin(),
        ),
        (
            divergent(0),
            vec![
                arm(MatchPatternKind::Number("1".into()), number("2")),
                arm(MatchPatternKind::String("a".into()), number("3")),
                wildcard(number("4")),
            ],
            pattern_span(),
        ),
        (
            divergent(0),
            vec![arm(MatchPatternKind::String("a".into()), number("3"))],
            origin(),
        ),
    ];
    for (scrutinee, arms, expected_span) in cases {
        let error = rejected(&discarded(scrutinee, arms));
        assert!(
            matches!(error, CodegenError::InvalidMatch { .. }),
            "{error}"
        );
        assert_eq!(error.span(), Some(expected_span));
        assert!(error.to_string().starts_with("invalid match: "));
    }
}

#[test]
fn value_joins_are_structural_even_for_shadowed_arms_and_never_scrutinees() {
    for scrutinee in [number("1"), divergent(0)] {
        let result = if scrutinee.ty == ExprResult::Never {
            ExprResult::Never
        } else {
            Type::Number.into()
        };
        let value = match_value(
            scrutinee,
            vec![
                arm(MatchPatternKind::Number("1".into()), number("2")),
                arm(MatchPatternKind::Number("1".into()), boolean(true)),
                wildcard(number("3")),
            ],
            result,
        );
        let error = rejected(&returning(value, Type::Number));
        assert!(matches!(
            error,
            CodegenError::TypeMismatch {
                expected: Type::Number,
                found: Type::Bool,
                ..
            }
        ));
        assert_eq!(error.span(), Some(arm_span()));
    }
}

#[test]
fn every_arm_is_checked_in_statement_value_and_unreachable_matches() {
    for unreachable in [false, true] {
        for value_context in [false, true] {
            let arms = vec![
                arm(MatchPatternKind::Bool(true), number("1")),
                arm(MatchPatternKind::Bool(true), read(999, Type::Number)),
                wildcard(number("2")),
            ];
            let mut program = if value_context {
                returning(match_value(boolean(true), arms, Type::Number), Type::Number)
            } else {
                discarded(boolean(true), arms)
            };
            if unreachable {
                program.functions[0]
                    .body
                    .insert(0, statement(TypedStatementKind::Expression(divergent(0))));
            }
            assert!(matches!(
                rejected(&program),
                CodegenError::UnknownVariable {
                    id: VariableId(999),
                    ..
                }
            ));
        }
    }
}

#[test]
fn numeric_chain_caches_once_and_uses_mvp_result_regions_and_fractional_comparisons() {
    let value = match_value(
        number("1.5"),
        vec![
            arm(
                MatchPatternKind::Relational {
                    op: BinaryOp::LessThan,
                    value: "-0".into(),
                },
                number("10"),
            ),
            arm(
                MatchPatternKind::Interval {
                    start: "1.25".into(),
                    end: "2.75".into(),
                    inclusive: false,
                },
                number("20"),
            ),
            arm(
                MatchPatternKind::Interval {
                    start: "3".into(),
                    end: "2".into(),
                    inclusive: true,
                },
                number("30"),
            ),
            wildcard(number("40")),
        ],
        Type::Number,
    );
    let bytes = compiled(&returning(value, Type::Number));
    assert_eq!(locals(&bytes), [ValType::F64]);
    let ops = ops(&bytes);
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::LocalSet { local_index: 0 }))
            .count(),
        1
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(
                op,
                Operator::If {
                    blockty: BlockType::Type(ValType::F64)
                }
            ))
            .count(),
        3
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::F64Ge))
            .count(),
        2
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::F64Lt))
            .count(),
        2
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::F64Le))
            .count(),
        1
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::I32And))
            .count(),
        2
    );
    assert!(
        !Parser::new(0)
            .parse_all(&bytes)
            .any(|payload| matches!(payload.unwrap(), Payload::MemorySection(_)))
    );
}

#[test]
fn boolean_coverage_without_wildcard_has_an_unreachable_residual_not_a_fake_result() {
    let bytes = compiled(&returning(
        match_value(
            boolean(false),
            vec![
                arm(MatchPatternKind::Bool(true), number("1")),
                arm(MatchPatternKind::Bool(false), number("2")),
            ],
            Type::Number,
        ),
        Type::Number,
    ));
    assert_eq!(locals(&bytes), [ValType::I32]);
    let ops = ops(&bytes);
    assert!(ops.windows(4).any(|ops| matches!(
        ops,
        [
            Operator::Else,
            Operator::Unreachable,
            Operator::End,
            Operator::End
        ]
    )));
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::I32Eqz))
            .count(),
        4
    );
}

#[test]
fn statement_match_discards_heterogeneous_tails_with_empty_regions() {
    let bytes = compiled(&discarded(
        number("0"),
        vec![
            arm(MatchPatternKind::Number("0".into()), string("zero")),
            arm(MatchPatternKind::Number("1".into()), boolean(true)),
            wildcard(expr(
                TypedExprKind::Array(vec![number("3")]),
                Type::NumberArray,
            )),
        ],
    ));
    assert_eq!(locals(&bytes), [ValType::F64, ValType::I32]);
    let ops = ops(&bytes);
    assert_eq!(
        ops.iter().filter(|op| matches!(op, Operator::Drop)).count(),
        3
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(
                op,
                Operator::If {
                    blockty: BlockType::Empty
                }
            ))
            .count(),
        2
    );
}

#[test]
fn never_scrutinees_reserve_no_match_scratch_or_local_set_but_check_all_arms() {
    for arms in [
        vec![wildcard(number("2"))],
        vec![
            arm(MatchPatternKind::String("unused".into()), number("1")),
            wildcard(number("2")),
        ],
    ] {
        let bytes = compiled(&returning(
            match_value(divergent(0), arms, ExprResult::Never),
            Type::Number,
        ));
        assert!(locals(&bytes).is_empty());
        assert!(
            !ops(&bytes)
                .iter()
                .any(|op| matches!(op, Operator::LocalSet { .. } | Operator::LocalGet { .. }))
        );
    }
    let bytes = compiled(&discarded(divergent(0), vec![wildcard(boolean(true))]));
    assert!(locals(&bytes).is_empty());
}

#[test]
fn forged_result_annotations_and_arm_literals_remain_independently_checked() {
    let value = match_value(number("1"), vec![wildcard(number("2"))], Type::Bool);
    assert!(matches!(
        rejected(&returning(value, Type::Bool)),
        CodegenError::TypeMismatch { .. }
    ));
    let value = match_value(number("1"), vec![wildcard(divergent(0))], Type::Number);
    assert!(matches!(
        rejected(&returning(value, Type::Number)),
        CodegenError::ExprResultMismatch {
            expected: ExprResult::Never,
            ..
        }
    ));
    let value = match_value(divergent(0), vec![wildcard(number("2"))], Type::Number);
    assert!(matches!(
        rejected(&returning(value, Type::Number)),
        CodegenError::ExprResultMismatch {
            expected: ExprResult::Never,
            ..
        }
    ));
    assert!(matches!(
        rejected(&discarded(number("0"), vec![wildcard(number("bad"))])),
        CodegenError::InvalidNumberLiteral { .. }
    ));
}

#[test]
fn scrutinee_scratch_is_reserved_before_nested_matches_and_array_arm_scratch() {
    let scrutinee = match_value(boolean(true), vec![wildcard(number("1"))], Type::Number);
    let bytes = compiled(&returning(
        match_value(
            scrutinee,
            vec![wildcard(expr(
                TypedExprKind::Array(vec![number("3")]),
                Type::NumberArray,
            ))],
            Type::NumberArray,
        ),
        Type::NumberArray,
    ));
    assert_eq!(locals(&bytes), [ValType::F64, ValType::I32, ValType::I32]);
    let indices: Vec<_> = ops(&bytes)
        .into_iter()
        .filter_map(|op| match op {
            Operator::LocalSet { local_index } => Some(local_index),
            _ => None,
        })
        .collect();
    assert_eq!(indices, [1, 0, 2]);
}

#[test]
fn string_patterns_intern_in_scrutinee_pattern_body_order_and_call_content_equality() {
    let bytes = compiled(&returning(
        match_value(
            string("é"),
            vec![
                arm(MatchPatternKind::String("a".into()), string("body")),
                arm(MatchPatternKind::String("é".into()), string("a")),
                wildcard(string("last")),
            ],
            Type::String,
        ),
        Type::String,
    ));
    assert_eq!(locals(&bytes), [ValType::I32]);
    let ops = ops(&bytes);
    let handles: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            Operator::I32Const { value } => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(handles, [8, 24, 40, 8, 24, 56]);
    let calls: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            Operator::Call { function_index } => Some(*function_index),
            _ => None,
        })
        .collect();
    assert_eq!(calls, [6, 6]); // one user, Alloc..StringEqual in existing order
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(
                op,
                Operator::If {
                    blockty: BlockType::Type(ValType::I32)
                }
            ))
            .count(),
        2
    );
    let data = Parser::new(0)
        .parse_all(&bytes)
        .find_map(|payload| match payload.unwrap() {
            Payload::DataSection(reader) => Some(reader.into_iter().next().unwrap().unwrap().data),
            _ => None,
        })
        .unwrap();
    assert_eq!(&data[8..16], &[16, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(&data[16..18], "é".as_bytes());
    assert_eq!(&data[32..33], b"a");
    assert_eq!(&data[48..52], b"body");
    assert_eq!(&data[64..68], b"last");
}

#[test]
fn pattern_only_strings_enable_memory_after_never_scrutinee() {
    let bytes = compiled(&returning(
        match_value(
            divergent(0),
            vec![
                arm(MatchPatternKind::String("pattern only".into()), number("1")),
                wildcard(number("2")),
            ],
            ExprResult::Never,
        ),
        Type::Number,
    ));
    assert!(
        Parser::new(0)
            .parse_all(&bytes)
            .any(|payload| matches!(payload.unwrap(), Payload::DataSection(_)))
    );
    assert!(
        !ops(&bytes)
            .iter()
            .any(|op| matches!(op, Operator::Call { .. }))
    );
}

#[test]
fn arm_transfers_use_dynamic_depths_and_preserve_live_operand_prefixes() {
    let continue_arm = block(
        vec![statement(TypedStatementKind::Continue {
            target: LoopId(0),
        })],
        None,
        ExprResult::Never,
    );
    let break_arm = block(
        vec![statement(TypedStatementKind::BreakValue {
            target: LoopId(0),
            value: number("7"),
        })],
        None,
        ExprResult::Never,
    );
    let loop_value = expr(
        TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![statement(TypedStatementKind::Expression(match_value(
                boolean(true),
                vec![
                    arm(MatchPatternKind::Bool(true), continue_arm),
                    wildcard(break_arm),
                ],
                ExprResult::Never,
            )))],
        },
        Type::Number,
    );
    let bytes = compiled(&returning(
        expr(
            TypedExprKind::Binary {
                left: Box::new(number("5")),
                op: BinaryOp::Add,
                right: Box::new(loop_value),
            },
            Type::Number,
        ),
        Type::Number,
    ));
    let branches: Vec<_> = ops(&bytes)
        .iter()
        .filter_map(|op| match op {
            Operator::Br { relative_depth } => Some(*relative_depth),
            _ => None,
        })
        .collect();
    assert_eq!(branches, [2, 3, 0]); // arm block + match if + loop (+ result block)
    assert!(ops(&bytes).iter().any(|op| matches!(op, Operator::F64Add)));
}

#[test]
fn never_scrutinee_arm_breaks_do_not_constrain_enclosing_value_loop() {
    let scrutinee = block(
        vec![statement(TypedStatementKind::Continue {
            target: LoopId(0),
        })],
        None,
        ExprResult::Never,
    );
    let arm_body = block(
        vec![statement(TypedStatementKind::BreakValue {
            target: LoopId(0),
            value: boolean(true),
        })],
        None,
        ExprResult::Never,
    );
    let value = expr(
        TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![statement(TypedStatementKind::Expression(match_value(
                scrutinee,
                vec![wildcard(arm_body)],
                ExprResult::Never,
            )))],
        },
        ExprResult::Never,
    );
    compiled(&returning(value, Type::Number));
}

#[test]
fn duplicate_patterns_do_not_prune_loop_break_constraints() {
    let breaking = |value| {
        block(
            vec![statement(TypedStatementKind::BreakValue {
                target: LoopId(0),
                value,
            })],
            None,
            ExprResult::Never,
        )
    };
    let loop_value = expr(
        TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![statement(TypedStatementKind::Match {
                scrutinee: Box::new(boolean(true)),
                arms: vec![
                    arm(MatchPatternKind::Bool(true), breaking(number("1"))),
                    arm(MatchPatternKind::Bool(true), breaking(boolean(true))),
                    wildcard(breaking(number("2"))),
                ],
            })],
        },
        Type::Number,
    );
    assert!(matches!(
        rejected(&returning(loop_value, Type::Number)),
        CodegenError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        }
    ));
}

#[test]
fn match_is_rejected_in_forged_contracts_and_normalized_defaults() {
    let value = match_value(boolean(true), vec![wildcard(boolean(true))], Type::Bool);
    let mut contract_program = program(Type::Void, vec![]);
    contract_program.functions[0].contracts.push(TypedContract {
        value: value.clone(),
        span: origin(),
    });
    assert!(
        matches!(rejected(&contract_program), CodegenError::InvalidContractExpression { span, .. } if span == origin())
    );

    let sequence = expr(
        TypedExprKind::NormalizedCall(TypedNormalizedCall {
            bindings: vec![TypedCallBinding {
                id: VariableId(10),
                ty: Type::Bool,
                value,
                origin: CallBindingOrigin::Default { formal: 0 },
                span: arm_span(),
            }],
            formal_to_binding: vec![VariableId(10)],
            call: Box::new(expr(
                TypedExprKind::Call {
                    func: FunctionRef::User(FunctionId(0)),
                    name: "main".into(),
                    args: vec![read(10, Type::Bool)],
                },
                Type::Bool,
            )),
        }),
        Type::Bool,
    );
    let mut defaults_program = returning(sequence, Type::Bool);
    defaults_program.functions[0].params.push(TypedParam {
        id: VariableId(0),
        name: "p".into(),
        ty: Type::Bool,
        span: origin(),
    });
    assert!(
        matches!(rejected(&defaults_program), CodegenError::InvalidNormalizedCall { span, .. } if span == arm_span())
    );
}

#[test]
fn all_concrete_results_and_void_never_joins_use_their_exact_region_types() {
    let cases = vec![
        (number("2"), Type::Number, ValType::F64),
        (boolean(true), Type::Bool, ValType::I32),
        (string("a"), Type::String, ValType::I32),
        (
            expr(TypedExprKind::Array(vec![]), Type::NumberArray),
            Type::NumberArray,
            ValType::I32,
        ),
        (
            expr(
                TypedExprKind::Range {
                    start: Box::new(number("0")),
                    end: Box::new(number("2")),
                    step: None,
                    inclusive: false,
                },
                Type::Range,
            ),
            Type::Range,
            ValType::I32,
        ),
    ];
    for (value, result, wasm_type) in cases {
        let program = returning(
            match_value(
                boolean(true),
                vec![
                    arm(MatchPatternKind::Bool(true), divergent(0)),
                    wildcard(value),
                ],
                result,
            ),
            result,
        );
        let bytes = if result == Type::Range {
            let bytes = codegen::compile_with_exports(&program, &ExportOptions::None).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            bytes
        } else {
            compiled(&program)
        };
        assert!(ops(&bytes).iter().any(
            |op| matches!(op, Operator::If { blockty: BlockType::Type(ty) } if *ty == wasm_type)
        ));
    }
    let void = block(vec![], None, ExprResult::Void);
    let bytes = compiled(&program(
        Type::Void,
        vec![statement(TypedStatementKind::Expression(match_value(
            boolean(true),
            vec![
                arm(MatchPatternKind::Bool(true), divergent(0)),
                wildcard(void),
            ],
            ExprResult::Void,
        )))],
    ));
    assert!(ops(&bytes).iter().any(|op| matches!(
        op,
        Operator::If {
            blockty: BlockType::Empty
        }
    )));
    assert!(!ops(&bytes).iter().any(|op| matches!(op, Operator::Drop)));
}

#[test]
#[ignore = "requires Node.js"]
fn runtime_selects_first_arm_once_with_nan_signed_zero_fractional_and_string_patterns() {
    use std::{process::Command, time::Duration};
    use was_compiler::host::HostFunctionId;

    let mut compiler = Compiler::new();
    compiler
        .register_host_function("probe", "env", "probe", vec![Type::Number], Type::Number)
        .unwrap();
    let mut program = compiler.analyze("fn main(n: number) -> number { return probe(n); } fn text(s: string) -> number { return 0; }").unwrap();
    let probe = |value| {
        expr(
            TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(0)),
                name: "probe".into(),
                args: vec![value],
            },
            Type::Number,
        )
    };
    program.functions[0].body = vec![statement(TypedStatementKind::Return(Some(match_value(
        probe(read(0, Type::Number)),
        vec![
            arm(MatchPatternKind::Number("-0".into()), probe(number("10"))),
            arm(
                MatchPatternKind::Interval {
                    start: "1.25".into(),
                    end: "2.75".into(),
                    inclusive: false,
                },
                probe(number("20")),
            ),
            arm(
                MatchPatternKind::Interval {
                    start: "1".into(),
                    end: "3".into(),
                    inclusive: true,
                },
                probe(number("30")),
            ),
            arm(
                MatchPatternKind::Relational {
                    op: BinaryOp::GreaterThan,
                    value: "3".into(),
                },
                probe(number("40")),
            ),
            wildcard(probe(number("50"))),
        ],
        Type::Number,
    ))))];
    program.functions[1].body = vec![statement(TypedStatementKind::Return(Some(match_value(
        read(0, Type::String),
        vec![
            arm(MatchPatternKind::String("é".into()), number("1")),
            wildcard(number("2")),
        ],
        Type::Number,
    ))))];
    let bytes = compiled(&program);
    let script = r#"
        const assert = require('node:assert/strict');
        const events = [];
        const bytes = require('node:fs').readFileSync(0);
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
            env: { probe(n) { events.push(n); return n; } }
        }).exports;
        for (const [n, expected] of [[0,10],[-0,10],[1.5,20],[2.75,30],[3,30],[Infinity,40],[-Infinity,50],[NaN,50]]) {
            assert.equal(e.main(n), expected);
            assert.equal(events.length, 2);
            assert.ok(Object.is(events[0], n));
            assert.equal(events[1], expected);
            events.length = 0;
        }
        function text(s) {
            const b = Buffer.from(s, 'utf8');
            const h = e.__alloc(8+b.length);
            const view = new DataView(e.memory.buffer);
            view.setUint32(h, h+8, true);
            view.setUint32(h+4, b.length, true);
            new Uint8Array(e.memory.buffer, h+8, b.length).set(b);
            return h;
        }
        assert.equal(e.text(text('é')), 1);
        assert.equal(e.text(text('e\u0301')), 2);
        assert.equal(e.text(text('other')), 2);
    "#;
    let output = test_process::output(
        Command::new("node").args(["-e", script]),
        Some(&bytes),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn arm_blocks_cannot_leak_initialized_bindings_to_other_arms() {
    let declaring = block(
        vec![statement(TypedStatementKind::Let {
            id: VariableId(8),
            name: "local".into(),
            ty: Type::Number,
            value: number("1"),
        })],
        Some(read(8, Type::Number)),
        Type::Number,
    );
    let value = match_value(
        boolean(true),
        vec![
            arm(MatchPatternKind::Bool(true), declaring),
            wildcard(read(8, Type::Number)),
        ],
        Type::Number,
    );
    assert!(matches!(
        rejected(&returning(value, Type::Number)),
        CodegenError::UninitializedVariable {
            id: VariableId(8),
            ..
        }
    ));
}
