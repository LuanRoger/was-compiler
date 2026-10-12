use was_compiler::{
    CodegenError, Compiler, Type, codegen,
    tokens::Span,
    typed_ast::{
        CallBindingOrigin, TypedCallBinding, TypedExpr, TypedExprKind, TypedNormalizedCall,
        TypedProgram, TypedStatementKind, VariableId,
    },
};
use wasmparser::{Operator, Parser, Payload, ValType, Validator};

fn identifier(id: VariableId, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Identifier {
            id,
            name: "synthetic".into(),
        },
        ty: ty.into(),
    }
}

fn fixture() -> TypedProgram {
    let mut program = Compiler::new()
        .analyze(
            "fn target(a: number, b: number) -> number { return a - b; }
         fn main() -> number { return target(11, 22); }",
        )
        .unwrap();
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    let mut call = value.clone();
    let TypedExprKind::Call { args, .. } = &mut call.kind else {
        panic!("call")
    };
    let values = std::mem::take(args);
    *args = vec![
        identifier(VariableId(101), Type::Number),
        identifier(VariableId(100), Type::Number),
    ];
    value.kind = TypedExprKind::NormalizedCall(TypedNormalizedCall {
        bindings: values
            .into_iter()
            .enumerate()
            .map(|(argument, value)| TypedCallBinding {
                id: VariableId(100 + argument),
                ty: Type::Number,
                value,
                origin: CallBindingOrigin::Explicit { argument },
                span: Span::new(7 + argument, 8 + argument),
            })
            .collect(),
        formal_to_binding: vec![VariableId(101), VariableId(100)],
        call: Box::new(call),
    });
    program
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
fn written_order_storage_and_formal_order_reads_are_encoded_and_validated() {
    let program = fixture();
    let snapshot = program.clone();
    let bytes = codegen::compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let body = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .nth(1)
        .unwrap();
    let locals: Vec<_> = body
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .flat_map(|local| {
            let (count, ty) = local.unwrap();
            std::iter::repeat_n(ty, usize::try_from(count).unwrap())
        })
        .collect();
    assert_eq!(locals, [ValType::F64, ValType::F64]);
    let ops: Vec<_> = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(matches!(ops.as_slice(), [
        Operator::F64Const { value: first }, Operator::LocalSet { local_index: 0 },
        Operator::F64Const { value: second }, Operator::LocalSet { local_index: 1 },
        Operator::LocalGet { local_index: 1 }, Operator::LocalGet { local_index: 0 },
        Operator::Call { function_index: 0 }, Operator::Return, Operator::End,
    ] if first.bits() == 11f64.to_bits() && second.bits() == 22f64.to_bits()));
    assert_eq!(program, snapshot);
}

fn assert_invalid(program: &TypedProgram, reason: &'static str, span: Span) {
    let expected = CodegenError::InvalidNormalizedCall { reason, span };
    assert_eq!(codegen::compile(program).unwrap_err(), expected);
    let mut compiler = Compiler::new();
    for level in [
        was_compiler::OptimizationLevel::None,
        was_compiler::OptimizationLevel::Basic,
        was_compiler::OptimizationLevel::Standard,
        was_compiler::OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        compiler.set_exports(was_compiler::ExportOptions::None);
        assert_eq!(compiler.compile(program).unwrap_err(), expected);
    }
}

#[test]
fn forged_structure_mapping_origins_and_canonical_arguments_are_rejected() {
    for case in 0..14 {
        let mut program = fixture();
        let span = program.functions[1].body[0].span;
        let seq = sequence(&mut program);
        let mut error_span = span;
        let reason = match case {
            0 => {
                seq.formal_to_binding.pop();
                "binding, mapping and argument counts must match the signature"
            }
            1 => {
                seq.bindings.pop();
                "binding, mapping and argument counts must match the signature"
            }
            2 => {
                seq.formal_to_binding[1] = seq.formal_to_binding[0];
                "formal mapping must be a bijection"
            }
            3 => {
                seq.formal_to_binding[0] = VariableId(999);
                "formal mapping must own every referenced binding"
            }
            4 | 5 => {
                seq.bindings[1].origin = if case == 4 {
                    CallBindingOrigin::Default { formal: 1 }
                } else {
                    CallBindingOrigin::Explicit { argument: 0 }
                };
                error_span = seq.bindings[1].span;
                "bindings must have contiguous written-order explicit origins"
            }
            6 => {
                seq.bindings[0].ty = Type::Bool;
                "mapped binding and argument types must match the formal"
            }
            7 => {
                seq.call.ty = was_compiler::typed_ast::ExprResult::Never;
                "final call result does not match its signature"
            }
            8 => {
                seq.call.kind = TypedExprKind::Number("1".into());
                "final expression must be a direct user call"
            }
            9..=13 => {
                let TypedExprKind::Call { func, name, args } = &mut seq.call.kind else {
                    panic!("call")
                };
                match case {
                    9 => {
                        *func = was_compiler::typed_ast::FunctionRef::Builtin(
                            was_compiler::typed_ast::BuiltinFunction::Len,
                        );
                        "final expression must be a direct user call"
                    }
                    10 => {
                        args[0].ty = Type::Bool.into();
                        "mapped binding and argument types must match the formal"
                    }
                    11 => {
                        args[0] = identifier(VariableId(100), Type::Number);
                        "final arguments must read owned bindings in formal order"
                    }
                    12 => {
                        args[0] = TypedExpr {
                            kind: TypedExprKind::Number("1".into()),
                            ty: Type::Number.into(),
                        };
                        "final arguments must read owned bindings in formal order"
                    }
                    13 => {
                        *name = "wrong".into();
                        "final call name does not match its target"
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };
        assert_invalid(&program, reason, error_span);
    }
}

#[test]
fn synthetic_bindings_cannot_be_read_in_any_initializer_or_escape() {
    for argument in 0..2 {
        for id in [VariableId(100), VariableId(101)] {
            let mut program = fixture();
            let seq = sequence(&mut program);
            seq.bindings[argument].value = identifier(id, Type::Number);
            let span = seq.bindings[argument].span;
            assert_invalid(
                &program,
                "synthetic binding is only readable by its final call",
                span,
            );
        }
    }
    let mut program = fixture();
    let span = program.functions[1].body[0].span;
    program.functions[1]
        .body
        .push(was_compiler::typed_ast::TypedStatement {
            kind: TypedStatementKind::Expression(identifier(VariableId(100), Type::Number)),
            span,
        });
    assert_invalid(
        &program,
        "synthetic binding is only readable by its final call",
        span,
    );
}

#[test]
fn nested_sequence_initializers_cannot_read_enclosing_bindings() {
    let mut program = fixture();
    let mut nested_program = fixture();
    let inner = sequence(&mut nested_program);
    for (argument, binding) in inner.bindings.iter_mut().enumerate() {
        binding.id = VariableId(200 + argument);
    }
    inner.formal_to_binding = vec![VariableId(201), VariableId(200)];
    let TypedExprKind::Call { args, .. } = &mut inner.call.kind else {
        panic!("call")
    };
    *args = vec![
        identifier(VariableId(201), Type::Number),
        identifier(VariableId(200), Type::Number),
    ];
    inner.bindings[1].value = identifier(VariableId(100), Type::Number);
    let span = inner.bindings[1].span;
    sequence(&mut program).bindings[1].value = TypedExpr {
        kind: TypedExprKind::NormalizedCall(inner.clone()),
        ty: Type::Number.into(),
    };
    assert_invalid(
        &program,
        "synthetic binding is only readable by its final call",
        span,
    );
}

#[test]
fn synthetic_bindings_cannot_be_assigned_in_initializers() {
    use was_compiler::ast::BinaryOp;
    use was_compiler::typed_ast::{TypedStatement, TypedValueBlock};
    for compound in [
        None,
        Some(BinaryOp::Add),
        Some(BinaryOp::Subtract),
        Some(BinaryOp::Multiply),
        Some(BinaryOp::Divide),
        Some(BinaryOp::Remainder),
    ] {
        let mut program = fixture();
        let seq = sequence(&mut program);
        let span = seq.bindings[1].span;
        let target = identifier(VariableId(100), Type::Number);
        let value = TypedExpr {
            kind: TypedExprKind::Number("1".into()),
            ty: Type::Number.into(),
        };
        let kind = match compound {
            None => TypedStatementKind::Assignment { target, value },
            Some(op) => TypedStatementKind::CompoundAssignment { target, op, value },
        };
        seq.bindings[1].value = TypedExpr {
            kind: TypedExprKind::Block(TypedValueBlock {
                statements: vec![TypedStatement { kind, span }],
                tail: Some(Box::new(TypedExpr {
                    kind: TypedExprKind::Number("2".into()),
                    ty: Type::Number.into(),
                })),
                span,
            }),
            ty: Type::Number.into(),
        };
        assert_invalid(
            &program,
            "synthetic binding is only readable by its final call",
            span,
        );
    }
}

#[test]
fn normalized_calls_support_mixed_storage_types_and_void_results() {
    for (result, target_body, main_body) in [
        ("string", "return a;", "return target(\"text\", [1], true);"),
        (
            "number[]",
            "return b;",
            "return target(\"text\", [1], true);",
        ),
        ("bool", "return c;", "return target(\"text\", [1], true);"),
        ("void", "", "target(\"text\", [1], true);"),
    ] {
        let source = format!(
            "fn target(a: string, b: number[], c: bool) -> {result} {{ {target_body} }} fn main() -> {result} {{ {main_body} }}"
        );
        let mut program = Compiler::new().analyze(&source).unwrap();
        let span = program.functions[1].body[0].span;
        let value = match &mut program.functions[1].body[0].kind {
            TypedStatementKind::Return(Some(value)) | TypedStatementKind::Expression(value) => {
                value
            }
            _ => panic!("call statement"),
        };
        let mut call = value.clone();
        let TypedExprKind::Call { args, .. } = &mut call.kind else {
            panic!("call")
        };
        let bindings: Vec<_> = std::mem::take(args)
            .into_iter()
            .rev()
            .enumerate()
            .map(|(argument, value)| TypedCallBinding {
                id: VariableId(100 + argument),
                ty: value.ty.value_type().unwrap(),
                value,
                origin: CallBindingOrigin::Explicit { argument },
                span,
            })
            .collect();
        *args = bindings
            .iter()
            .rev()
            .map(|binding| identifier(binding.id, binding.ty))
            .collect();
        value.kind = TypedExprKind::NormalizedCall(TypedNormalizedCall {
            bindings,
            formal_to_binding: vec![VariableId(102), VariableId(101), VariableId(100)],
            call: Box::new(call),
        });
        let bytes = codegen::compile(&program).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let mut compiler = Compiler::new();
        for level in [
            was_compiler::OptimizationLevel::None,
            was_compiler::OptimizationLevel::Basic,
            was_compiler::OptimizationLevel::Standard,
            was_compiler::OptimizationLevel::Aggressive,
        ] {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap(), bytes);
        }
    }
}

#[test]
fn synthetic_storage_ids_are_unique_across_the_whole_function() {
    let mut program = fixture();
    let span = sequence(&mut program).bindings[1].span;
    sequence(&mut program).bindings[1].id = VariableId(100);
    assert_eq!(
        codegen::compile(&program),
        Err(CodegenError::DuplicateVariableId {
            function: "main".into(),
            id: VariableId(100),
            span,
        })
    );
    let mut program = fixture();
    let span = program.functions[1].span;
    program.functions[1]
        .params
        .push(was_compiler::typed_ast::TypedParam {
            id: VariableId(100),
            name: "collision".into(),
            ty: Type::Number,
            span,
        });
    let binding_span = sequence(&mut program).bindings[0].span;
    assert_eq!(
        codegen::compile(&program),
        Err(CodegenError::DuplicateVariableId {
            function: "main".into(),
            id: VariableId(100),
            span: binding_span,
        })
    );
}

#[test]
fn initializers_transfer_to_the_existing_loop_with_live_stack_prefixes() {
    use was_compiler::typed_ast::{ExprResult, TypedStatement, TypedValueBlock};
    let mut program = Compiler::new()
        .analyze(
            "fn target(a: number, b: number) -> number { return a - b; }
         fn main() -> number { return (loop { break target(11, 22); }); }",
        )
        .unwrap();
    let TypedStatementKind::Return(Some(outer)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::Loop { id, body } = &mut outer.kind else {
        panic!("loop")
    };
    let span = body[0].span;
    let TypedStatementKind::BreakValue { value, .. } = &mut body[0].kind else {
        panic!("break")
    };
    let mut normalized = fixture();
    let seq = sequence(&mut normalized);
    seq.bindings[0].value = TypedExpr {
        kind: TypedExprKind::Block(TypedValueBlock {
            statements: vec![TypedStatement {
                kind: TypedStatementKind::BreakValue {
                    target: *id,
                    value: TypedExpr {
                        kind: TypedExprKind::Number("5".into()),
                        ty: Type::Number.into(),
                    },
                },
                span,
            }],
            tail: None,
            span,
        }),
        ty: ExprResult::Never,
    };
    *value = TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(TypedExpr {
                kind: TypedExprKind::Number("99".into()),
                ty: Type::Number.into(),
            }),
            op: was_compiler::ast::BinaryOp::Add,
            right: Box::new(TypedExpr {
                kind: TypedExprKind::NormalizedCall(seq.clone()),
                ty: ExprResult::Never,
            }),
        },
        ty: ExprResult::Never,
    };
    let bytes = codegen::compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn final_call_children_are_still_traversed_by_declaration_planning() {
    let mut program = fixture();
    let span = sequence(&mut program).bindings[0].span;
    let TypedExprKind::Call { args, .. } = &mut sequence(&mut program).call.kind else {
        panic!("call")
    };
    args[0].kind = TypedExprKind::Block(was_compiler::typed_ast::TypedValueBlock {
        statements: vec![was_compiler::typed_ast::TypedStatement {
            kind: TypedStatementKind::Let {
                id: VariableId(100),
                name: "collision".into(),
                ty: Type::Number,
                value: TypedExpr {
                    kind: TypedExprKind::Number("1".into()),
                    ty: Type::Number.into(),
                },
            },
            span,
        }],
        tail: None,
        span,
    });
    assert_eq!(
        codegen::compile(&program),
        Err(CodegenError::DuplicateVariableId {
            function: "main".into(),
            id: VariableId(100),
            span,
        })
    );
}

#[test]
fn skipped_initializers_plan_declaration_preorder_memory_scratch_and_remainder_separately() {
    let mut program = fixture();
    let mut initializer = Compiler::new().analyze(
        "fn inner() -> number { return { let text = \"first\"; let data = [7 % 3]; len(data) }; }",
    ).unwrap();
    let TypedStatementKind::Return(Some(mut value)) = initializer.functions[0].body.remove(0).kind
    else {
        panic!("return")
    };
    let TypedExprKind::Block(block) = &mut value.kind else {
        panic!("block")
    };
    let TypedStatementKind::Let { id, .. } = &mut block.statements[0].kind else {
        panic!("let")
    };
    *id = VariableId(300);
    let TypedStatementKind::Let { id, .. } = &mut block.statements[1].kind else {
        panic!("let")
    };
    *id = VariableId(301);
    let TypedExprKind::Call { args, .. } = &mut block.tail.as_mut().unwrap().kind else {
        panic!("call")
    };
    args[0] = identifier(VariableId(301), Type::NumberArray);
    sequence(&mut program).bindings[0].value = value;
    let span = program.functions[1].body[0].span;
    program.functions[1].body.insert(
        0,
        was_compiler::typed_ast::TypedStatement {
            kind: TypedStatementKind::Return(Some(TypedExpr {
                kind: TypedExprKind::Number("0".into()),
                ty: Type::Number.into(),
            })),
            span,
        },
    );
    let bytes = codegen::compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let mut bodies = Vec::new();
    let mut data = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::CodeSectionEntry(body) => bodies.push(body),
            Payload::DataSection(section) => {
                for segment in section {
                    data.extend_from_slice(segment.unwrap().data);
                }
            }
            _ => {}
        }
    }
    assert_eq!(bodies.len(), 10); // Two users, seven memory helpers, floating remainder.
    assert!(data.windows(5).any(|bytes| bytes == b"first"));
    let locals: Vec<_> = bodies[1]
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .flat_map(|local| {
            let (count, ty) = local.unwrap();
            std::iter::repeat_n(ty, usize::try_from(count).unwrap())
        })
        .collect();
    assert_eq!(
        locals,
        [
            ValType::F64,
            ValType::I32,
            ValType::I32,
            ValType::F64,
            ValType::I32
        ]
    );
    let ops: Vec<_> = bodies[1]
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(
        ops.iter()
            .any(|op| matches!(op, Operator::Call { function_index: 9 }))
    );
    let stores: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            Operator::LocalSet { local_index } => Some(*local_index),
            _ => None,
        })
        .collect();
    assert_eq!(stores, [1, 4, 2, 0, 3]);
}

#[test]
fn never_initializers_preserve_canonical_result_and_check_later_children() {
    use was_compiler::typed_ast::{ExprResult, LoopId};
    let mut program = fixture();
    sequence(&mut program).bindings[0].value = TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(77),
            body: vec![],
        },
        ty: ExprResult::Never,
    };
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    value.ty = ExprResult::Never;
    let bytes = codegen::compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    assert_eq!(
        sequence(&mut program).call.ty,
        ExprResult::Value(Type::Number)
    );
    sequence(&mut program).bindings[1].value.kind = TypedExprKind::Number("invalid".into());
    let span = sequence(&mut program).bindings[1].span;
    assert_eq!(
        codegen::compile(&program),
        Err(CodegenError::InvalidNumberLiteral {
            function: "main".into(),
            literal: "invalid".into(),
            span,
        })
    );
}
