use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::BinaryOp,
    codegen,
    typed_ast::{
        ExprResult, LoopId, TypedBlock, TypedExpr, TypedExprKind, TypedProgram, TypedStatement,
        TypedStatementKind, VariableId,
    },
};
use wasmparser::{BlockType, FunctionBody, Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn analyze(source: &str) -> TypedProgram {
    Compiler::new()
        .analyze(source)
        .expect("valid source fixture")
}

fn for_parts(
    statement: &mut TypedStatement,
) -> (
    &mut LoopId,
    &mut VariableId,
    &mut Box<TypedExpr>,
    &mut TypedBlock,
) {
    let TypedStatementKind::For {
        id,
        variable,
        iterable,
        body,
        ..
    } = &mut statement.kind
    else {
        panic!("expected for");
    };
    (id, variable, iterable, body)
}

fn number() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number("1".into()),
        ty: Type::Number.into(),
    }
}

fn identifier(id: VariableId, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Identifier {
            id,
            name: "i".into(),
        },
        ty: ty.into(),
    }
}

fn assert_rejected(program: &TypedProgram, expected: CodegenError) {
    let snapshot = program.clone();
    let error = codegen::compile(program).unwrap_err();
    assert_eq!(error.span(), expected.span());
    assert_eq!(error, expected);

    let mut compiler = Compiler::new();
    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["main".into()]),
    ] {
        compiler.set_exports(exports);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let error = compiler.compile(program).unwrap_err();
            assert_eq!(error.span(), expected.span());
            assert_eq!(error, expected, "{level:?}");
            assert_eq!(program, &snapshot);
        }
    }
}

fn function_body(bytes: &[u8], index: usize) -> FunctionBody<'_> {
    Validator::new()
        .validate_all(bytes)
        .expect("valid checked Wasm");
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .nth(index)
        .expect("user function body")
}

fn local_types(body: &FunctionBody<'_>) -> Vec<ValType> {
    body.get_locals_reader()
        .unwrap()
        .into_iter()
        .flat_map(|local| {
            let (count, ty) = local.unwrap();
            std::iter::repeat_n(ty, usize::try_from(count).unwrap())
        })
        .collect()
}

#[test]
fn iteration_binding_and_scratch_have_exact_types_and_order() {
    let compiler = Compiler::new();
    let program = compiler
        .analyze("fn main() -> void { for i in 0..3 { i; } }")
        .unwrap();
    let snapshot = program.clone();
    let bytes = codegen::compile(&program).unwrap();
    let body = function_body(&bytes, 0);

    assert_eq!(
        local_types(&body),
        [
            ValType::F64,
            ValType::I32,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I32
        ],
        "binding, handle, current, end, step, inclusivity"
    );
    let ops = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(ops.windows(2).any(|ops| matches!(
        ops,
        [
            Operator::F64ConvertI64S,
            Operator::LocalSet { local_index: 0 }
        ]
    )));
    assert_eq!(compiler.compile(&program).unwrap(), bytes);
    assert_eq!(program, snapshot);
}

#[test]
fn nested_expression_transfers_decode_latch_and_exit_depths_not_fixed_depths() {
    let program = analyze(
        "fn main() -> void {
        for i in 0..3 {
            if true && { if false { continue; } else {} true } {
                { if true { break; } else {} };
            }
            continue;
        }
    }",
    );
    let bytes = codegen::compile(&program).unwrap();
    let body = function_body(&bytes, 0);
    let ops = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let regions = ops
        .iter()
        .filter_map(|op| match op {
            Operator::Block { blockty } => Some(("block", *blockty)),
            Operator::Loop { blockty } => Some(("loop", *blockty)),
            Operator::If { blockty } => Some(("if", *blockty)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        &regions[..3],
        &[
            ("block", BlockType::Empty),
            ("loop", BlockType::Empty),
            ("if", BlockType::Type(ValType::I32)),
        ]
    );
    assert_eq!(
        regions
            .iter()
            .copied()
            .filter(|(kind, _)| *kind != "if")
            .collect::<Vec<_>>(),
        [
            ("block", BlockType::Empty),
            ("loop", BlockType::Empty),
            ("block", BlockType::Empty),
            ("block", BlockType::Type(ValType::I32)),
            ("block", BlockType::Empty),
        ]
    );
    let branches = ops
        .iter()
        .filter_map(|op| match op {
            Operator::Br { relative_depth } => Some((false, *relative_depth)),
            Operator::BrIf { relative_depth } => Some((true, *relative_depth)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        branches,
        [(true, 1), (false, 3), (false, 5), (false, 0), (false, 0)]
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, Operator::I64LtS | Operator::I64LeS))
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, Operator::I64GtS | Operator::I64GeS))
    );
    assert!(!ops.iter().any(|op| matches!(
        op,
        Operator::F64Add
            | Operator::I64LtU
            | Operator::I64LeU
            | Operator::I64GtU
            | Operator::I64GeU
    )));
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::I64Add))
            .count(),
        1
    );
    assert!(
        ops.windows(5).any(|ops| matches!(
            ops,
            [
                Operator::LocalGet { local_index: 2 },
                Operator::LocalGet { local_index: 4 },
                Operator::I64Add,
                Operator::LocalSet { local_index: 2 },
                Operator::Br { relative_depth: 0 },
            ]
        )),
        "the latch exits before exact advancement and the backedge"
    );
}

#[test]
fn iterable_finishes_and_record_is_cached_before_for_regions_and_binding() {
    let program = analyze(
        "fn main() -> void {
        for i in { let a = [1]; for j in 0..1 { j; } 0..len(a) } { i; }
    }",
    );
    let bytes = codegen::compile(&program).unwrap();
    let body = function_body(&bytes, 0);
    assert_eq!(
        local_types(&body),
        [
            ValType::F64,
            ValType::I32,
            ValType::F64,
            ValType::I32,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I64,
            ValType::I64,
            ValType::I64,
            ValType::I32,
        ],
        "outer binding precedes iterable declarations; outer scratch precedes iterable scratch"
    );
    let ops = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 8 }))
            .count(),
        2,
        "each iterable constructs its range once"
    );
    let handles = ops
        .iter()
        .filter_map(|op| match op {
            Operator::LocalSet {
                local_index: index @ (3 | 9),
            } => Some(*index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        handles,
        [9, 3],
        "inner iterable executes before the outer iterable is cached"
    );
    let outer_cache = ops
        .iter()
        .position(|op| matches!(op, Operator::LocalSet { local_index: 3 }))
        .unwrap();
    let outer_region = ops
        .iter()
        .rposition(|op| matches!(op, Operator::Loop { .. }))
        .unwrap()
        - 1;
    let cache = &ops[outer_cache..outer_region];
    assert!(matches!(
        ops[outer_region],
        Operator::Block {
            blockty: BlockType::Empty
        }
    ));
    assert_eq!(
        cache
            .iter()
            .filter_map(|op| match op {
                Operator::LocalSet { local_index } => Some(*local_index),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [3, 4, 5, 6, 7]
    );
    assert_eq!(
        cache
            .iter()
            .filter_map(|op| match op {
                Operator::I64Load { memarg } => Some((memarg.offset, memarg.align)),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [(0, 3), (8, 3), (16, 3)]
    );
    assert!(cache.iter().any(
        |op| matches!(op, Operator::I32Load { memarg } if memarg.offset == 24 && memarg.align == 2)
    ));
    assert!(
        matches!(
            ops.get(outer_cache - 1),
            Some(Operator::Call { function_index: 9 })
        ),
        "validate the iterable handle before caching it and loading fields"
    );
    assert!(
        !cache
            .iter()
            .any(|op| matches!(op, Operator::LocalSet { local_index: 0 }))
    );
}

#[test]
fn completing_iterables_require_range_or_array_not_another_storage_type_or_void() {
    for ty in [Type::Number, Type::Bool, Type::String, Type::Void] {
        let mut program = analyze("fn hidden() -> void { for i in 0..1 {} } fn main() -> void {}");
        let span = program.functions[0].body[0].span;
        let (_, _, iterable, _) = for_parts(&mut program.functions[0].body[0]);
        **iterable = TypedExpr {
            kind: match ty {
                Type::Number => TypedExprKind::Number("1".into()),
                Type::Bool => TypedExprKind::Bool(true),
                Type::String => TypedExprKind::String("x".into()),
                Type::Array(_) => unreachable!(),
                Type::Void => TypedExprKind::Block(was_compiler::typed_ast::TypedValueBlock {
                    statements: vec![],
                    tail: None,
                    span,
                }),
                Type::Range => unreachable!(),
            },
            ty: ty.into(),
        };
        assert_rejected(
            &program,
            CodegenError::TypeMismatch {
                function: "hidden".into(),
                expected: Type::Range,
                found: ty,
                span,
            },
        );
    }
}

#[test]
fn valid_array_iterables_are_accepted_by_checked_lowering() {
    for elements in [vec![], vec![number()]] {
        let mut program = analyze("fn main() -> void { for i in 0..1 { i; } }");
        let (_, _, iterable, _) = for_parts(&mut program.functions[0].body[0]);
        **iterable = TypedExpr {
            kind: TypedExprKind::Array(elements, was_compiler::ArrayElementType::Number),
            ty: Type::Array(was_compiler::ArrayElementType::Number).into(),
        };
        let snapshot = program.clone();
        let baseline = codegen::compile(&program).expect("valid numeric array iterable");
        function_body(&baseline, 0);
        let mut compiler = Compiler::new();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let selected_bytes = compiler.compile(&program).unwrap();
            wasmparser::Validator::new()
                .validate_all(&selected_bytes)
                .unwrap();
            if level == OptimizationLevel::None {
                assert_eq!(selected_bytes, baseline);
            }
            let baseline = selected_bytes;
            assert_eq!(compiler.compile(&program).unwrap(), baseline);
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn range_annotations_do_not_disguise_numbers_or_forged_never_results() {
    for corruption in 0..4 {
        let mut program = analyze("fn hidden() -> void { for i in 0..1 {} } fn main() -> void {}");
        let span = program.functions[0].body[0].span;
        let (_, _, iterable, _) = for_parts(&mut program.functions[0].body[0]);
        let expected = match corruption {
            0 => {
                iterable.kind = TypedExprKind::Number("1".into());
                CodegenError::TypeMismatch {
                    function: "hidden".into(),
                    expected: Type::Number,
                    found: Type::Range,
                    span,
                }
            }
            1 => {
                iterable.ty = ExprResult::Never;
                CodegenError::ExprResultMismatch {
                    function: "hidden".into(),
                    expected: Type::Range.into(),
                    found: ExprResult::Never,
                    span,
                }
            }
            2 => {
                iterable.ty = ExprResult::Value(Type::Void);
                CodegenError::ExprResultMismatch {
                    function: "hidden".into(),
                    expected: Type::Range.into(),
                    found: ExprResult::Value(Type::Void),
                    span,
                }
            }
            _ => {
                iterable.ty = ExprResult::Never;
                let TypedExprKind::Range { start, .. } = &mut iterable.kind else {
                    panic!("range");
                };
                start.ty = ExprResult::Never;
                CodegenError::ExprResultMismatch {
                    function: "hidden".into(),
                    expected: Type::Number.into(),
                    found: ExprResult::Never,
                    span,
                }
            }
        };
        assert_rejected(&program, expected);
    }
}

#[test]
fn genuine_never_iterable_preserves_completion_but_still_checks_the_body() {
    for source in [
        "fn main() -> number { for i in loop {} { i; } }",
        "fn main() -> number { for i in { return 7; } { i; } }",
    ] {
        let program = analyze(source);
        let bytes = codegen::compile(&program).unwrap();
        let body = function_body(&bytes, 0);
        assert_eq!(
            local_types(&body),
            [
                ValType::F64,
                ValType::I32,
                ValType::I64,
                ValType::I64,
                ValType::I64,
                ValType::I32
            ],
            "Never retains the default range scratch shape"
        );
    }

    let original = analyze(
        "fn hidden() -> number {
        for i in { return 7; 0..1 } { i; }
    } fn main() -> void {}",
    );
    let mut valid = original.clone();
    let (_, _, iterable, _) = for_parts(&mut valid.functions[0].body[0]);
    assert_eq!(iterable.ty, ExprResult::Never);
    let bytes = codegen::compile(&valid).unwrap();
    function_body(&bytes, 0);

    let mut forged = original.clone();
    let span = forged.functions[0].body[0].span;
    let (_, _, iterable, _) = for_parts(&mut forged.functions[0].body[0]);
    iterable.ty = Type::Range.into();
    assert_rejected(
        &forged,
        CodegenError::ExprResultMismatch {
            function: "hidden".into(),
            expected: ExprResult::Never,
            found: Type::Range.into(),
            span,
        },
    );

    let mut forged = original;
    let (_, _, _, body) = for_parts(&mut forged.functions[0].body[0]);
    let span = body[0].span;
    body[0].kind = TypedStatementKind::Continue {
        target: LoopId(usize::MAX),
    };
    assert_rejected(
        &forged,
        CodegenError::InvalidLoopTarget {
            function: "hidden".into(),
            target: LoopId(usize::MAX),
            span,
        },
    );
}

#[test]
fn iteration_binding_annotations_are_checked_against_numeric_storage() {
    for iterable in ["0..1", "[1]"] {
        check_binding_annotations(iterable);
    }
}

fn check_binding_annotations(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    for ty in [
        ExprResult::Value(Type::Bool),
        ExprResult::Never,
        ExprResult::Value(Type::Void),
    ] {
        let mut program =
            analyze("fn hidden() -> void { for i in 0..1 { i; } } fn main() -> void {}");
        let (_, _, _, body) = for_parts(&mut program.functions[0].body[0]);
        let span = body[0].span;
        let TypedStatementKind::Expression(expr) = &mut body[0].kind else {
            panic!("identifier");
        };
        expr.ty = ty;
        let expected = if ty == Type::Bool {
            CodegenError::TypeMismatch {
                function: "hidden".into(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            }
        } else {
            CodegenError::ExprResultMismatch {
                function: "hidden".into(),
                expected: Type::Number.into(),
                found: ty,
                span,
            }
        };
        assert_rejected(&program, expected);
    }
}

fn read_statement(body: &mut TypedBlock) -> &mut TypedStatement {
    let last = body.last_mut().expect("read fixture");
    if matches!(
        &last.kind,
        TypedStatementKind::Expression(TypedExpr {
            kind: TypedExprKind::Identifier { .. },
            ..
        })
    ) {
        return last;
    }
    match &mut last.kind {
        TypedStatementKind::If { then, else_, .. } => {
            read_statement(else_.as_mut().unwrap_or(then))
        }
        TypedStatementKind::Expression(TypedExpr {
            kind: TypedExprKind::Binary { right, .. },
            ..
        }) => {
            let TypedExprKind::Block(block) = &mut right.kind else {
                panic!("skipped block");
            };
            read_statement(&mut block.statements)
        }

        _ => panic!("expected read statement"),
    }
}

#[test]
fn ordinary_and_all_compound_writes_reject_exact_readonly_id_and_span_everywhere() {
    for iterable in ["0..1", "[1]"] {
        check_readonly_writes(iterable);
    }
}

fn check_readonly_writes(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    for fixture in [
        "i;",
        "continue; i;",
        "break; i;",
        "return; i;",
        "if false { i; }",
        "if true {} else { i; }",
        "false && { i; true };",
    ] {
        let source = format!(
            "fn hidden() -> void {{ for i in 0..1 {{ {fixture} }} }} fn main() -> void {{}}"
        );
        let original = analyze(&source);
        for op in [
            None,
            Some(BinaryOp::Add),
            Some(BinaryOp::Subtract),
            Some(BinaryOp::Multiply),
            Some(BinaryOp::Divide),
            Some(BinaryOp::Remainder),
        ] {
            let mut program = original.clone();
            let (_, variable, _, body) = for_parts(&mut program.functions[0].body[0]);
            let id = *variable;
            let statement = read_statement(body);
            let span = statement.span;
            let TypedStatementKind::Expression(target) = &statement.kind else {
                panic!("identifier");
            };
            let target = target.clone();
            let value = number();
            statement.kind = match op {
                None => TypedStatementKind::Assignment { target, value },
                Some(op) => TypedStatementKind::CompoundAssignment { target, op, value },
            };
            assert_rejected(
                &program,
                CodegenError::ReadOnlyVariable {
                    function: "hidden".into(),
                    id,
                    span,
                },
            );
        }
    }
}

#[test]
fn inner_shadow_is_mutable_and_readonly_restriction_restores_after_its_scope() {
    for iterable in ["0..1", "[1]"] {
        check_shadow_restrictions(iterable);
    }
}

fn check_shadow_restrictions(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    let original = analyze(
        "fn hidden() -> void {
        for i in 0..1 { { let i = i; i = 2; i += 1; i -= 1; i *= 2; i /= 2; i %= 2; }; i; }
        let i = 8; i = 9; i += 1;
    } fn main() -> void {}",
    );
    let bytes = codegen::compile(&original).unwrap();
    let body = function_body(&bytes, 0);
    let ops = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::LocalSet { local_index: 1 }))
            .count(),
        7,
        "shadow initialization and six mutable assignments use a distinct slot"
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, Operator::LocalSet { local_index: 2 })),
        "a later same-named let is not marked read-only"
    );
    let mut program = original;
    let (_, variable, _, body) = for_parts(&mut program.functions[0].body[0]);
    let id = *variable;
    let statement = body.last_mut().unwrap();
    let span = statement.span;
    statement.kind = TypedStatementKind::Assignment {
        target: identifier(id, Type::Number),
        value: number(),
    };
    assert_rejected(
        &program,
        CodegenError::ReadOnlyVariable {
            function: "hidden".into(),
            id,
            span,
        },
    );
}

#[test]
fn sparse_ids_are_valid_and_function_local_without_affecting_wasm_indices() {
    for iterable in ["0..1", "[1]"] {
        check_sparse_ids(iterable);
    }
}

fn check_sparse_ids(iterable: &str) {
    let analyze = |source: &str| {
        let source = source.replace("0..1", iterable);
        let source = if iterable == "[1]" {
            source.replace("seed..3", "[seed]")
        } else {
            source
        };
        analyze(&source)
    };
    let original = analyze(
        "fn hidden(seed: number) -> void {
        for i in seed..3 { i; for j in 0..1 { j; continue; } break; }
        for k in 0..1 { k; break; }
    } fn main() -> void { for i in 0..1 { i; continue; } }",
    );
    let baseline = codegen::compile(&original).unwrap();
    let mut program = original;
    let parameter = VariableId(usize::MAX);
    let outer_variable = VariableId(9000);
    let inner_variable = VariableId(42);
    let sibling_variable = VariableId(777);
    let outer_loop = LoopId(usize::MAX);
    let inner_loop = LoopId(7000);
    let sibling_loop = LoopId(3);
    program.functions[0].params[0].id = parameter;
    let (id, variable, iterable, body) = for_parts(&mut program.functions[0].body[0]);
    *id = outer_loop;
    *variable = outer_variable;
    match &mut iterable.kind {
        TypedExprKind::Range { start, .. } => **start = identifier(parameter, Type::Number),
        TypedExprKind::Array(elements, _) => elements[0] = identifier(parameter, Type::Number),
        _ => panic!("range or array"),
    }
    body[0].kind = TypedStatementKind::Expression(identifier(outer_variable, Type::Number));
    let (id, variable, _, nested) = for_parts(&mut body[1]);
    *id = inner_loop;
    *variable = inner_variable;
    nested[0].kind = TypedStatementKind::Expression(identifier(inner_variable, Type::Number));
    nested[1].kind = TypedStatementKind::Continue { target: inner_loop };
    body[2].kind = TypedStatementKind::Break { target: outer_loop };
    let (id, variable, _, body) = for_parts(&mut program.functions[0].body[1]);
    *id = sibling_loop;
    *variable = sibling_variable;
    body[0].kind = TypedStatementKind::Expression(identifier(sibling_variable, Type::Number));
    body[1].kind = TypedStatementKind::Break {
        target: sibling_loop,
    };
    let (id, variable, _, body) = for_parts(&mut program.functions[1].body[0]);
    *id = outer_loop;
    *variable = parameter;
    body[0].kind = TypedStatementKind::Expression(identifier(parameter, Type::Number));
    body[1].kind = TypedStatementKind::Continue { target: outer_loop };
    let bytes = codegen::compile(&program).unwrap();
    function_body(&bytes, 0);
    function_body(&bytes, 1);
    assert_eq!(
        bytes, baseline,
        "semantic IDs do not become raw Wasm indices"
    );
}

#[test]
fn for_bindings_participate_in_function_wide_variable_id_uniqueness() {
    for iterable in ["0..1", "[1]"] {
        check_variable_ids(iterable);
    }
}

fn check_variable_ids(iterable: &str) {
    let analyze = |source: &str| {
        let source = source.replace("0..1", iterable);
        let source = if iterable == "[1]" {
            source.replace("0..x", "[x]")
        } else {
            source
        };
        analyze(&source)
    };
    for fixture in [
        "fn hidden(p: number) -> void { for i in 0..1 {} } fn main() -> void {}",
        "fn hidden() -> void { let p = 1; for i in 0..1 {} } fn main() -> void {}",
        "fn hidden() -> void { for p in 0..1 {} return; for i in 0..1 {} } fn main() -> void {}",
    ] {
        let mut program = analyze(fixture);
        let existing = if let Some(parameter) = program.functions[0].params.first() {
            parameter.id
        } else {
            match program.functions[0].body[0].kind {
                TypedStatementKind::Let { id, .. } => id,
                TypedStatementKind::For { variable, .. } => variable,
                _ => panic!("binding"),
            }
        };
        let statement = program.functions[0].body.last_mut().unwrap();
        let span = statement.span;
        let (_, variable, _, _) = for_parts(statement);
        *variable = existing;
        assert_rejected(
            &program,
            CodegenError::DuplicateVariableId {
                function: "hidden".into(),
                id: existing,
                span,
            },
        );
    }
    for iterable_local in [false, true] {
        let source = if iterable_local {
            "fn hidden() -> void { for i in { let x = 1; 0..x } {} } fn main() -> void {}"
        } else {
            "fn hidden() -> void { for i in 0..1 { if false { let x = 1; } } } fn main() -> void {}"
        };
        let mut program = analyze(source);
        let (_, variable, iterable, body) = for_parts(&mut program.functions[0].body[0]);
        let id = *variable;
        let statement = if iterable_local {
            let TypedExprKind::Block(block) = &mut iterable.kind else {
                panic!("block");
            };
            &mut block.statements[0]
        } else {
            let TypedStatementKind::If { then, .. } = &mut body[0].kind else {
                panic!("if");
            };
            &mut then[0]
        };
        let span = statement.span;
        let TypedStatementKind::Let { id: local, .. } = &mut statement.kind else {
            panic!("let");
        };
        *local = id;
        assert_rejected(
            &program,
            CodegenError::DuplicateVariableId {
                function: "hidden".into(),
                id,
                span,
            },
        );
    }
}

#[test]
fn loop_ids_remain_unique_in_iterable_body_and_unreachable_siblings() {
    for iterable in ["0..1", "[1]"] {
        check_loop_ids(iterable);
    }
}

fn check_loop_ids(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    for (case, source) in [
        "fn hidden() -> void { for i in { for j in 0..1 {} 0..1 } {} } fn main() -> void {}",
        "fn hidden() -> void { for i in 0..1 { if false { while false {} } } } fn main() -> void {}",
        "fn hidden() -> void { for i in 0..1 {} return; for j in 0..1 {} } fn main() -> void {}",
    ].into_iter().enumerate() {
        let mut program = analyze(source);
        let duplicate = *for_parts(&mut program.functions[0].body[0]).0;
        let repeated = if case == 2 {
            program.functions[0].body.last_mut().unwrap()
        } else {
            let (_, _, iterable, body) = for_parts(&mut program.functions[0].body[0]);
            if let TypedExprKind::Block(block) = &mut iterable.kind {
                &mut block.statements[0]
            } else {
                let TypedStatementKind::If { then, .. } = &mut body[0].kind else {
                    panic!("if");
                };
                &mut then[0]
            }
        };
        let span = repeated.span;
        match &mut repeated.kind {
            TypedStatementKind::For { id, .. } | TypedStatementKind::While { id, .. } => {
                *id = duplicate
            }
            _ => panic!("loop"),
        }
        assert_rejected(
            &program,
            CodegenError::DuplicateLoopId {
                function: "hidden".into(),
                id: duplicate,
                span,
            },
        );
    }
}

#[test]
fn both_transfers_reject_unknown_inactive_outer_and_cross_function_targets() {
    for iterable in ["0..1", "[1]"] {
        check_transfer_targets(iterable);
    }
}

fn check_transfer_targets(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    let original = analyze(
        "fn hidden() -> void {
        for i in 0..1 { for j in 0..1 { continue; } }
    } fn main() -> void { for k in 0..1 {} }",
    );
    for is_break in [false, true] {
        for corruption in 0..4 {
            let mut program = original.clone();
            let foreign = LoopId(123456);
            *for_parts(&mut program.functions[1].body[0]).0 = foreign;
            let (outer, _, _, body) = for_parts(&mut program.functions[0].body[0]);
            let outer = *outer;
            let (inner, _, _, nested) = for_parts(&mut body[0]);
            let inner = *inner;
            let target = match corruption {
                0 => LoopId(usize::MAX),
                1 => outer,
                2 => foreign,
                _ => inner,
            };
            let mut transfer = nested[0].clone();
            let span = transfer.span;
            transfer.kind = if is_break {
                TypedStatementKind::Break { target }
            } else {
                TypedStatementKind::Continue { target }
            };
            if corruption == 3 {
                body.push(transfer);
            } else {
                nested[0] = transfer;
            }
            assert_rejected(
                &program,
                CodegenError::InvalidLoopTarget {
                    function: "hidden".into(),
                    target,
                    span,
                },
            );
        }
    }
}

#[test]
fn iterable_transfers_use_the_enclosing_loop_never_the_new_for() {
    for iterable in ["0..1", "[1]"] {
        check_iterable_transfers(iterable);
    }
}

fn check_iterable_transfers(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    for keyword in ["break", "continue"] {
        let source = format!(
            "fn hidden() -> void {{ while true {{
            for i in {{ {keyword}; 0..1 }} {{}}
        }} }} fn main() -> void {{}}"
        );
        let original = analyze(&source);
        let bytes = codegen::compile(&original).unwrap();
        function_body(&bytes, 0);
        let mut program = original;
        let TypedStatementKind::While { body, .. } = &mut program.functions[0].body[0].kind else {
            panic!("while");
        };
        let (id, _, iterable, _) = for_parts(&mut body[0]);
        let target = *id;
        let TypedExprKind::Block(block) = &mut iterable.kind else {
            panic!("block");
        };
        let span = block.statements[0].span;
        block.statements[0].kind = if keyword == "break" {
            TypedStatementKind::Break { target }
        } else {
            TypedStatementKind::Continue { target }
        };
        assert_rejected(
            &program,
            CodegenError::InvalidLoopTarget {
                function: "hidden".into(),
                target,
                span,
            },
        );
    }
}

#[test]
fn iteration_binding_is_uninitialized_in_its_iterable_and_after_its_scope() {
    for iterable in ["0..1", "[1]"] {
        check_binding_initialization(iterable);
    }
}

fn check_binding_initialization(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    for during_iterable in [true, false] {
        let mut program =
            analyze("fn hidden() -> void { for i in 0..1 { i; } 1; } fn main() -> void {}");
        let for_span = program.functions[0].body[0].span;
        let (_, variable, iterable, _) = for_parts(&mut program.functions[0].body[0]);
        let id = *variable;
        let span = if during_iterable {
            match &mut iterable.kind {
                TypedExprKind::Range { start, .. } => **start = identifier(id, Type::Number),
                TypedExprKind::Array(elements, _) => elements[0] = identifier(id, Type::Number),
                _ => panic!("range or array"),
            }
            for_span
        } else {
            let statement = &mut program.functions[0].body[1];
            statement.kind = TypedStatementKind::Expression(identifier(id, Type::Number));
            statement.span
        };
        assert_rejected(
            &program,
            CodegenError::UninitializedVariable {
                function: "hidden".into(),
                id,
                span,
            },
        );
    }
}

#[test]
fn wrong_kind_value_breaks_are_rejected_even_when_unreachable_or_rhs_is_never() {
    for iterable in ["0..1", "[1]"] {
        check_value_breaks(iterable);
    }
}

fn check_value_breaks(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    for prefix in ["", "continue;", "break;", "return;"] {
        let source = format!(
            "fn hidden() -> void {{ for i in 0..1 {{ {prefix} i; }} }} fn main() -> void {{}}"
        );
        let original = analyze(&source);
        for never in [false, true] {
            let mut program = original.clone();
            let (id, _, _, body) = for_parts(&mut program.functions[0].body[0]);
            let target = *id;
            let statement = body.last_mut().unwrap();
            let span = statement.span;
            let value = if never {
                TypedExpr {
                    kind: TypedExprKind::Loop {
                        id: LoopId(999),
                        body: vec![],
                    },
                    ty: ExprResult::Never,
                }
            } else {
                number()
            };
            statement.kind = TypedStatementKind::BreakValue { target, value };
            assert_rejected(
                &program,
                CodegenError::BreakValueNotAllowed {
                    function: "hidden".into(),
                    span,
                },
            );
        }
    }
}

#[test]
fn normally_completing_iterable_does_not_make_body_return_definite() {
    for iterable in ["0..1", "[1]"] {
        check_body_return(iterable);
    }
}

fn check_body_return(iterable: &str) {
    let analyze = |source: &str| analyze(&source.replace("0..1", iterable));
    let mut program = analyze(
        "fn hidden() -> number { for i in 0..1 { return i; } return 7; } fn main() -> void {}",
    );
    program.functions[0].body.pop();
    assert_rejected(
        &program,
        CodegenError::MissingReturnValue {
            function: "hidden".into(),
            return_type: Type::Number,
        },
    );
}

#[test]
fn planning_keeps_preorder_slots_for_nested_unreachable_and_short_circuited_fors() {
    let program = analyze(
        "fn main() -> void {
        for i in { let a = []; for j in 0..1 {} 0..1 } {
            let b = [1]; b[0] += len([2]);
        }
        false && { for k in 0..1 {} true };
        return;
        for n in 0..1 { let dead = [3]; }
    }",
    );
    let snapshot = program.clone();
    let expected_locals = [
        ValType::F64,
        ValType::I32,
        ValType::F64,
        ValType::I32,
        ValType::F64,
        ValType::F64,
        ValType::I32,
        ValType::I32,
        ValType::I64,
        ValType::I64,
        ValType::I64,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I64,
        ValType::I64,
        ValType::I64,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::F64,
        ValType::I32,
        ValType::I32,
        ValType::I64,
        ValType::I64,
        ValType::I64,
        ValType::I32,
        ValType::I32,
        ValType::I64,
        ValType::I64,
        ValType::I64,
        ValType::I32,
        ValType::I32,
    ];
    let baseline = codegen::compile(&program).unwrap();
    let mut compiler = Compiler::new();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(bytes, baseline);
        }
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        let body = function_body(&bytes, 0);
        assert_eq!(local_types(&body), expected_locals);
        let ops = body
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let induction_updates = ops
            .windows(2)
            .filter_map(|ops| match ops {
                [Operator::I64Add, Operator::LocalSet { local_index }] => Some(*local_index),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            induction_updates,
            [14, 8, 23, 28],
            "distinct reserved slots, emitted in structural child order"
        );
        assert_eq!(program, snapshot);
    }
}

#[test]
fn array_annotations_do_not_disguise_numbers_or_forged_never_results() {
    for corruption in 0..4 {
        let mut program = analyze("fn hidden() -> void { for i in [1] {} } fn main() -> void {}");
        let span = program.functions[0].body[0].span;
        let (_, _, iterable, _) = for_parts(&mut program.functions[0].body[0]);
        let expected = match corruption {
            0 => {
                iterable.kind = TypedExprKind::Number("1".into());
                CodegenError::TypeMismatch {
                    function: "hidden".into(),
                    expected: Type::Number,
                    found: Type::Array(was_compiler::ArrayElementType::Number),
                    span,
                }
            }
            1 | 2 => {
                iterable.ty = if corruption == 1 {
                    ExprResult::Never
                } else {
                    ExprResult::Value(Type::Void)
                };
                CodegenError::ExprResultMismatch {
                    function: "hidden".into(),
                    expected: if corruption == 1 {
                        Type::Array(was_compiler::ArrayElementType::Number).into()
                    } else {
                        Type::Range.into()
                    },
                    found: iterable.ty,
                    span,
                }
            }
            _ => {
                let TypedExprKind::Array(elements, was_compiler::ArrayElementType::Number) =
                    &mut iterable.kind
                else {
                    panic!("array");
                };
                elements[0].ty = ExprResult::Never;
                iterable.ty = ExprResult::Never;
                CodegenError::ExprResultMismatch {
                    function: "hidden".into(),
                    expected: Type::Number.into(),
                    found: ExprResult::Never,
                    span,
                }
            }
        };
        assert_rejected(&program, expected);
    }
}

#[test]
fn divergent_array_iterable_still_checks_hidden_body_and_annotation() {
    let original =
        analyze("fn hidden() -> number { for i in { return 7; [1] } { i; } } fn main() -> void {}");
    let bytes = codegen::compile(&original).unwrap();
    function_body(&bytes, 0);
    let mut forged = original.clone();
    let span = forged.functions[0].body[0].span;
    for_parts(&mut forged.functions[0].body[0]).2.ty =
        Type::Array(was_compiler::ArrayElementType::Number).into();
    assert_rejected(
        &forged,
        CodegenError::ExprResultMismatch {
            function: "hidden".into(),
            expected: ExprResult::Never,
            found: Type::Array(was_compiler::ArrayElementType::Number).into(),
            span,
        },
    );
    let mut forged = original;
    let (_, _, _, body) = for_parts(&mut forged.functions[0].body[0]);
    let span = body[0].span;
    body[0].kind = TypedStatementKind::Continue {
        target: LoopId(usize::MAX),
    };
    assert_rejected(
        &forged,
        CodegenError::InvalidLoopTarget {
            function: "hidden".into(),
            target: LoopId(usize::MAX),
            span,
        },
    );
}
