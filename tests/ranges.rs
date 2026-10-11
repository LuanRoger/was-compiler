#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, HostRegistrationError, OptimizationLevel,
    Type,
    ast::{BinaryOp, Expr, StatementKind},
    lexer::Lexer,
    parser::Parser as SourceParser,
    typed_ast::{ExprResult, TypedExpr, TypedExprKind, TypedProgram, TypedStatementKind},
};
use wasmparser::{BlockType, ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const INTERNAL_SOURCE: &str = r#"
    fn identity(r: range) -> range { return r; }
    fn make(a: number, b: number, s: number) -> range { return a..=b by s; }
    fn choose(flag: bool) -> range {
        return if flag { { let r = 1..4; identity(r) } }
               else { loop { break make(9, 1, -2); } };
    }
    fn main(flag: bool) -> number {
        let r: range = choose(flag);
        let copy = r;
        copy = identity(r);
        return 17;
    }
    fn hidden() -> void { return; 5..5 by 2; }
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    compiler
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let bytes = compiler.compile_source(source).expect("compile ranges");
    Validator::new()
        .validate_all(&bytes)
        .expect("valid range Wasm");
    bytes
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Decoded {
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    bodies: Vec<Vec<u8>>,
    calls: Vec<Vec<u32>>,
    sections: Vec<(u8, Vec<u8>)>,
}

fn decode(bytes: &[u8]) -> Decoded {
    Validator::new().validate_all(bytes).unwrap();
    let mut decoded = Decoded::default();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.unwrap();
        if let Some((id, range)) = payload.as_section() {
            assert_ne!(id, 0, "no implicit range marshaling metadata");
            if id != 7 {
                let start = usize::try_from(range.start).expect("section start fits usize");
                let end = usize::try_from(range.end).expect("section end fits usize");
                decoded.sections.push((id, bytes[start..end].to_vec()));
            }
        }
        match payload {
            Payload::TypeSection(section) => {
                for signature in section.into_iter_err_on_gc_types() {
                    let signature = signature.unwrap();
                    decoded
                        .signatures
                        .push((signature.params().to_vec(), signature.results().to_vec()));
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import.unwrap();
                    let TypeRef::Func(index) = import.ty else {
                        panic!("no implicit nonfunction imports");
                    };
                    decoded
                        .imports
                        .push((import.module.into(), import.name.into(), index));
                }
            }
            Payload::FunctionSection(section) => {
                decoded
                    .functions
                    .extend(section.into_iter().map(Result::unwrap));
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    decoded
                        .exports
                        .push((export.name.into(), export.kind, export.index));
                }
            }
            Payload::CodeSectionEntry(body) => {
                decoded.bodies.push(body.as_bytes().to_vec());
                decoded.calls.push(
                    body.get_operators_reader()
                        .unwrap()
                        .into_iter()
                        .filter_map(|operator| match operator.unwrap() {
                            Operator::Call { function_index } => Some(function_index),
                            _ => None,
                        })
                        .collect(),
                );
            }
            _ => {}
        }
    }
    assert_eq!(decoded.functions.len(), decoded.bodies.len());
    decoded
}

#[test]
fn syntax_preserves_numeric_tokens_arithmetic_precedence_and_contextual_by() {
    for (expression, inclusive, explicit_step) in [
        ("0..10", false, false),
        ("0..=10", true, false),
        ("1.25..2.5 by -0.5", false, true),
        ("1 + 2..=4 * 5 by 6 - 7", true, true),
        ("0/* start */..=/* end */10 by/* step */2", true, true),
    ] {
        let source = format!("fn main() -> void {{ let r: range = {expression}; }}");
        let tokens = Lexer::new(&source).tokenize();
        let raw = SourceParser::new(&tokens).parse_program().unwrap();
        let StatementKind::Let {
            ty,
            value:
                Expr::Range {
                    start,
                    end,
                    step,
                    inclusive: found,
                },
            ..
        } = &raw.functions[0].body[0].kind
        else {
            panic!("expected raw range: {expression}");
        };
        assert_eq!(*ty, Some(Type::Range));
        assert_eq!(*found, inclusive);
        assert_eq!(step.is_some(), explicit_step);
        if expression == "0..10" {
            assert_eq!(**start, Expr::Number("0".into()));
            assert_eq!(**end, Expr::Number("10".into()));
        }
        if expression == "1 + 2..=4 * 5 by 6 - 7" {
            assert!(matches!(
                **start,
                Expr::Binary {
                    op: BinaryOp::Add,
                    ..
                }
            ));
            assert!(matches!(
                **end,
                Expr::Binary {
                    op: BinaryOp::Multiply,
                    ..
                }
            ));
            assert!(matches!(
                step.as_deref(),
                Some(Expr::Binary {
                    op: BinaryOp::Subtract,
                    ..
                })
            ));
        }
        let compiler = Compiler::new();
        let typed = compiler.analyze(&source).unwrap();
        let TypedStatementKind::Let { ty, value, .. } = &typed.functions[0].body[0].kind else {
            panic!("expected typed binding");
        };
        assert_eq!(*ty, Type::Range);
        assert_eq!(value.ty, ExprResult::Value(Type::Range));
        compile(&compiler, &source);
    }
    // `by` is contextual, not a globally reserved name.
    compile(
        &Compiler::new(),
        "fn by(by: number) -> number { return by; } fn main() -> void { let by = 2; let r = 0..10 by by; }",
    );
}

#[test]
fn malformed_ranges_chains_and_legacy_malformed_decimals_are_parse_errors() {
    for expression in [
        "0..",
        "..10",
        "0..=",
        "0..10 by",
        "0..10..20",
        "0..=10..=20",
        "0..10 by 2 by 3",
        "1 by 2",
        ".5",
        "1.",
        "0...10",
        "0..==10",
    ] {
        let source = format!("fn main() -> void {{ let r = {expression}; }}");
        assert!(
            matches!(
                Compiler::new().analyze(&source),
                Err(CompileError::Parse(_))
            ),
            "{source}"
        );
    }
}

#[test]
fn wrong_operands_equality_and_range_misuse_are_static_errors_even_when_hidden() {
    for body in [
        "true..4;",
        "0..false;",
        "0..4 by true;",
        "\"a\"..4;",
        "0..[4];",
        "0..4 by [];",
        "(0..2)..4;",
        "0..(2..4);",
        "let r: number = 0..4;",
        "let r = 0..4; r = 2;",
        "let r = 0..4; r += 1;",
        "let r = 0..4; r[0];",
        "len(0..4);",
        "(0..4) + (0..4);",
        "(0..4) == (0..4);",
        "(0..4) != (0..4);",
        "let r = if true { 0..4 } else { 1 };",
        "let r = 0..4; r(1);",
        "missing..4;",
        "0..missing;",
        "0..4 by missing;",
    ] {
        for prefix in ["", "return;"] {
            let source =
                format!("fn hidden() -> void {{ {prefix} {body} }} fn main() -> void {{}}");
            let mut compiler = compiler();
            for level in LEVELS {
                compiler.set_optimization_level(level);
                for error in [
                    compiler.analyze(&source).unwrap_err(),
                    compiler.compile_source(&source).unwrap_err(),
                ] {
                    assert!(
                        matches!(error, CompileError::Semantic(_)),
                        "{source}: {error:?}"
                    );
                    assert!(error.span().is_some(), "{source}: {error:?}");
                }
            }
        }
    }
    for body in [
        "false && { true..4; true };",
        "if true { } else { 0..4 by false; }",
    ] {
        let source = format!("fn main() -> void {{ {body} }}");
        assert!(matches!(
            Compiler::new().analyze(&source),
            Err(CompileError::Semantic(_))
        ));
    }
}

#[test]
fn constant_invalid_numbers_are_runtime_constructions_not_static_range_errors() {
    for expression in [
        "0.5..1",
        "0..1.5",
        "0..1 by 0.5",
        "0..0 by 0",
        "9..1 by -0",
        "9007199254740992..0",
        "0..-9007199254740992",
        "0..1 by 9007199254740992",
        "(0 / 0)..0",
        "0..(1 / 0)",
        "0..0 by (-1 / 0)",
    ] {
        let source = format!("fn main() -> void {{ {expression}; }}");
        for level in LEVELS {
            let mut compiler = Compiler::new();
            compiler.set_optimization_level(level);
            compile(&compiler, &source);
        }
    }
}

#[test]
fn owned_pipeline_snapshots_export_selection_and_optimization_preserve_all_bodies() {
    let mut compiler = compiler();
    compiler
        .register_host_function("mark", "effects", "mark", vec![Type::Number], Type::Void)
        .unwrap();
    compiler
        .register_host_function("unused", "other", "unused", vec![], Type::Bool)
        .unwrap();
    let program = compiler.analyze(INTERNAL_SOURCE).unwrap();
    let snapshot = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    let decoded = decode(&baseline);
    assert_eq!(
        decoded
            .imports
            .iter()
            .map(|(m, n, _)| (m.as_str(), n.as_str()))
            .collect::<Vec<_>>(),
        [("effects", "mark"), ("other", "unused")]
    );
    assert_eq!(
        decoded.signatures[usize::try_from(decoded.functions[0]).unwrap()],
        (vec![ValType::I32], vec![ValType::I32])
    );
    assert_eq!(
        decoded.signatures[usize::try_from(decoded.functions[1]).unwrap()],
        (vec![ValType::F64; 3], vec![ValType::I32])
    );
    assert_eq!(
        decoded.signatures[usize::try_from(decoded.functions[2]).unwrap()],
        (vec![ValType::I32], vec![ValType::I32])
    );
    assert_eq!(
        decoded.functions[0], decoded.functions[2],
        "internal signatures deduplicate"
    );
    assert_eq!(
        decoded
            .exports
            .iter()
            .filter(|(_, k, _)| *k == ExternalKind::Func)
            .map(|(n, _, i)| (n.as_str(), *i))
            .collect::<Vec<_>>(),
        [("main", 5), ("__alloc", 7)]
    );
    assert!(
        decoded
            .exports
            .contains(&("memory".into(), ExternalKind::Memory, 0))
    );
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.analyze(INTERNAL_SOURCE).unwrap(), snapshot);
        assert_eq!(compiler.compile_source(INTERNAL_SOURCE).unwrap(), baseline);
        assert_eq!(compiler.compile(&program).unwrap(), baseline);
        assert_eq!(program, snapshot);
    }
    compiler
        .register_host_function("later", "later", "added", vec![], Type::Void)
        .unwrap();
    assert_ne!(compiler.compile_source(INTERNAL_SOURCE).unwrap(), baseline);
    for (selection, names) in [
        (ExportOptions::None, vec![]),
        (
            ExportOptions::Only(vec!["hidden".into(), "main".into()]),
            vec!["main", "hidden"],
        ),
    ] {
        compiler.set_exports(selection);
        let expected = compiler.compile(&program).unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            let actual = decode(&bytes);
            assert_eq!(bytes, expected);
            assert_eq!(actual.sections, decoded.sections);
            assert_eq!(
                actual
                    .exports
                    .iter()
                    .filter(|(name, kind, _)| *kind == ExternalKind::Func && name != "__alloc")
                    .map(|(name, _, _)| name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            assert!(
                actual
                    .exports
                    .contains(&("memory".into(), ExternalKind::Memory, 0))
            );
            assert!(
                actual
                    .exports
                    .contains(&("__alloc".into(), ExternalKind::Func, 7))
            );
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn only_selected_range_signatures_are_rejected_but_every_host_snapshot_is_checked() {
    let mut compiler = compiler();
    let program = compiler.analyze(INTERNAL_SOURCE).unwrap();
    for selection in [
        ExportOptions::All,
        ExportOptions::Only(vec!["identity".into()]),
        ExportOptions::Only(vec!["make".into()]),
    ] {
        compiler.set_exports(selection);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let error = compiler.compile(&program).unwrap_err();
            assert!(
                matches!(error, CodegenError::RangeExport { .. }),
                "{error:?}"
            );
            assert!(error.to_string().contains("export selection"));
        }
    }
    for (params, result) in [(vec![Type::Range], Type::Void), (vec![], Type::Range)] {
        assert_eq!(
            compiler.register_host_function("bad", "env", "bad", params, result),
            Err(HostRegistrationError::UnsupportedType {
                name: "bad".into(),
                ty: Type::Range
            })
        );
    }
    compiler
        .register_host_function("unused", "env", "unused", vec![Type::Number], Type::Number)
        .unwrap();
    let original = compiler.analyze(INTERNAL_SOURCE).unwrap();
    for parameter in [true, false] {
        let mut forged = original.clone();
        if parameter {
            forged.host_functions[0].params[0] = Type::Range;
        } else {
            forged.host_functions[0].return_type = Type::Range;
        }
        let snapshot = forged.clone();
        for selection in [
            ExportOptions::None,
            ExportOptions::Only(vec!["main".into()]),
        ] {
            compiler.set_exports(selection);
            for level in LEVELS {
                compiler.set_optimization_level(level);
                assert!(matches!(
                    compiler.compile(&forged),
                    Err(CodegenError::UnsupportedAbiType {
                        ty: Type::Range,
                        ..
                    })
                ));
                assert_eq!(forged, snapshot);
            }
        }
    }
}

#[test]
fn internal_range_signatures_alone_require_memory_and_parameter_validation() {
    let source = "fn consume(r: range) -> number { return 1; } fn main() -> void {}";
    let mut compiler = compiler();
    let program = compiler.analyze(source).unwrap();
    for selection in [
        ExportOptions::None,
        ExportOptions::Only(vec!["main".into()]),
    ] {
        compiler.set_exports(selection);
        let baseline = compiler.compile(&program).unwrap();
        let decoded = decode(&baseline);
        assert!(
            decoded
                .exports
                .contains(&("memory".into(), ExternalKind::Memory, 0))
        );
        assert!(
            decoded
                .exports
                .contains(&("__alloc".into(), ExternalKind::Func, 2))
        );
        assert_eq!(
            decoded.signatures[usize::try_from(decoded.functions[0]).unwrap()],
            (vec![ValType::I32], vec![ValType::F64])
        );
        assert!(
            !decoded.calls[0].is_empty(),
            "unused internal range parameters are checked"
        );
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap(), baseline);
        }
    }
    compiler.set_exports(ExportOptions::Only(vec!["consume".into()]));
    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::RangeExport {
            function: "consume".into()
        })
    );
}

#[test]
fn decoded_record_stores_and_mvp_regions_use_exact_integers_and_i32_handles() {
    let bytes = compile(&compiler(), INTERNAL_SOURCE);
    let decoded = decode(&bytes);
    let helper_count = decoded.functions.len();
    assert_eq!(
        decoded.signatures[usize::try_from(decoded.functions[helper_count - 2]).unwrap()],
        (
            vec![ValType::F64, ValType::F64, ValType::F64, ValType::I32],
            vec![ValType::I32]
        )
    );
    assert_eq!(
        decoded.signatures[usize::try_from(decoded.functions[helper_count - 1]).unwrap()],
        (vec![ValType::I32], vec![ValType::I32])
    );
    let mut stores = Vec::new();
    let mut regions = Vec::new();
    let mut conversions = 0;
    let mut body_index = 0;
    let mut validation_ops = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            for operator in body.get_operators_reader().unwrap() {
                let operator = operator.unwrap();
                if body_index == helper_count - 1 {
                    validation_ops.push(operator.clone());
                }
                match operator {
                    Operator::I64Store { memarg } => stores.push((64, memarg.offset, memarg.align)),
                    Operator::I32Store { memarg } => stores.push((32, memarg.offset, memarg.align)),
                    Operator::I64TruncF64S => conversions += 1,
                    Operator::If { blockty } | Operator::Block { blockty } if body_index == 2 => {
                        regions.push(blockty)
                    }
                    Operator::Loop { blockty } if body_index == 2 => {
                        assert_eq!(blockty, BlockType::Empty)
                    }
                    _ => {}
                }
            }
            body_index += 1;
        }
    }
    for expected in [
        (64, 0, 3),
        (64, 8, 3),
        (64, 16, 3),
        (32, 24, 2),
        (32, 28, 2),
    ] {
        assert!(
            stores.contains(&expected),
            "missing independent record field {expected:?}: {stores:?}"
        );
    }
    assert!(conversions >= 3, "validated inputs convert to signed i64");
    assert!(
        validation_ops
            .iter()
            .any(|op| matches!(op, Operator::MemorySize { mem: 0 }))
    );
    assert!(
        validation_ops
            .iter()
            .any(|op| matches!(op, Operator::I64ExtendI32U))
    );
    assert!(
        validation_ops
            .iter()
            .any(|op| matches!(op, Operator::I64Const { value: 32 }))
    );
    assert!(
        validation_ops
            .iter()
            .any(|op| matches!(op, Operator::I64Add))
    );
    assert!(
        validation_ops
            .iter()
            .any(|op| matches!(op, Operator::I64GtU))
    );
    for offset in [0, 8, 16] {
        assert!(
            validation_ops
                .iter()
                .any(|op| matches!(op, Operator::I64Load { memarg } if memarg.offset == offset))
        );
    }
    for offset in [24, 28] {
        assert!(
            validation_ops
                .iter()
                .any(|op| matches!(op, Operator::I32Load { memarg } if memarg.offset == offset))
        );
    }
    assert!(
        regions
            .iter()
            .filter(|ty| **ty == BlockType::Type(ValType::I32))
            .count()
            >= 2
    );
}

#[test]
fn extension_helpers_follow_legacy_memory_and_remainder_without_reordering() {
    let baseline = r#"
        fn main() -> void { [1]; 9 % 2; }
        fn hidden() -> void { return; [1]; }
    "#;
    let ranges = baseline.replace("return; [1];", "return; 1..4 by 2;");
    let compiler = Compiler::new();
    let before = decode(&compile(&compiler, baseline));
    let after = decode(&compile(&compiler, &ranges));
    // Two users, the seven existing memory helpers, then floating remainder.
    assert_eq!(before.bodies.len(), 10);
    assert!(
        after.bodies.len() > 10,
        "hidden range construction still plans its helpers"
    );
    assert_eq!(&after.bodies[2..10], &before.bodies[2..10]);
    assert_eq!(after.exports, before.exports);
    assert_eq!(after.imports, before.imports);
    assert_eq!(after.calls[0], before.calls[0]);
    assert!(after.calls[1].iter().any(|index| *index >= 10));
    let scalar = decode(&compile(&compiler, "fn main() -> number { return 1; }"));
    assert_eq!(scalar.bodies.len(), 1);
    assert!(
        !scalar
            .sections
            .iter()
            .any(|(id, _)| [5, 6, 11].contains(id))
    );
}

fn range_expression(program: &mut TypedProgram) -> &mut TypedExpr {
    let statement = program.functions[0].body.last_mut().unwrap();
    let TypedStatementKind::Expression(value) = &mut statement.kind else {
        panic!("expected range expression");
    };
    value
}

fn assert_forged_rejected(compiler: &mut Compiler, program: &TypedProgram) {
    let snapshot = program.clone();
    for selection in [
        ExportOptions::None,
        ExportOptions::Only(vec!["main".into()]),
    ] {
        compiler.set_exports(selection);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let error = compiler.compile(program).unwrap_err();
            assert!(
                error.span().is_some(),
                "forged expression must retain a location: {error:?}"
            );
            assert_eq!(*program, snapshot);
        }
    }
}

#[test]
fn forged_range_result_categories_and_each_wrong_child_are_checked_in_hidden_code() {
    for prefix in ["", "return;"] {
        let source =
            format!("fn hidden() -> void {{ {prefix} 1..4 by 2; }} fn main() -> void {{}}");
        let mut compiler = compiler();
        let original = compiler.analyze(&source).unwrap();
        for result in [
            Type::Number.into(),
            Type::Bool.into(),
            ExprResult::Void,
            ExprResult::Never,
            ExprResult::Value(Type::Void),
        ] {
            let mut forged = original.clone();
            range_expression(&mut forged).ty = result;
            assert_forged_rejected(&mut compiler, &forged);
        }
        for position in 0..3 {
            for corruption in 0..3 {
                let mut forged = original.clone();
                let TypedExprKind::Range {
                    start, end, step, ..
                } = &mut range_expression(&mut forged).kind
                else {
                    panic!("expected range");
                };
                let child = match position {
                    0 => start.as_mut(),
                    1 => end.as_mut(),
                    _ => step.as_deref_mut().unwrap(),
                };
                match corruption {
                    0 => {
                        *child = TypedExpr {
                            kind: TypedExprKind::Bool(true),
                            ty: Type::Bool.into(),
                        }
                    }
                    // A numeric annotation must not conceal a nonnumeric node.
                    1 => child.kind = TypedExprKind::Bool(true),
                    _ => child.ty = ExprResult::Never,
                }
                assert_forged_rejected(&mut compiler, &forged);
            }
        }
    }
}

#[test]
fn forged_range_equality_and_invalid_literals_are_checked_left_to_right() {
    for op in [BinaryOp::Equal, BinaryOp::NotEqual] {
        for prefix in ["", "return;"] {
            let source = format!("fn hidden() -> void {{ {prefix} 1..4; }} fn main() -> void {{}}");
            let mut compiler = compiler();
            let mut forged = compiler.analyze(&source).unwrap();
            let range = range_expression(&mut forged).clone();
            *range_expression(&mut forged) = TypedExpr {
                kind: TypedExprKind::Binary {
                    left: Box::new(range.clone()),
                    op,
                    right: Box::new(range),
                },
                ty: Type::Bool.into(),
            };
            assert_forged_rejected(&mut compiler, &forged);
        }
    }
    for position in 0..3 {
        let mut compiler = compiler();
        let mut forged = compiler
            .analyze("fn hidden() -> void { return; 1..4 by 2; } fn main() -> void {}")
            .unwrap();
        let span = forged.functions[0].body.last().unwrap().span;
        let TypedExprKind::Range {
            start, end, step, ..
        } = &mut range_expression(&mut forged).kind
        else {
            panic!("expected range");
        };
        for (index, child) in [start.as_mut(), end.as_mut(), step.as_deref_mut().unwrap()]
            .into_iter()
            .enumerate()
        {
            if index >= position {
                child.kind = TypedExprKind::Number(["NaN", "Infinity", "bad"][index].into());
            }
        }
        let snapshot = forged.clone();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(
                compiler.compile(&forged),
                Err(CodegenError::InvalidNumberLiteral {
                    function: "hidden".into(),
                    literal: ["NaN", "Infinity", "bad"][position].into(),
                    span,
                })
            );
            assert_eq!(forged, snapshot);
        }
    }
}

#[test]
fn overflowing_literals_in_range_children_retain_codegen_errors_and_statement_spans() {
    let overflow = "9".repeat(400);
    for expression in [
        format!("{overflow}..4"),
        format!("0..{overflow}"),
        format!("0..4 by {overflow}"),
    ] {
        let source =
            format!("fn hidden() -> void {{ return; {expression}; }} fn main() -> void {{}}");
        let mut compiler = compiler();
        let program = compiler.analyze(&source).unwrap();
        let snapshot = program.clone();
        let expected = CodegenError::NumberLiteralOutOfRange {
            function: "hidden".into(),
            literal: overflow.clone(),
            span: program.functions[0].body[1].span,
        };
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap_err(), expected);
            assert_eq!(
                compiler.compile_source(&source).unwrap_err().span(),
                expected.span()
            );
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn forged_range_children_in_skipped_arms_and_after_abrupt_inputs_are_not_pruned() {
    for body in [
        "false && { 1..4 by 2; true };",
        "if true {} else { 1..4 by 2; }",
        "{ return; 0 }..4 by 2;",
    ] {
        let source = format!("fn hidden() -> void {{ {body} }} fn main() -> void {{}}");
        let mut compiler = compiler();
        let mut program = compiler.analyze(&source).unwrap();
        let statement = &mut program.functions[0].body[0];
        let value = match &mut statement.kind {
            TypedStatementKind::If {
                else_: Some(block), ..
            } => {
                let TypedStatementKind::Expression(value) = &mut block[0].kind else {
                    panic!("expected unselected range");
                };
                value
            }
            TypedStatementKind::Expression(value) => match &mut value.kind {
                TypedExprKind::Binary { right, .. } => {
                    let TypedExprKind::Block(block) = &mut right.kind else {
                        panic!("expected skipped block");
                    };
                    let TypedStatementKind::Expression(value) = &mut block.statements[0].kind
                    else {
                        panic!("expected skipped range");
                    };
                    value
                }
                TypedExprKind::Range { end, .. } => end,
                _ => panic!("expected range or logical expression"),
            },
            _ => panic!("expected expression or statement if"),
        };
        // For the abrupt-input case `value` is already the end operand.
        if let TypedExprKind::Range { step, .. } = &mut value.kind {
            step.as_deref_mut().unwrap().kind = TypedExprKind::Bool(true);
        } else {
            value.kind = TypedExprKind::Bool(true);
        }
        assert_forged_rejected(&mut compiler, &program);
    }
}

#[test]
fn range_children_plan_nested_locals_memory_remainder_and_abrupt_flow() {
    let source = r#"
        fn main() -> number {
            let r = { let a = [1]; a[0] }..={ let b = "x"; 5 }
                    by { let s = 7 % 3; s };
            let copy: range = if true { r } else { loop { break 0..1; } };
            return 8;
        }
        fn abrupt() -> number {
            let r: range = { return 9; 0 }..{ let a = [2]; a[0] } by 1;
            return 0;
        }
    "#;
    let mut compiler = Compiler::new();
    let baseline = compile(&compiler, source);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compile(&compiler, source), baseline);
    }
    let hidden = "fn main() -> bool { return false && { 0..4 by missing; true }; }";
    assert!(matches!(
        compiler.analyze(hidden),
        Err(CompileError::Semantic(_))
    ));
}

#[test]
fn forged_membership_annotations_and_children_are_checked_even_when_skipped() {
    for context in ["1 in 0..3", "{ return; 1 } in 0..3", "false && (1 in 0..3)"] {
        let source = format!("fn hidden() -> void {{ return; {context}; }} fn main() -> void {{}}");
        let mut compiler = compiler();
        let original = compiler.analyze(&source).unwrap();
        for corruption in 0..9 {
            let mut forged = original.clone();
            let TypedStatementKind::Expression(expr) = &mut forged.functions[0].body[1].kind else {
                panic!("expression");
            };
            let member = if context.starts_with("false") {
                let TypedExprKind::Binary { right, .. } = &mut expr.kind else {
                    panic!("logical");
                };
                right.as_mut()
            } else {
                expr
            };
            let TypedExprKind::Binary { left, right, .. } = &mut member.kind else {
                panic!("membership");
            };
            match corruption {
                0 => member.ty = Type::Number.into(),
                1 => member.ty = ExprResult::Void,
                2 => member.ty = ExprResult::Value(Type::Void),
                3 => {
                    **left = TypedExpr {
                        kind: TypedExprKind::Bool(true),
                        ty: Type::Bool.into(),
                    }
                }
                4 => {
                    **right = TypedExpr {
                        kind: TypedExprKind::Number("1".into()),
                        ty: Type::Number.into(),
                    }
                }
                5 => {
                    left.kind = TypedExprKind::Bool(true);
                    left.ty = Type::Number.into();
                }
                6 => {
                    right.kind = TypedExprKind::Bool(true);
                    right.ty = Type::Range.into();
                }
                7 => {
                    right.kind = TypedExprKind::Bool(true);
                    right.ty = ExprResult::Never;
                }
                _ => {
                    left.kind = TypedExprKind::Number("bad".into());
                    left.ty = Type::Number.into();
                }
            }
            assert_forged_rejected(&mut compiler, &forged);
        }
    }
}

fn decoded_operators(bytes: &[u8]) -> Vec<Vec<Operator<'_>>> {
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            bodies.push(
                body.get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
            );
        }
    }
    bodies
}

#[test]
fn membership_discovery_in_hidden_skipped_and_never_children_preserves_helper_order() {
    for expression in [
        "1 in 0..3",
        "false && (1 in 0..3)",
        "{ return; 1 } in 0..3",
        "(loop {}) in (loop {})",
    ] {
        let source = format!(
            "fn hidden() -> void {{ return; 5 % 2; {expression}; }} fn main() -> void {{}}"
        );
        let bytes = compile(&compiler(), &source);
        let decoded = decode(&bytes);
        assert_eq!(decoded.bodies.len(), 13); // two users, seven memory, remainder, three range
        let bodies = decoded_operators(&bytes);
        assert!(bodies[12].iter().any(|op| matches!(op, Operator::I64RemS)));
        assert!(
            bodies[12]
                .iter()
                .any(|op| matches!(op, Operator::Call { function_index: 11 }))
        );
        assert!(decoded.calls[0].contains(&9));
        assert!(decoded.calls[0].contains(&12));
    }
}

#[test]
fn divergent_membership_reconstructs_flow_and_validates_without_an_artificial_result() {
    for expression in [
        "(loop {}) in 0..3",
        "1 in (loop {})",
        "(loop {}) in (loop {})",
        "{ return 7; 1 } in 0..3",
        "1 in { return 7; 0..3 }",
    ] {
        let source = format!("fn main() -> number {{ let member: bool = {expression}; }}");
        let bytes = compile(&Compiler::new(), &source);
        let decoded = decode(&bytes);
        assert!(decoded.calls[0].contains(&10));
    }
}

#[test]
fn membership_lowers_to_exact_integer_congruence_without_floating_remainder() {
    let mut compiler = Compiler::new();
    let source =
        "fn main(n: number) -> bool { return n in -9007199254740991..=9007199254740991 by 3; }";
    let bytes = compile(&compiler, source);
    let decoded = decode(&bytes);
    assert_eq!(decoded.bodies.len(), 11); // user, seven memory, construct, validate, contains
    let bodies = decoded_operators(&bytes);
    let ops = bodies
        .iter()
        .find(|ops| ops.iter().any(|op| matches!(op, Operator::I64RemS)))
        .expect("signed integer congruence helper");
    assert!(ops.iter().any(|op| matches!(op, Operator::I64Sub)));
    assert!(ops.iter().any(|op| matches!(op, Operator::I64TruncF64S)));
    assert!(
        ops.iter()
            .any(|op| matches!(op, Operator::Call { function_index: 9 }))
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, Operator::F64Div | Operator::F64Mul))
    );
    assert_eq!(decoded.calls[0], vec![8, 10]);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compile(&compiler, source), bytes);
    }
}

fn run_node_at_all_levels(compiler: &mut Compiler, source: &str, script: &str) {
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compile(compiler, source);
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: Node failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_records_pin_bytes_safe_limits_signed_zero_default_steps_and_fresh_construction() {
    let source = r#"
        fn explicit(a: number, b: number, s: number) -> void { a..=b by s; }
        fn default(a: number, b: number) -> void { a..b; }
        fn exclusive(a: number, b: number, s: number) -> void { a..b by s; }
        fn scalar(n: number) -> number { return n; }
        fn main() -> void {}
    "#;
    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes)).exports;
        function check(name, args, expected) {
            // No static data: bracket the allocation using the public allocator,
            // rather than assuming a private heap global or exposing a range handle.
            const anchor = e.__alloc(8);
            e[name](...args);
            const next = e.__alloc(8);
            const h = anchor + 8;
            assert.equal(h % 8, 0);
            assert.equal(next, h + 32, 'exactly one 32-byte record, no materialized array');
            const v = new DataView(e.memory.buffer);
            assert.deepEqual([v.getBigInt64(h,true), v.getBigInt64(h+8,true),
                v.getBigInt64(h+16,true), v.getUint32(h+24,true), v.getUint32(h+28,true)], expected);
            const reference = Buffer.alloc(32);
            reference.writeBigInt64LE(expected[0],0);
            reference.writeBigInt64LE(expected[1],8);
            reference.writeBigInt64LE(expected[2],16);
            reference.writeUInt32LE(expected[3],24);
            assert.deepEqual(Buffer.from(e.memory.buffer,h,32), reference);
            return [h, Buffer.from(new Uint8Array(e.memory.buffer,h,32))];
        }
        assert.ok(Object.is(e.scalar(-0), -0), 'range normalization does not alter ordinary number semantics');
        const M = 9007199254740991;
        for (const [args, expected] of [
            [[0,3,1], [0n,3n,1n,1,0]],
            [[3,0,-1], [3n,0n,-1n,1,0]],
            [[5,5,2], [5n,5n,2n,1,0]],
            [[-0,0,-1], [0n,0n,-1n,1,0]],
            [[0,-0,1], [0n,0n,1n,1,0]],
            [[-M,M,M], [-9007199254740991n,9007199254740991n,9007199254740991n,1,0]],
            [[M,-M,-M], [9007199254740991n,-9007199254740991n,-9007199254740991n,1,0]],
        ]) check('explicit',args,expected);
        check('exclusive',[10,0,-2],[10n,0n,-2n,0,0]);
                check('exclusive',[0,10,-2],[0n,10n,-2n,0,0]);
                check('default',[10,0],[10n,0n,1n,0,0]);
        check('default',[-0,0],[0n,0n,1n,0,0]);
        const [first, saved] = check('default',[1,4],[1n,4n,1n,0,0]);
        const [second] = check('default',[1,4],[1n,4n,1n,0,0]);
        assert.notEqual(first,second);
        assert.deepEqual(Buffer.from(e.memory.buffer,first,32),saved,'later construction preserves earlier record');
    "#;
    let mut compiler = Compiler::new();
    run_node_at_all_levels(&mut compiler, source, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_invalid_discarded_empty_and_constant_ranges_trap_after_all_operand_effects() {
    let source = r#"
        fn explicit(a: number, b: number, s: number) -> void {
            mark(0); { mark(1); a }..{ mark(2); b } by { mark(3); s }; mark(4);
        }
        fn default(a: number, b: number) -> void {
            mark(0); { mark(1); a }..{ mark(2); b }; mark(4);
        }
        fn constant() -> void { mark(0); 5..5 by 0; mark(4); }
        fn constantFraction() -> void { mark(0); 0.5..0; mark(4); }
        fn abrupt() -> number { { mark(1); return 7; 0 }..{ mark(2); 4 } by { mark(3); 1 }; return 0; }
        fn lazy() -> bool { return false && { mark(90); 0..1 by 0; true }; }
    "#;
    let script = r#"
        const assert = require('node:assert/strict');
        const events = [];
        const hostFailure = new Error('later operand failed');
        let throwAt = null;
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            effects: { mark(n) { events.push(n); if (n === throwAt) throw hostFailure; } }
        }).exports;
        const invalid = [NaN, Infinity, -Infinity, 0.5, -0.5, Number.MIN_VALUE,
            -Number.MIN_VALUE, 9007199254740992, -9007199254740992, Number.MAX_VALUE];
        for (const bad of invalid) {
            for (let position = 0; position < 3; ++position) {
                const args = [5,5,1]; args[position] = bad;
                events.length = 0;
                assert.throws(() => e.explicit(...args), WebAssembly.RuntimeError);
                assert.deepEqual(events,[0,1,2,3], 'all inputs evaluate once before validating even empty ranges');
            }
            for (const args of [[bad,0],[0,bad]]) {
                events.length = 0;
                assert.throws(() => e.default(...args), WebAssembly.RuntimeError);
                assert.deepEqual(events,[0,1,2]);
            }
        }
        events.length = 0;
                throwAt = 2;
                assert.throws(() => e.explicit(NaN,5,1), error => error === hostFailure);
                assert.deepEqual(events,[0,1,2], 'later operand failure precedes validation of invalid start');
                throwAt = null;
                for (const zero of [0,-0]) {
            events.length = 0;
            assert.throws(() => e.explicit(5,5,zero), WebAssembly.RuntimeError);
            assert.deepEqual(events,[0,1,2,3]);
        }
        for (const name of ['constant','constantFraction']) {
            events.length = 0;
            assert.throws(() => e[name](),WebAssembly.RuntimeError);
            assert.deepEqual(events,[0]);
        }
        events.length = 0;
        e.explicit(-0,0,1);
        assert.deepEqual(events,[0,1,2,3,4]);
        events.length = 0;
        assert.equal(e.abrupt(),7);
        assert.deepEqual(events,[1], 'abrupt input prevents later input evaluation');
        events.length = 0;
        assert.equal(e.lazy(),0);
        assert.deepEqual(events,[], 'runtime-skipped construction does not trap');
    "#;
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "effects", "mark", vec![Type::Number], Type::Void)
        .unwrap();
    run_node_at_all_levels(&mut compiler, source, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_internal_calls_blocks_if_loops_and_copies_share_one_immutable_record() {
    let script = r#"
        const assert = require('node:assert/strict');
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0))).exports;
        for (const [flag, expected] of [[1,[1n,4n,1n,0,0]], [0,[9n,1n,-2n,1,0]]]) {
            const anchor = e.__alloc(8);
            assert.equal(e.main(flag),17);
            const next = e.__alloc(8);
            assert.equal(next,anchor+40,'blocks, joins, local copies and internal calls allocate no copies');
            const v = new DataView(e.memory.buffer), h = anchor+8;
            assert.deepEqual([v.getBigInt64(h,true),v.getBigInt64(h+8,true),v.getBigInt64(h+16,true),
                v.getUint32(h+24,true),v.getUint32(h+28,true)],expected);
        }
        assert.equal(e.identity,undefined);
        assert.equal(e.make,undefined);
        assert.equal(e.choose,undefined);
    "#;
    run_node_at_all_levels(&mut compiler(), INTERNAL_SOURCE, script);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_internal_parameter_boundaries_reject_corrupted_private_record_fields() {
    let source = r#"
        fn consume(r: range) -> number { return 11; }
        fn main() -> number { let r = 1..=4 by 2; mark(1); return consume(r); }
    "#;
    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        // Deliberately adversarial host writes probe guest boundary checks; these
        // writes are not a supported range-marshaling or host-mutation API.
        for (const [offset, value, wide] of [
            [0,9007199254740992n,true], [8,-9007199254740992n,true],
            [16,0n,true], [16,9007199254740992n,true], [24,2,false], [28,1,false],
        ]) {
            let e, h;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                effects: { mark() {
                    const v = new DataView(e.memory.buffer);
                    if (wide) v.setBigInt64(h+offset,value,true);
                    else v.setUint32(h+offset,value,true);
                } }
            });
            e = instance.exports;
            h = e.__alloc(8)+8;
            assert.throws(() => e.main(),WebAssembly.RuntimeError,`offset ${offset}`);
        }
    "#;
    let mut compiler = compiler();
    compiler
        .register_host_function("mark", "effects", "mark", vec![Type::Number], Type::Void)
        .unwrap();
    run_node_at_all_levels(&mut compiler, source, script);
}
