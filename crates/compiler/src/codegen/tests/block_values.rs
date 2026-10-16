use super::*;
use crate::{
    exports::ExportOptions,
    typed_ast::{ExprResult, TypedValueBlock},
};

fn span() -> Span {
    Span::new(12, 34)
}

fn value_block(statements: Vec<TypedStatement>, tail: Option<TypedExpr>) -> TypedValueBlock {
    TypedValueBlock {
        statements,
        tail: tail.map(Box::new),
        span: span(),
    }
}

fn block(statements: Vec<TypedStatement>, tail: Option<TypedExpr>, ty: ExprResult) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Block(value_block(statements, tail)),
        ty,
    }
}

fn never() -> TypedExpr {
    block(
        vec![returned(Some(number_literal("7")), span())],
        None,
        ExprResult::Never,
    )
}

fn expression(value: TypedExpr) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::Expression(value),
        span: span(),
    }
}

fn value_if(
    cond: TypedExpr,
    then: TypedValueBlock,
    else_: Option<TypedValueBlock>,
    ty: ExprResult,
) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::If {
            cond: Box::new(cond),
            then,
            else_,
        },
        ty,
    }
}

#[test]
fn scalar_handle_and_void_regions_use_mvp_results_and_only_values_are_dropped() {
    for (ty, tail, wasm_ty) in [
        (Type::Number, number(), ValType::F64),
        (Type::Bool, boolean(), ValType::I32),
        (
            Type::String,
            TypedExpr {
                kind: TypedExprKind::String("s".into()),
                ty: Type::String.into(),
            },
            ValType::I32,
        ),
        (
            Type::Array(crate::ast::ArrayElementType::Number),
            TypedExpr {
                kind: TypedExprKind::Array(vec![], crate::ast::ArrayElementType::Number),
                ty: Type::Array(crate::ast::ArrayElementType::Number).into(),
            },
            ValType::I32,
        ),
    ] {
        let value = block(vec![], Some(tail), ty.into());
        let branch = value_if(
            boolean(),
            value_block(vec![], Some(value.clone())),
            Some(value_block(vec![], Some(value.clone()))),
            ty.into(),
        );
        let program = local_program(vec![expression(value), expression(branch)]);
        let module = decode(&compile(&program).unwrap());
        let body = &module.bodies[0];
        assert_eq!(
            body.iter()
                .filter(|op| **op == DecodedInstruction::Block(BlockType::Type(wasm_ty)))
                .count(),
            3
        );
        assert!(body.contains(&DecodedInstruction::If(BlockType::Type(wasm_ty))));
        assert_eq!(
            body.iter()
                .filter(|op| **op == DecodedInstruction::Drop)
                .count(),
            2
        );
    }

    let value = block(vec![], None, ExprResult::Void);
    let program = local_program(vec![expression(value)]);
    assert_eq!(
        decode(&compile(&program).unwrap()).bodies[0],
        [
            DecodedInstruction::Block(BlockType::Empty),
            DecodedInstruction::End,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn divergent_initializers_introduce_static_bindings_without_stores_or_drops() {
    let program = return_program(None, Type::Number, span());
    let mut program = program;
    program.functions[0].body = vec![
        declaration(99, Type::Number, never(), span()),
        expression(identifier(99, "unreachable", Type::Number)),
        returned(Some(number()), span()),
    ];
    let module = decode(&compile(&program).unwrap());
    assert_eq!(module.locals[0], [ValType::F64]);
    assert!(!module.bodies[0].contains(&DecodedInstruction::LocalSet(0)));
    assert!(module.bodies[0].contains(&DecodedInstruction::LocalGet(0)));
    assert_eq!(
        module.bodies[0]
            .iter()
            .filter(|op| **op == DecodedInstruction::Drop)
            .count(),
        1
    );
    assert!(
        module.bodies[0]
            .windows(2)
            .any(|ops| ops == [DecodedInstruction::End, DecodedInstruction::Unreachable])
    );
}

#[test]
fn divergent_arms_join_each_concrete_type_without_artificial_result_locals() {
    for (ty, value, wasm_type) in [
        (Type::Number, number(), ValType::F64),
        (Type::Bool, boolean(), ValType::I32),
        (
            Type::String,
            TypedExpr {
                kind: TypedExprKind::String("s".into()),
                ty: Type::String.into(),
            },
            ValType::I32,
        ),
        (
            Type::Array(crate::ast::ArrayElementType::Number),
            TypedExpr {
                kind: TypedExprKind::Array(vec![], crate::ast::ArrayElementType::Number),
                ty: Type::Array(crate::ast::ArrayElementType::Number).into(),
            },
            ValType::I32,
        ),
    ] {
        for divergent_then in [false, true] {
            let (then, else_) = if divergent_then {
                (never(), value.clone())
            } else {
                (value.clone(), never())
            };
            let expr = value_if(
                boolean(),
                value_block(vec![], Some(then)),
                Some(value_block(vec![], Some(else_))),
                ty.into(),
            );
            let mut program =
                local_program(vec![expression(expr), returned(Some(number()), span())]);
            program.functions[0].return_type = Type::Number;
            let module = decode(&compile(&program).unwrap());
            assert!(module.bodies[0].contains(&DecodedInstruction::If(BlockType::Type(wasm_type))));
            assert_eq!(
                module.locals[0].len(),
                usize::from(ty == Type::Array(crate::ast::ArrayElementType::Number))
            );
            assert_eq!(
                module.bodies[0]
                    .iter()
                    .filter(|op| **op == DecodedInstruction::Drop)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn eager_expressions_propagate_never_without_stopping_static_lowering() {
    let mut add = binary(never(), BinaryOp::Add, number());
    add.ty = ExprResult::Never;
    let mut negate = unary(UnaryOp::Negate, never());
    negate.ty = ExprResult::Never;
    let array = TypedExpr {
        kind: TypedExprKind::Array(
            vec![never(), number()],
            crate::ast::ArrayElementType::Number,
        ),
        ty: ExprResult::Never,
    };
    let index = TypedExpr {
        kind: TypedExprKind::Index {
            expr: Box::new(TypedExpr {
                kind: TypedExprKind::Array(vec![], crate::ast::ArrayElementType::Number),
                ty: Type::Array(crate::ast::ArrayElementType::Number).into(),
            }),
            index: Box::new(never()),
        },
        ty: ExprResult::Never,
    };
    let mut call = call(
        FunctionRef::User(FunctionId(1)),
        "callee",
        vec![never(), number()],
        Type::Number,
    );
    call.ty = ExprResult::Never;
    let mut equality = bool_binary(never(), BinaryOp::Equal, never());
    equality.ty = ExprResult::Never;

    for value in [add, negate, array, index, call, equality] {
        let mut program = return_program(Some(value), Type::Number, span());
        let mut callee = user(1, "callee", &[Type::Number, Type::Number], Type::Number);
        callee.body = vec![returned(Some(number()), span())];
        program.functions.push(callee);
        let module = decode(&compile(&program).unwrap());
        assert!(!module.bodies[0].contains(&DecodedInstruction::Drop));
        assert_eq!(module.bodies[0].last(), Some(&DecodedInstruction::End));
    }
}

#[test]
fn assignments_and_compounds_propagate_abrupt_target_and_rhs_flow() {
    for source in [
        "fn test() -> number { let x = 1; x = { return 7; }; }",
        "fn test() -> number { let x = 1; x += { return 7; }; }",
        "fn test() -> number { let a = [1]; a[0] = { return 7; }; }",
        "fn test() -> number { let a = [1]; a[0] += { return 7; }; }",
        "fn test() -> number { let a = [1]; a[{ return 7; }] = { let rhs = [2]; 3 }; }",
        "fn test() -> number { let a = [1]; a[{ return 7; }] %= { let rhs = [2]; 3 }; }",
    ] {
        let program = Compiler::new().analyze(source).unwrap();
        let module = decode(&compile(&program).unwrap());
        assert!(module.bodies[0].contains(&DecodedInstruction::Return));
        assert!(!module.bodies[0].contains(&DecodedInstruction::Drop));
    }
}

#[test]
fn short_circuit_rhs_never_retains_a_normal_bool_path_but_left_never_does_not() {
    for op in [BinaryOp::And, BinaryOp::Or] {
        for left_never in [false, true] {
            let mut value = bool_binary(if left_never { never() } else { boolean() }, op, never());
            if left_never {
                value.ty = ExprResult::Never;
            }
            let program = return_program(Some(value), Type::Number, span());
            let mut program = program;
            // The short-circuit expression is discarded; its nested return is numeric.
            let TypedStatementKind::Return(Some(value)) = program.functions[0].body.remove(0).kind
            else {
                panic!("return")
            };
            program.functions[0].body = vec![expression(value), returned(Some(number()), span())];
            let body = &decode(&compile(&program).unwrap()).bodies[0];
            assert!(body.contains(&DecodedInstruction::If(if left_never {
                BlockType::Empty
            } else {
                BlockType::Type(ValType::I32)
            })));
            assert_eq!(
                body.iter()
                    .filter(|op| **op == DecodedInstruction::Drop)
                    .count(),
                usize::from(!left_never)
            );
        }
    }
}

#[test]
fn never_conditions_and_unreachable_tails_do_not_create_live_region_results() {
    let unreachable_tail = block(
        vec![returned(Some(number()), span())],
        Some(boolean()),
        ExprResult::Never,
    );
    let conditional = value_if(
        never(),
        value_block(vec![], Some(boolean())),
        Some(value_block(vec![], Some(boolean()))),
        ExprResult::Never,
    );
    for value in [unreachable_tail, conditional] {
        let program = return_program(Some(value), Type::Number, span());
        let body = &decode(&compile(&program).unwrap()).bodies[0];
        assert!(!body.contains(&DecodedInstruction::Drop));
        assert!(
            body.windows(2)
                .any(|ops| ops == [DecodedInstruction::End, DecodedInstruction::Unreachable])
        );
    }
}

#[test]
fn checked_lowering_rejects_forged_result_categories_including_value_void() {
    let cases = [
        (
            block(vec![returned(None, span())], None, Type::Number.into()),
            ExprResult::Never,
            Type::Number.into(),
        ),
        (
            block(vec![], Some(number()), ExprResult::Never),
            Type::Number.into(),
            ExprResult::Never,
        ),
        (
            block(vec![], None, ExprResult::Never),
            ExprResult::Void,
            ExprResult::Never,
        ),
        (
            block(vec![], None, ExprResult::Value(Type::Void)),
            ExprResult::Void,
            ExprResult::Value(Type::Void),
        ),
        (
            TypedExpr {
                kind: TypedExprKind::Number("1".into()),
                ty: ExprResult::Never,
            },
            Type::Number.into(),
            ExprResult::Never,
        ),
    ];
    for (value, expected, found) in cases {
        let error = compile(&local_program(vec![expression(value)])).unwrap_err();
        assert_eq!(
            error,
            CodegenError::ExprResultMismatch {
                function: "bad".into(),
                expected,
                found,
                span: span()
            }
        );
        assert_eq!(error.span(), Some(span()));
    }
    let identifier = identifier(0, "param", Type::Number);
    let mut function = user(0, "bad", &[Type::Number], Type::Void);
    let mut identifier = identifier;
    identifier.ty = ExprResult::Never;
    function.body = vec![expression(identifier)];
    assert!(matches!(
        compile(&TypedProgram {
            functions: vec![function],
            host_functions: vec![]
        }),
        Err(CodegenError::ExprResultMismatch {
            expected: ExprResult::Value(Type::Number),
            found: ExprResult::Never,
            ..
        })
    ));
}

#[test]
fn forged_if_arms_conditions_and_missing_else_are_checked_structurally() {
    let make = |cond, then, else_, ty| expression(value_if(cond, then, else_, ty));
    let cases = [
        (
            make(
                boolean(),
                value_block(vec![], Some(number())),
                None,
                Type::Number.into(),
            ),
            CodegenError::MissingElse {
                function: "bad".into(),
                span: span(),
            },
        ),
        (
            make(boolean(), value_block(vec![], None), None, ExprResult::Void),
            CodegenError::MissingElse {
                function: "bad".into(),
                span: span(),
            },
        ),
        (
            make(
                number(),
                value_block(vec![], Some(number())),
                Some(value_block(vec![], Some(number()))),
                Type::Number.into(),
            ),
            CodegenError::TypeMismatch {
                function: "bad".into(),
                expected: Type::Bool,
                found: Type::Number,
                span: span(),
            },
        ),
        (
            make(
                boolean(),
                value_block(vec![], Some(number())),
                Some(value_block(vec![], Some(boolean()))),
                Type::Number.into(),
            ),
            CodegenError::TypeMismatch {
                function: "bad".into(),
                expected: Type::Number,
                found: Type::Bool,
                span: span(),
            },
        ),
        (
            make(
                boolean(),
                value_block(vec![returned(None, span())], None),
                Some(value_block(vec![returned(None, span())], None)),
                Type::Number.into(),
            ),
            CodegenError::ExprResultMismatch {
                function: "bad".into(),
                expected: ExprResult::Never,
                found: Type::Number.into(),
                span: span(),
            },
        ),
    ];
    for (statement, expected) in cases {
        assert_eq!(compile(&local_program(vec![statement])), Err(expected));
    }
}

#[test]
fn hidden_skipped_and_unreachable_value_children_are_not_pruned() {
    let invalid = number_literal("NaN");
    let cases = [
        value_if(
            boolean(),
            value_block(vec![], Some(number())),
            Some(value_block(vec![], Some(invalid.clone()))),
            Type::Number.into(),
        ),
        block(
            vec![returned(None, span())],
            Some(invalid.clone()),
            ExprResult::Never,
        ),
        bool_binary(
            boolean(),
            BinaryOp::And,
            block(
                vec![expression(invalid.clone())],
                Some(boolean()),
                Type::Bool.into(),
            ),
        ),
        block(
            vec![returned(None, span()), expression(invalid)],
            None,
            ExprResult::Never,
        ),
    ];
    for value in cases {
        let program = local_program(vec![expression(value)]);
        for exports in [ExportOptions::All, ExportOptions::None] {
            assert_eq!(
                super::super::compile_with_exports(&program, &exports),
                Err(CodegenError::InvalidNumberLiteral {
                    function: "bad".into(),
                    literal: "NaN".into(),
                    span: span()
                })
            );
        }
    }
    // Eager Never does not excuse the other operand's type or invalid contents.
    let mut value = binary(
        block(vec![returned(None, span())], None, ExprResult::Never),
        BinaryOp::Add,
        boolean(),
    );
    value.ty = ExprResult::Never;
    assert!(matches!(
        compile(&local_program(vec![expression(value)])),
        Err(CodegenError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        })
    ));
}

#[test]
fn eager_never_does_not_mask_later_argument_array_index_or_equality_type_errors() {
    let abrupt = || block(vec![returned(None, span())], None, ExprResult::Never);
    let array = TypedExpr {
        kind: TypedExprKind::Array(
            vec![abrupt(), boolean()],
            crate::ast::ArrayElementType::Number,
        ),
        ty: ExprResult::Never,
    };
    let index = TypedExpr {
        kind: TypedExprKind::Index {
            expr: Box::new(abrupt()),
            index: Box::new(boolean()),
        },
        ty: ExprResult::Never,
    };
    let mut call = call(
        FunctionRef::Host(HostFunctionId(0)),
        "callee",
        vec![abrupt(), boolean()],
        Type::Number,
    );
    call.ty = ExprResult::Never;
    for value in [array, index, call] {
        let mut program = local_program(vec![expression(value)]);
        program.host_functions.push(host(
            0,
            "callee",
            &[Type::Number, Type::Number],
            Type::Number,
        ));
        assert_eq!(
            compile(&program),
            Err(CodegenError::TypeMismatch {
                function: "bad".into(),
                expected: Type::Number,
                found: Type::Bool,
                span: span()
            })
        );
    }
    let mut equal = bool_binary(
        abrupt(),
        BinaryOp::Equal,
        block(vec![], None, ExprResult::Void),
    );
    equal.ty = ExprResult::Never;
    assert_eq!(
        compile(&local_program(vec![expression(equal)])),
        Err(CodegenError::TypeMismatch {
            function: "bad".into(),
            expected: Type::Number,
            found: Type::Void,
            span: span()
        })
    );
}

#[test]
fn expression_local_scopes_reject_sibling_and_later_forged_reads() {
    let inside = declaration(900, Type::Number, number(), span());
    let first = block(
        vec![inside.clone()],
        Some(identifier(900, "inside", Type::Number)),
        Type::Number.into(),
    );
    let program = local_program(vec![
        expression(first),
        expression(identifier(900, "outside", Type::Number)),
    ]);
    assert_eq!(
        compile(&program),
        Err(CodegenError::UninitializedVariable {
            function: "bad".into(),
            id: VariableId(900),
            span: span()
        })
    );

    let sibling = value_if(
        boolean(),
        value_block(vec![inside], Some(number())),
        Some(value_block(
            vec![],
            Some(identifier(900, "sibling", Type::Number)),
        )),
        Type::Number.into(),
    );
    assert_eq!(
        compile(&local_program(vec![expression(sibling)])),
        Err(CodegenError::UninitializedVariable {
            function: "bad".into(),
            id: VariableId(900),
            span: span()
        })
    );
    let self_initializer = declaration(
        900,
        Type::Number,
        block(
            vec![],
            Some(identifier(900, "self", Type::Number)),
            Type::Number.into(),
        ),
        span(),
    );
    assert!(matches!(
        compile(&local_program(vec![self_initializer])),
        Err(CodegenError::UninitializedVariable {
            id: VariableId(900),
            ..
        })
    ));
}

#[test]
fn own_loop_condition_transfers_resolve_through_value_region_labels() {
    for is_break in [false, true] {
        let transfer = TypedStatement {
            kind: if is_break {
                TypedStatementKind::Break { target: LoopId(99) }
            } else {
                TypedStatementKind::Continue { target: LoopId(99) }
            },
            span: span(),
        };
        let inner = block(vec![transfer], None, ExprResult::Never);
        let cond = block(vec![], Some(inner), ExprResult::Never);
        let loop_ = TypedStatement {
            kind: TypedStatementKind::While {
                id: LoopId(99),
                cond: Box::new(cond),
                body: vec![],
            },
            span: span(),
        };
        let body = &decode(&compile(&local_program(vec![loop_])).unwrap()).bodies[0];
        assert!(body.contains(&DecodedInstruction::Br(if is_break { 3 } else { 2 })));
    }
    let loop_ = TypedStatement {
        kind: TypedStatementKind::While {
            id: LoopId(99),
            cond: Box::new(never()),
            body: vec![],
        },
        span: span(),
    };
    let mut program = local_program(vec![loop_]);
    program.functions[0].return_type = Type::Number;
    decode(&compile(&program).unwrap());
}

#[test]
fn expression_transfers_resolve_dynamic_depths_through_if_and_block_regions() {
    let transfer = |is_break| {
        block(
            vec![TypedStatement {
                kind: if is_break {
                    TypedStatementKind::Break { target: LoopId(77) }
                } else {
                    TypedStatementKind::Continue { target: LoopId(77) }
                },
                span: span(),
            }],
            None,
            ExprResult::Never,
        )
    };
    let branch = value_if(
        boolean(),
        value_block(vec![], Some(transfer(true))),
        Some(value_block(vec![], Some(transfer(false)))),
        ExprResult::Never,
    );
    let cond = block(vec![], Some(branch), ExprResult::Never);
    let loop_ = TypedStatement {
        kind: TypedStatementKind::While {
            id: LoopId(77),
            cond: Box::new(cond),
            body: vec![],
        },
        span: span(),
    };
    let module = decode(&compile(&local_program(vec![loop_])).unwrap());
    let branches = module.bodies[0]
        .iter()
        .filter_map(|op| match op {
            DecodedInstruction::Br(depth) => Some(*depth),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(branches, [4, 3, 0]);
}

#[test]
fn expression_locals_keep_outer_declaration_preorder_before_initializer_bindings() {
    let init = block(
        vec![
            declaration(2, Type::Bool, boolean(), span()),
            declaration(3, Type::Number, number(), span()),
        ],
        Some(identifier(3, "inner", Type::Number)),
        Type::Number.into(),
    );
    let mut function = user(0, "bad", &[Type::Number], Type::Void);
    function.body = vec![declaration(100, Type::Number, init, span())];
    let layout = LocalLayout::new(&function).unwrap();
    assert_eq!(layout.get(VariableId(0)).unwrap().index.raw(), 0);
    assert_eq!(layout.get(VariableId(100)).unwrap().index.raw(), 1);
    assert_eq!(layout.get(VariableId(2)).unwrap().index.raw(), 2);
    assert_eq!(layout.get(VariableId(3)).unwrap().index.raw(), 3);
    let module = decode(
        &compile(&TypedProgram {
            functions: vec![function],
            host_functions: vec![],
        })
        .unwrap(),
    );
    assert_eq!(module.locals[0], [ValType::F64, ValType::I32, ValType::F64]);
    let stores = module.bodies[0]
        .iter()
        .filter_map(|op| {
            if let DecodedInstruction::LocalSet(index) = op {
                Some(*index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(stores, [2, 3, 1]);
}

#[test]
fn value_children_preserve_independent_memory_temporary_and_remainder_planning() {
    let program = Compiler::new().analyze("fn test(a: number[]) -> void {
        let result: number = { let first = [1]; a[{ let index = [2]; 0 }] += { let rhs = [3]; 4 % 2 }; 1 };
        let text = if { let condition = [4]; false } { \"first\" } else { let hidden = [5]; \"second\" };
        return;
        { let last = [6]; \"third\" };
    }").unwrap();
    let function = &program.functions[0];
    use wasm_encoder::ValType as ScratchType;
    assert_eq!(
        super::super::temporaries::plan(function).unwrap(),
        [
            ScratchType::I32,
            ScratchType::I32,
            ScratchType::F64,
            ScratchType::I32,
            ScratchType::I32,
            ScratchType::I32,
            ScratchType::I32,
            ScratchType::I32
        ]
    );
    let memory = super::super::memory::MemoryPlan::new(&program)
        .unwrap()
        .unwrap();
    assert_eq!(memory.string("first"), Some(8));
    assert_eq!(memory.string("second"), Some(24));
    assert_eq!(memory.string("third"), Some(40));
    assert!(super::super::numeric::needs_remainder(&program));
    let module = decode(&compile(&program).unwrap());
    assert_eq!(module.functions.len(), 9);
    assert_eq!(
        &module.locals[0][8..],
        &[
            ValType::I32,
            ValType::I32,
            ValType::F64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32
        ]
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_value_blocks_preserve_original_compound_places_and_conditional_effects() {
    let bytes = Compiler::new()
        .compile_source(
            "fn main() -> number {
        let scalar = 4;
        scalar += { scalar = 100; 3 };
        let original = [10];
        let target = original;
        target[{ let index = 0; index }] += { target = [99]; original[0] = 50; 2 };
        let selected = if true { { let shadow = 5; shadow } } else { return 99; };
        let skipped = false && { return 98; };
        let once = 0;
        while { once += 1; if once < 3 { continue; } else { break; } } { return 97; }
        return scalar + original[0] + target[0] + selected + once;
    }",
        )
        .unwrap();
    run_node(
        &bytes,
        "let bytes=[]; process.stdin.on('data',b=>bytes.push(b)); process.stdin.on('end',async()=>{ const {instance}=await WebAssembly.instantiate(Buffer.concat(bytes)); const result=instance.exports.main(); if(result!==126) throw new Error('unexpected result '+result); });",
    );
}
