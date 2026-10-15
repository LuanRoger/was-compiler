use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::{BinaryOp, UnaryOp},
    codegen,
    host::HostFunctionId,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, ExprResult, FunctionId, FunctionRef, LoopId, TypedContract, TypedExpr,
        TypedExprKind, TypedNormalizedCall, TypedProgram, TypedStatementKind, TypedValueBlock,
        VariableId,
    },
};
use wasmparser::{BlockType, Operator, Parser, Payload, Validator};

fn origin() -> Span {
    Span::new(71, 93)
}
const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn expr(kind: TypedExprKind, ty: Type) -> TypedExpr {
    TypedExpr {
        kind,
        ty: ty.into(),
    }
}
fn boolean(value: bool) -> TypedExpr {
    expr(TypedExprKind::Bool(value), Type::Bool)
}
fn number(value: &str) -> TypedExpr {
    expr(TypedExprKind::Number(value.into()), Type::Number)
}
fn string(value: &str) -> TypedExpr {
    expr(TypedExprKind::String(value.into()), Type::String)
}
fn read(id: usize, ty: Type) -> TypedExpr {
    expr(
        TypedExprKind::Identifier {
            id: VariableId(id),
            name: "not_used_for_resolution".into(),
        },
        ty,
    )
}
fn binary(left: TypedExpr, op: BinaryOp, right: TypedExpr, ty: Type) -> TypedExpr {
    expr(
        TypedExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
        ty,
    )
}
fn contract(value: TypedExpr) -> TypedContract {
    TypedContract {
        value,
        span: origin(),
    }
}
fn fixture() -> TypedProgram {
    Compiler::new()
        .analyze("fn target(p: bool) -> number { let local = true; return 7; }")
        .unwrap()
}
fn compile(program: &TypedProgram) -> Result<Vec<u8>, CodegenError> {
    codegen::compile_with_exports(program, &ExportOptions::None)
}
fn operators(bytes: &[u8], index: usize) -> Vec<Operator<'_>> {
    Validator::new().validate_all(bytes).unwrap();
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| match payload.unwrap() {
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
fn calls(ops: &[Operator<'_>]) -> Vec<u32> {
    ops.iter()
        .filter_map(|op| match op {
            Operator::Call { function_index } => Some(*function_index),
            _ => None,
        })
        .collect()
}

#[test]
fn every_forbidden_contract_form_is_checked_in_skipped_and_unexported_contracts() {
    let value_block = TypedValueBlock {
        statements: vec![],
        tail: Some(Box::new(boolean(true))),
        span: origin(),
    };
    let user_call = expr(
        TypedExprKind::Call {
            func: FunctionRef::User(FunctionId(0)),
            name: "target".into(),
            args: vec![boolean(true)],
        },
        Type::Bool,
    );
    let forbidden = vec![
        user_call.clone(),
        expr(
            TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(99)),
                name: "host".into(),
                args: vec![],
            },
            Type::Bool,
        ),
        expr(
            TypedExprKind::NormalizedCall(TypedNormalizedCall {
                bindings: vec![],
                formal_to_binding: vec![],
                call: Box::new(user_call),
            }),
            Type::Bool,
        ),
        expr(TypedExprKind::Array(vec![]), Type::Bool),
        expr(
            TypedExprKind::Index {
                expr: Box::new(read(0, Type::NumberArray)),
                index: Box::new(number("0")),
            },
            Type::Bool,
        ),
        expr(
            TypedExprKind::Range {
                start: Box::new(number("0")),
                end: Box::new(number("1")),
                step: None,
                inclusive: false,
            },
            Type::Bool,
        ),
        binary(number("0"), BinaryOp::In, read(0, Type::Range), Type::Bool),
        expr(TypedExprKind::Block(value_block.clone()), Type::Bool),
        expr(
            TypedExprKind::If {
                cond: Box::new(boolean(true)),
                then: value_block.clone(),
                else_: Some(value_block),
            },
            Type::Bool,
        ),
        expr(
            TypedExprKind::Loop {
                id: LoopId(17),
                body: vec![],
            },
            Type::Bool,
        ),
        read(1, Type::Bool), // A real body-local ID is not a formal.
        read(999, Type::Bool),
    ];
    for (case, value) in forbidden.into_iter().enumerate() {
        for skipped in [false, true] {
            let mut program = fixture();
            program.functions[0].contracts = vec![
                contract(boolean(false)),
                contract(if skipped {
                    binary(boolean(false), BinaryOp::And, value.clone(), Type::Bool)
                } else {
                    value.clone()
                }),
            ];
            for level in LEVELS {
                let mut compiler = Compiler::new();
                compiler.set_exports(ExportOptions::None);
                compiler.set_optimization_level(level);
                let error = compiler.compile(&program).unwrap_err();
                assert!(
                    matches!(error, CodegenError::InvalidContractExpression { .. }),
                    "case {case}: {error}"
                );
                assert_eq!(error.span(), Some(origin()));
            }
        }
    }
}

#[test]
fn admitted_malformed_children_keep_ordinary_errors_and_contract_origins() {
    let cases = vec![
        (
            binary(number("NaN"), BinaryOp::Equal, number("0"), Type::Bool),
            "literal",
        ),
        (
            binary(
                number("0"),
                BinaryOp::Equal,
                number("9".repeat(400).as_str()),
                Type::Bool,
            ),
            "overflow",
        ),
        (
            expr(
                TypedExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr(TypedExprKind::Number("1".into()), Type::Bool)),
                },
                Type::Bool,
            ),
            "annotation",
        ),
        (
            binary(
                read(0, Type::Number),
                BinaryOp::Equal,
                number("0"),
                Type::Bool,
            ),
            "parameter type",
        ),
        (
            binary(
                expr(
                    TypedExprKind::Call {
                        func: FunctionRef::Builtin(BuiltinFunction::Len),
                        name: "len".into(),
                        args: vec![],
                    },
                    Type::Number,
                ),
                BinaryOp::Equal,
                number("0"),
                Type::Bool,
            ),
            "arity",
        ),
        (
            binary(
                expr(
                    TypedExprKind::Call {
                        func: FunctionRef::Builtin(BuiltinFunction::Len),
                        name: "len".into(),
                        args: vec![number("0")],
                    },
                    Type::Number,
                ),
                BinaryOp::Equal,
                number("0"),
                Type::Bool,
            ),
            "operand",
        ),
    ];
    for (value, case) in cases {
        let mut program = fixture();
        program.functions[0].contracts = vec![contract(binary(
            boolean(false),
            BinaryOp::And,
            value,
            Type::Bool,
        ))];
        let error = compile(&program).unwrap_err();
        assert_eq!(error.span(), Some(origin()), "{case}");
        assert!(
            match case {
                "literal" => matches!(error, CodegenError::InvalidNumberLiteral { .. }),
                "overflow" => matches!(error, CodegenError::NumberLiteralOutOfRange { .. }),
                "arity" => matches!(error, CodegenError::ArgumentCountMismatch { .. }),
                _ => matches!(
                    error,
                    CodegenError::TypeMismatch { .. } | CodegenError::ExprResultMismatch { .. }
                ),
            },
            "{case}: {error}"
        );
    }
    for ty in [
        ExprResult::Value(Type::Number),
        ExprResult::Value(Type::Void),
        ExprResult::Void,
        ExprResult::Never,
    ] {
        let mut program = fixture();
        let mut value = boolean(true);
        value.ty = ty;
        program.functions[0].contracts = vec![contract(value)];
        let error = compile(&program).unwrap_err();
        assert!(matches!(
            error,
            CodegenError::TypeMismatch { .. } | CodegenError::ExprResultMismatch { .. }
        ));
        assert_eq!(error.span(), Some(origin()));
    }
}

#[test]
fn constant_false_does_not_excuse_body_checks_or_missing_returns() {
    let mut program = fixture();
    program.functions[0].contracts = vec![contract(boolean(false))];
    program.functions[0].body.clear();
    assert!(matches!(
        compile(&program),
        Err(CodegenError::MissingReturnValue { .. })
    ));
    let mut program = fixture();
    program.functions[0].contracts = vec![contract(boolean(false))];
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[0].body[1].kind else {
        panic!("return")
    };
    *value = read(999, Type::Number);
    assert!(matches!(
        compile(&program),
        Err(CodegenError::UnknownVariable { .. })
    ));
}

#[test]
fn entry_validates_all_handle_parameters_then_ordered_contracts_then_body() {
    let mut program = Compiler::new()
        .analyze("fn guarded(s: string, a: number[], r: range, flag: bool) -> number { return 7; }")
        .unwrap();
    // Sparse identities map to formal positions, not raw Wasm local indices.
    for (param, id) in program.functions[0]
        .params
        .iter_mut()
        .zip([101, 500, 900, 77])
    {
        param.id = VariableId(id);
    }
    program.functions[0].contracts = vec![
        contract(read(77, Type::Bool)),
        contract(boolean(false)),
        contract(boolean(true)),
    ];
    let bytes = compile(&program).unwrap();
    let ops = operators(&bytes, 0);
    assert!(matches!(ops.as_slice(), [
        Operator::LocalGet { local_index: 0 }, Operator::Call { function_index: 2 }, Operator::Drop,
        Operator::LocalGet { local_index: 1 }, Operator::Call { function_index: 3 }, Operator::Drop,
        Operator::LocalGet { local_index: 2 }, Operator::Call { function_index: 9 }, Operator::Drop,
        Operator::LocalGet { local_index: 3 }, Operator::I32Eqz, Operator::If { blockty: BlockType::Empty }, Operator::Unreachable, Operator::End,
        Operator::I32Const { value: 0 }, Operator::I32Eqz, Operator::If { blockty: BlockType::Empty }, Operator::Unreachable, Operator::End,
        Operator::I32Const { value: 1 }, Operator::I32Eqz, Operator::If { blockty: BlockType::Empty }, Operator::Unreachable, Operator::End,
        Operator::F64Const { value }, Operator::Return, Operator::End,
    ] if value.bits() == 7f64.to_bits()));
    let body_count = Parser::new(0)
        .parse_all(&bytes)
        .filter(|p| matches!(p, Ok(Payload::CodeSectionEntry(_))))
        .count();
    assert_eq!(body_count, 10); // user + seven memory + construct/validate, no contains

    program.functions[0].contracts.push(contract(binary(
        binary(number("5"), BinaryOp::Remainder, number("2"), Type::Number),
        BinaryOp::Equal,
        number("1"),
        Type::Bool,
    )));
    let bytes = compile(&program).unwrap();
    // Remainder follows memory helpers and precedes both private range helpers.
    assert_eq!(calls(&operators(&bytes, 0)), [2, 3, 10, 8]);
    assert_eq!(
        Parser::new(0)
            .parse_all(&bytes)
            .filter(|p| matches!(p, Ok(Payload::CodeSectionEntry(_))))
            .count(),
        11
    );
}

#[test]
fn contract_only_remainder_plans_a_helper_without_memory_and_keeps_checks_at_all_levels() {
    let mut program = Compiler::new()
        .analyze("fn target() -> number { return 7; }")
        .unwrap();
    program.functions[0].contracts = vec![contract(binary(
        binary(number("5"), BinaryOp::Remainder, number("2"), Type::Number),
        BinaryOp::Equal,
        number("1"),
        Type::Bool,
    ))];
    let bytes = compile(&program).unwrap();
    assert_eq!(calls(&operators(&bytes, 0)), [1]);
    for payload in Parser::new(0).parse_all(&bytes) {
        assert!(!matches!(
            payload.unwrap(),
            Payload::MemorySection(_) | Payload::GlobalSection(_) | Payload::DataSection(_)
        ));
    }
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
    }
}

#[test]
fn contract_only_equality_and_len_keep_memory_helper_order_and_body_temporaries() {
    let mut program = Compiler::new().analyze("fn target(a: number[], b: number[], s: string) -> number { let body = [3]; return len(body); }").unwrap();
    let len = expr(
        TypedExprKind::Call {
            func: FunctionRef::Builtin(BuiltinFunction::Len),
            name: "len".into(),
            args: vec![read(0, Type::NumberArray)],
        },
        Type::Number,
    );
    program.functions[0].contracts = vec![
        contract(binary(
            read(0, Type::NumberArray),
            BinaryOp::Equal,
            read(1, Type::NumberArray),
            Type::Bool,
        )),
        contract(binary(
            read(2, Type::String),
            BinaryOp::Equal,
            string("entry"),
            Type::Bool,
        )),
        contract(binary(len, BinaryOp::GreaterThan, number("0"), Type::Bool)),
        contract(binary(
            binary(number("5"), BinaryOp::Remainder, number("2"), Type::Number),
            BinaryOp::Equal,
            number("1"),
            Type::Bool,
        )),
    ];
    let bytes = compile(&program).unwrap();
    assert_eq!(calls(&operators(&bytes, 0)), [3, 3, 2, 7, 6, 5, 8, 1, 5]);
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            let locals: Vec<_> = body
                .get_locals_reader()
                .unwrap()
                .into_iter()
                .map(Result::unwrap)
                .collect();
            assert_eq!(
                locals,
                [(1, wasmparser::ValType::I32), (1, wasmparser::ValType::I32)]
            );
            break;
        }
    }
}

#[test]
fn collection_inequality_contracts_complement_content_helpers_without_allocation() {
    let source = "fn target(a: number[], b: number[], s: string, t: string) -> number
        requires a != b requires s != t { return 7; }";
    let mut compiler = Compiler::new();
    let baseline = compiler.compile_source(source).unwrap();
    let ops = operators(&baseline, 0);
    assert_eq!(calls(&ops), [3, 3, 2, 2, 7, 6]);
    for (left, right, helper) in [(0, 1, 7), (2, 3, 6)] {
        assert!(ops.windows(8).any(|window| matches!(window, [
            Operator::LocalGet { local_index: a },
            Operator::LocalGet { local_index: b },
            Operator::Call { function_index },
            Operator::I32Eqz, Operator::I32Eqz,
            Operator::If { blockty: BlockType::Empty },
            Operator::Unreachable, Operator::End,
        ] if *a == left && *b == right && *function_index == helper)));
    }

    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile_source(source).unwrap(), baseline);
    }
}

#[test]
fn static_strings_are_discovered_in_contract_order_before_body_and_after_prior_functions() {
    let mut program = Compiler::new()
        .analyze(
            "fn prior() -> string { return \"prior\"; } fn target() -> string { return \"body\"; }",
        )
        .unwrap();
    program.functions[1].contracts = vec![
        contract(binary(
            string("entry"),
            BinaryOp::Equal,
            string("entry"),
            Type::Bool,
        )),
        contract(binary(
            boolean(false),
            BinaryOp::And,
            binary(
                string("hidden"),
                BinaryOp::Equal,
                string("hidden"),
                Type::Bool,
            ),
            Type::Bool,
        )),
    ];
    let bytes = compile(&program).unwrap();
    let mut expected = vec![0; 8];
    for (handle, text) in [
        (8usize, "prior"),
        (24, "entry"),
        (40, "hidden"),
        (56, "body"),
    ] {
        expected.resize(handle, 0);
        expected.extend_from_slice(&u32::try_from(handle + 8).unwrap().to_le_bytes());
        expected.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
        expected.extend_from_slice(text.as_bytes());
    }
    let data: Vec<_> = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::DataSection(r) => Some(
                r.into_iter()
                    .map(|d| d.unwrap().data.to_vec())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(data, [expected]);
    let ops = operators(&bytes, 1);
    assert_eq!(calls(&ops), [7, 7]); // two users, then StringEqual at memory offset five
    assert!(
        ops.iter()
            .any(|op| matches!(op, Operator::I32Const { value: 40 }))
    );
    assert!(matches!(
        &ops[ops.len() - 3..],
        [
            Operator::I32Const { value: 56 },
            Operator::Return,
            Operator::End
        ]
    ));
}

#[test]
fn contract_only_strings_enable_memory_without_body_or_signature_memory() {
    let mut program = Compiler::new().analyze("fn target() -> void {}").unwrap();
    program.functions[0].contracts = vec![contract(binary(
        string("entry"),
        BinaryOp::Equal,
        string("entry"),
        Type::Bool,
    ))];
    let bytes = compile(&program).unwrap();
    assert_eq!(calls(&operators(&bytes, 0)), [6]);
    assert!(
        Parser::new(0)
            .parse_all(&bytes)
            .any(|p| matches!(p, Ok(Payload::MemorySection(_))))
    );
    assert_eq!(
        Parser::new(0)
            .parse_all(&bytes)
            .filter(|p| matches!(p, Ok(Payload::CodeSectionEntry(_))))
            .count(),
        8
    );
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::DataSection(reader) = payload.unwrap() {
            let data = reader.into_iter().next().unwrap().unwrap();
            assert_eq!(
                data.data,
                [
                    0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0, 5, 0, 0, 0, b'e', b'n', b't', b'r', b'y'
                ]
            );
        }
    }
}

#[test]
fn direct_recursive_and_default_calls_share_the_same_callee_entry_checks() {
    let mut program = Compiler::new()
        .analyze(
            "fn target(n: number = 2) -> number { if n > 0 { return target(n - 1); } return n; }
         fn main() -> number { return target(); }",
        )
        .unwrap();
    program.functions[0].contracts = vec![contract(binary(
        read(0, Type::Number),
        BinaryOp::GreaterThanOrEqual,
        number("0"),
        Type::Bool,
    ))];
    let bytes = codegen::compile(&program).unwrap();
    let target_ops = operators(&bytes, 0);
    assert!(matches!(&target_ops[..7], [
        Operator::LocalGet { local_index: 0 }, Operator::F64Const { value }, Operator::F64Ge,
        Operator::I32Eqz, Operator::If { blockty: BlockType::Empty }, Operator::Unreachable, Operator::End,
    ] if value.bits() == 0f64.to_bits()));
    assert_eq!(calls(&target_ops), [0]);
    let caller_ops = operators(&bytes, 1);
    assert!(matches!(caller_ops.as_slice(), [
        Operator::F64Const { value }, Operator::LocalSet { local_index: 0 },
        Operator::LocalGet { local_index: 0 }, Operator::Call { function_index: 0 }, Operator::Return, Operator::End,
    ] if value.bits() == 2f64.to_bits()));
    for level in LEVELS {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
    }
}

#[test]
fn range_formal_reads_are_admitted_but_do_not_broaden_operator_rules() {
    let mut program = Compiler::new()
        .analyze("fn target(r: range) -> void {}")
        .unwrap();
    program.functions[0].contracts = vec![contract(binary(
        read(0, Type::Range),
        BinaryOp::Equal,
        read(0, Type::Range),
        Type::Bool,
    ))];
    let error = compile(&program).unwrap_err();
    assert!(matches!(error, CodegenError::UnsupportedOperator { .. }));
    assert_eq!(error.span(), Some(origin()));
}

#[test]
fn inferred_and_explicit_contract_calls_preserve_analyzed_and_source_bytes() {
    let inferred = "fn main() requires true { return target(c: 9, a: 3); }
        fn mixed() requires true { return target(3, c: 9); }
        fn target(a: number, b: number = a + 1, c: number = b * 2)
            requires a > 0 requires b == a + 1 requires c > b {
            return a * 100 + b * 10 + c;
        }";
    let annotated = "fn main() -> number requires true { return target(c: 9, a: 3); }
        fn mixed() -> number requires true { return target(3, c: 9); }
        fn target(a: number, b: number = a + 1, c: number = b * 2) -> number
            requires a > 0 requires b == a + 1 requires c > b {
            return a * 100 + b * 10 + c;
        }";
    let mut compiler = Compiler::new();
    let analyzed = compiler.analyze(inferred).unwrap();
    assert!(
        analyzed
            .functions
            .iter()
            .all(|f| f.return_type == Type::Number)
    );

    let baseline = compiler.compile_source(annotated).unwrap();
    Validator::new().validate_all(&baseline).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile_source(inferred).unwrap(), baseline);
        assert_eq!(compiler.compile_source(annotated).unwrap(), baseline);
        assert_eq!(compiler.compile(&analyzed).unwrap(), baseline);
    }
}

#[test]
fn divergent_argument_cannot_reach_contract_entry_but_later_defaults_are_checked() {
    let source = "fn target(a: number, b: number = a + 1) -> number
        requires false { return b; }
        fn main() -> number { return target(a: loop {}); }";
    let mut compiler = Compiler::new();
    let mut program = compiler.analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    assert_eq!(value.ty, ExprResult::Never);
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized call")
    };
    assert_eq!(sequence.call.ty, ExprResult::Value(Type::Number));

    let baseline = compiler.compile(&program).unwrap();
    let entry = operators(&baseline, 0);
    assert!(matches!(
        &entry[..5],
        [
            Operator::I32Const { value: 0 },
            Operator::I32Eqz,
            Operator::If {
                blockty: BlockType::Empty
            },
            Operator::Unreachable,
            Operator::End,
        ]
    ));
    let caller = operators(&baseline, 1);
    // The empty argument loop branches to itself; the canonical call remains
    // structurally present and checked, but cannot execute after this backedge.
    let backedge = caller
        .windows(2)
        .position(|ops| {
            matches!(
                ops,
                [
                    Operator::Loop {
                        blockty: BlockType::Empty
                    },
                    Operator::Br { relative_depth: 0 },
                ]
            )
        })
        .unwrap();
    let invocation = caller
        .iter()
        .position(|op| matches!(op, Operator::Call { function_index: 0 }))
        .unwrap();
    assert!(backedge < invocation);
    assert_eq!(calls(&caller), [0]);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), baseline);
    }

    let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &mut value.kind else {
        panic!("normalized call")
    };
    let default = &mut sequence.bindings[1];
    let span = default.span;
    default.value = number("invalid");
    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::InvalidNumberLiteral {
            function: "main".into(),
            literal: "invalid".into(),
            span,
        })
    );
}

#[test]
fn empty_contracts_preserve_bytes_and_true_checks_are_not_eliminated() {
    let mut program = fixture();
    assert!(program.functions[0].contracts.is_empty());
    let baseline = compile(&program).unwrap();
    program.functions[0].contracts.push(contract(boolean(true)));
    let checked = compile(&program).unwrap();
    assert_ne!(checked, baseline);
    assert!(matches!(
        &operators(&checked, 0)[..5],
        [
            Operator::I32Const { value: 1 },
            Operator::I32Eqz,
            Operator::If {
                blockty: BlockType::Empty
            },
            Operator::Unreachable,
            Operator::End
        ]
    ));
    program.functions[0].contracts.clear();
    assert_eq!(compile(&program).unwrap(), baseline);
}
