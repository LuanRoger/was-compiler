use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type, codegen,
    typed_ast::{
        CallBindingOrigin, TypedExpr, TypedExprKind, TypedNormalizedCall, TypedProgram,
        TypedStatement, TypedStatementKind,
    },
};
use wasmparser::{Operator, Parser, Payload, ValType, Validator};

fn fixture() -> TypedProgram {
    Compiler::new()
        .analyze(
            "fn target(a: number, b: number = a + 1, c: number = b * 2) -> number { return c; }
         fn main() -> number { return target(3); }",
        )
        .unwrap()
}

fn sequence(program: &mut TypedProgram) -> &mut TypedNormalizedCall {
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &mut value.kind else {
        panic!("sequence")
    };
    sequence
}

#[test]
fn dependent_defaults_encode_full_arity_and_ordered_invocation_locals() {
    let program = fixture();
    let bytes = codegen::compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let mut signatures = Vec::new();
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::TypeSection(reader) => {
                signatures.extend(reader.into_iter_err_on_gc_types().map(Result::unwrap))
            }
            Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    assert_eq!(signatures[0].params(), [ValType::F64; 3]);
    assert_eq!(signatures[0].results(), [ValType::F64]);
    let locals: Vec<_> = bodies[1]
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(locals, [(1, ValType::F64); 3]);
    let ops: Vec<_> = bodies[1]
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(matches!(ops.as_slice(), [
        Operator::F64Const { value: a }, Operator::LocalSet { local_index: 0 },
        Operator::LocalGet { local_index: 0 }, Operator::F64Const { value: one }, Operator::F64Add, Operator::LocalSet { local_index: 1 },
        Operator::LocalGet { local_index: 1 }, Operator::F64Const { value: two }, Operator::F64Mul, Operator::LocalSet { local_index: 2 },
        Operator::LocalGet { local_index: 0 }, Operator::LocalGet { local_index: 1 }, Operator::LocalGet { local_index: 2 },
        Operator::Call { function_index: 0 }, Operator::Return, Operator::End,
    ] if a.bits() == 3f64.to_bits() && one.bits() == 1f64.to_bits() && two.bits() == 2f64.to_bits()));
}

#[test]
fn forged_default_origins_order_reads_and_whitelist_are_checked() {
    for case in 0..12 {
        let mut program = fixture();
        let seq = sequence(&mut program);
        let span = seq.bindings[1].span;
        match case {
            0 => seq.bindings[1].origin = CallBindingOrigin::Default { formal: 99 },
            1 => seq.bindings[1].origin = CallBindingOrigin::Default { formal: 0 },
            2 => seq.bindings.swap(1, 2),
            3 => seq.bindings[2].origin = CallBindingOrigin::Explicit { argument: 1 },
            4..=6 => {
                let id = match case {
                    4 => seq.bindings[1].id,
                    5 => seq.bindings[2].id,
                    _ => was_compiler::typed_ast::VariableId(999),
                };
                seq.bindings[1].value = TypedExpr {
                    kind: TypedExprKind::Identifier {
                        id,
                        name: "forged".into(),
                    },
                    ty: Type::Number.into(),
                };
            }
            7 => seq.bindings[1].value = *seq.call.clone(),
            8 => {
                seq.bindings[1].value.kind =
                    TypedExprKind::Block(was_compiler::typed_ast::TypedValueBlock {
                        statements: vec![],
                        tail: Some(Box::new(seq.bindings[1].value.clone())),
                        span,
                    })
            }
            9 => seq.bindings[1].value.kind = TypedExprKind::Array(vec![]),
            10 => {
                seq.bindings[1].value.kind = TypedExprKind::Range {
                    start: Box::new(seq.bindings[0].value.clone()),
                    end: Box::new(seq.bindings[0].value.clone()),
                    step: None,
                    inclusive: false,
                }
            }
            11 => {
                seq.bindings[1].value = TypedExpr {
                    kind: TypedExprKind::Binary {
                        left: Box::new(TypedExpr {
                            kind: TypedExprKind::Bool(false),
                            ty: Type::Bool.into(),
                        }),
                        op: was_compiler::ast::BinaryOp::And,
                        right: Box::new(*seq.call.clone()),
                    },
                    ty: Type::Number.into(),
                };
            }
            _ => unreachable!(),
        }
        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let mut compiler = Compiler::new();
            compiler.set_optimization_level(level);
            compiler.set_exports(ExportOptions::None);
            let error = compiler.compile(&program).unwrap_err();
            assert!(
                matches!(error, CodegenError::InvalidNormalizedCall { .. }),
                "case {case}: {error}"
            );
            assert!(error.span().is_some());
        }
    }
}

#[test]
fn forged_defaults_in_unreachable_and_short_circuited_unexported_bodies_are_checked() {
    for skipped in [false, true] {
        for malformed_mapping in [false, true] {
            let mut program = fixture();
            let span = program.functions[1].body[0].span;
            let seq = sequence(&mut program);
            let default_span = seq.bindings[1].span;
            if malformed_mapping {
                seq.formal_to_binding[0] = seq.formal_to_binding[1];
            } else {
                seq.bindings[1].value = *seq.call.clone();
            }
            let TypedStatementKind::Return(Some(value)) = program.functions[1].body.remove(0).kind
            else {
                panic!("return")
            };
            let expression = if skipped {
                TypedExpr {
                    kind: TypedExprKind::Binary {
                        left: Box::new(TypedExpr {
                            kind: TypedExprKind::Bool(false),
                            ty: Type::Bool.into(),
                        }),
                        op: was_compiler::ast::BinaryOp::And,
                        right: Box::new(TypedExpr {
                            kind: TypedExprKind::Binary {
                                left: Box::new(value),
                                op: was_compiler::ast::BinaryOp::Equal,
                                right: Box::new(TypedExpr {
                                    kind: TypedExprKind::Number("0".into()),
                                    ty: Type::Number.into(),
                                }),
                            },
                            ty: Type::Bool.into(),
                        }),
                    },
                    ty: Type::Bool.into(),
                }
            } else {
                value
            };
            program.functions[1].body = vec![
                TypedStatement {
                    kind: TypedStatementKind::Return(Some(TypedExpr {
                        kind: TypedExprKind::Number("8".into()),
                        ty: Type::Number.into(),
                    })),
                    span,
                },
                TypedStatement {
                    kind: TypedStatementKind::Expression(expression),
                    span,
                },
            ];
            if skipped {
                program.functions[1].body.swap(0, 1);
            }
            for level in [
                OptimizationLevel::None,
                OptimizationLevel::Basic,
                OptimizationLevel::Standard,
                OptimizationLevel::Aggressive,
            ] {
                let mut compiler = Compiler::new();
                compiler.set_optimization_level(level);
                compiler.set_exports(ExportOptions::Only(vec!["target".into()]));
                let error = compiler.compile(&program).unwrap_err();
                assert!(matches!(error, CodegenError::InvalidNormalizedCall { .. }));
                assert_eq!(
                    error.span(),
                    Some(if malformed_mapping {
                        span
                    } else {
                        default_span
                    })
                );
            }
        }
    }
}

#[test]
fn explicit_initializers_cannot_read_bindings_and_default_origins_survive_child_errors() {
    let mut program = fixture();
    let seq = sequence(&mut program);
    seq.bindings[0].value = seq.bindings[1].value.clone();
    assert!(matches!(
        codegen::compile(&program),
        Err(CodegenError::InvalidNormalizedCall { .. })
    ));

    let mut program = fixture();
    let seq = sequence(&mut program);
    let span = seq.bindings[2].span;
    seq.bindings[2].value = TypedExpr {
        kind: TypedExprKind::Number("NaN".into()),
        ty: Type::Number.into(),
    };
    assert!(
        matches!(codegen::compile(&program), Err(CodegenError::InvalidNumberLiteral { span: actual, .. }) if actual == span)
    );
}
