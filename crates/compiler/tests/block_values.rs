#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    semantic::SemanticError,
    tokens::Span,
    typed_ast::{ExprResult, TypedExpr, TypedExprKind, TypedStatementKind},
};
use wasmparser::{BlockType, ExternalKind, Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const SOURCE: &str = r#"
    fn main(flag: bool) -> number {
        let result = if { let guard = flag; guard } {
            let input = { let n = 4; n };
            forward({ let argument = input; argument })
        } else { mark(2); 8 % 3 };
        return { let tail = result; tail };
    }
    fn forward(n: number) -> number { return { mark(n); n + 1 }; }
    fn boolean(flag: bool) -> bool { return if flag { true } else { false }; }
    fn text(flag: bool) -> string { return if flag { "first" } else { "second" }; }
    fn items(flag: bool) -> number[] { return if flag { [1, 2] } else { [] }; }
    fn hidden() -> void {
        { let local = [3]; len(local) };
        (if true {} else {});
        return;
        { let unseen = [4]; "unreachable" };
    }
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "effects", "mark", vec![Type::Number], Type::Void)
        .unwrap();
    compiler
        .register_host_function("unused", "other", "unused", vec![], Type::Bool)
        .unwrap();
    compiler
}

fn validate(bytes: &[u8]) {
    Validator::new().validate_all(bytes).unwrap();
}

fn non_export_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| {
            let (id, range) = payload.unwrap().as_section()?;
            (id != 7).then(|| {
                let start = usize::try_from(range.start).unwrap();
                let end = usize::try_from(range.end).unwrap();
                (id, bytes[start..end].to_vec())
            })
        })
        .collect()
}

#[test]
fn public_pipeline_is_owned_deterministic_and_independent_of_optimization() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let original = program.clone();
    let expected = compiler.compile(&program).unwrap();
    validate(&expected);

    for level in LEVELS {
        compiler.set_optimization_level(level);
        for _ in 0..2 {
            assert_eq!(compiler.compile(&program).unwrap(), expected, "{level:?}");
            assert_eq!(compiler.compile_source(SOURCE).unwrap(), expected);
            assert_eq!(compiler.analyze(SOURCE).unwrap(), original);
            assert_eq!(program, original);
        }
    }

    compiler
        .register_host_function("later", "later", "added", vec![], Type::Void)
        .unwrap();
    assert_eq!(compiler.compile(&program).unwrap(), expected);
    assert_ne!(compiler.compile_source(SOURCE).unwrap(), expected);

    for (selection, names) in [
        (
            ExportOptions::All,
            vec!["main", "forward", "boolean", "text", "items", "hidden"],
        ),
        (ExportOptions::None, vec![]),
        (
            ExportOptions::Only(vec!["items".into(), "main".into()]),
            vec!["main", "items"],
        ),
    ] {
        compiler.set_exports(selection);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            validate(&bytes);
            assert_eq!(non_export_sections(&bytes), non_export_sections(&expected));
            let mut exports = Vec::new();
            let mut imports = Vec::new();
            for payload in Parser::new(0).parse_all(&bytes) {
                match payload.unwrap() {
                    Payload::ExportSection(section) => {
                        for export in section {
                            let export = export.unwrap();
                            exports.push((export.name.to_owned(), export.kind, export.index));
                        }
                    }
                    Payload::ImportSection(section) => {
                        for import in section.into_imports() {
                            let import = import.unwrap();
                            imports.push((import.module.to_owned(), import.name.to_owned()));
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(
                imports,
                [
                    ("effects".into(), "mark".into()),
                    ("other".into(), "unused".into())
                ]
            );
            let user_exports = exports
                .iter()
                .filter(|(name, _, _)| {
                    program
                        .functions
                        .iter()
                        .any(|function| function.name == *name)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                user_exports
                    .iter()
                    .map(|(name, _, _)| name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            for (name, kind, index) in user_exports {
                assert_eq!(*kind, ExternalKind::Func);
                let source_index = program
                    .functions
                    .iter()
                    .position(|function| function.name == *name)
                    .unwrap();
                assert_eq!(*index, u32::try_from(source_index + 2).unwrap());
            }
            assert!(
                exports
                    .iter()
                    .any(|(name, kind, _)| name == "memory" && *kind == ExternalKind::Memory)
            );
            assert!(exports.iter().any(|(name, kind, index)| name == "__alloc"
                && *kind == ExternalKind::Func
                && *index == 8));
            assert_eq!(program, original);
        }
    }
}

#[test]
fn analyzed_value_and_never_joins_keep_concrete_storage_and_region_spans() {
    for (name, ty, first, second) in [
        ("number", Type::Number, "1", "2"),
        ("bool", Type::Bool, "true", "false"),
        ("string", Type::String, "\"a\"", "\"b\""),
        ("number[]", Type::NumberArray, "[]", "[2]"),
    ] {
        for initializer in [
            format!("if flag {{ {first} }} else {{ {second} }}"),
            format!("if flag {{ return {first}; }} else {{ {second} }}"),
            format!("if flag {{ {first} }} else {{ return {second}; }}"),
            format!("{{ let inside = {first}; inside }}"),
        ] {
            let source = format!(
                "fn main(flag: bool) -> {name} {{ let value = {initializer}; return value; }}"
            );
            let mut compiler = Compiler::new();
            let program = compiler.analyze(&source).unwrap();
            let TypedStatementKind::Let {
                ty: storage, value, ..
            } = &program.functions[0].body[0].kind
            else {
                panic!("expected declaration");
            };
            assert_eq!(*storage, ty);
            assert_eq!(value.ty, ExprResult::Value(ty));
            let region = match &value.kind {
                TypedExprKind::Block(block) => block,
                TypedExprKind::If { then, .. } => then,
                _ => panic!("expected value region"),
            };
            assert!(source[region.span.start()..region.span.end()].starts_with('{'));
            assert!(source[region.span.start()..region.span.end()].ends_with('}'));
            let baseline = compiler.compile(&program).unwrap();
            validate(&baseline);
            for level in LEVELS {
                compiler.set_optimization_level(level);
                assert_eq!(compiler.compile_source(&source).unwrap(), baseline);
            }
        }

        let source =
            format!("fn main() -> number {{ let value: {name} = {{ return 7; }}; value; }}");
        let compiler = Compiler::new();
        let program = compiler.analyze(&source).unwrap();
        let TypedStatementKind::Let {
            ty: storage, value, ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected annotated divergent initializer");
        };
        assert_eq!(*storage, ty);
        assert_eq!(value.ty, ExprResult::Never);
        validate(&compiler.compile(&program).unwrap());
    }

    for source in [
        "fn main() -> number { (if true { return 1; } else { return 2; }); }",
        "fn main() -> number { while { return 3; } {} }",
        "fn main() -> void { (if true {} else {}); { 1; }; {} ; }",
        "fn main() -> number { return { if false { 1 } let n = 2; if true { n } else if false { 3 } else { 4 } }; }",
        "fn main() -> void { if true { 1 } else if false { \"discarded\" } }",
    ] {
        validate(&Compiler::new().compile_source(source).unwrap());
    }
}

#[test]
fn source_regions_decode_mvp_results_discards_and_expression_local_slots() {
    let source = "fn main() -> void {
        { let n = 1; n };
        (if { let condition = true; condition } { let yes = false; yes } else { true });
        { 2; };
        {};
    }";
    let bytes = Compiler::new().compile_source(source).unwrap();
    validate(&bytes);
    let mut body_count = 0;
    for payload in Parser::new(0).parse_all(&bytes) {
        let payload = payload.unwrap();
        if let Some((id, _)) = payload.as_section() {
            assert!(
                ![5, 6, 11].contains(&id),
                "scalar regions must not add memory"
            );
        }
        if let Payload::CodeSectionEntry(body) = payload {
            body_count += 1;
            assert_eq!(
                body.get_locals_reader()
                    .unwrap()
                    .into_iter()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>(),
                [(1, ValType::F64), (1, ValType::I32), (1, ValType::I32)]
            );
            let mut blocks = Vec::new();
            let mut ifs = Vec::new();
            let mut stores = Vec::new();
            let mut drops = 0;
            for op in body.get_operators_reader().unwrap() {
                match op.unwrap() {
                    Operator::Block { blockty } => blocks.push(blockty),
                    Operator::If { blockty } => ifs.push(blockty),
                    Operator::LocalSet { local_index } => stores.push(local_index),
                    Operator::Drop => drops += 1,
                    _ => {}
                }
            }
            assert_eq!(
                blocks,
                [
                    BlockType::Type(ValType::F64),
                    BlockType::Type(ValType::I32),
                    BlockType::Empty,
                    BlockType::Empty
                ]
            );
            assert_eq!(ifs, [BlockType::Type(ValType::I32)]);
            assert_eq!(stores, [0, 1, 2]);
            assert_eq!(drops, 3);
        }
    }
    assert_eq!(body_count, 1);
}

fn semantic_rejection(source: &str, expected: fn(&SemanticError) -> bool) {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    let error = compiler.analyze(source).unwrap_err();
    assert!(
        matches!(&error, CompileError::Semantic(error) if expected(error)),
        "{source}: {error:?}"
    );
    let span = error.span().expect("located semantic rejection");
    assert!(span.start() < span.end() && span.end() <= source.len());
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.analyze(source).unwrap_err(), error);
        assert_eq!(compiler.compile_source(source).unwrap_err(), error);
    }
}

#[test]
fn syntax_rejects_adjacency_truncation_and_implicit_function_or_while_tails() {
    for source in [
        "fn main() -> number { 1 }",
        "fn main() -> void { { 1 } }",
        "fn main() -> void { while false { 1 } }",
        "fn main() -> number { return { 1 2 }; }",
        "fn main() -> number { return { { 1 } { 2 } }; }",
        "fn main() -> number { return { (if true { 1 } else { 2 }) 3 }; }",
        "fn main() -> number { return { 1; 2; 3 4 }; }",
        "fn main() -> number { return if true { 1 } else; }",
        "fn main() -> number { return if true { 1 } else {",
        "fn main() -> number { return { let x = 1; x",
        "fn main() -> number { let n = { let x = 1 x }; return n; }",
    ] {
        let mut compiler = Compiler::new();
        let error = compiler.analyze(source).unwrap_err();
        assert!(
            matches!(error, CompileError::Parse(_)),
            "{source}: {error:?}"
        );
        assert!(error.span().is_some());
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile_source(source).unwrap_err(), error);
        }
    }
    semantic_rejection(
        "fn main() -> number { if true { 1 } else { 2 } }",
        |error| matches!(error, SemanticError::MissingReturn { .. }),
    );
}

#[test]
fn value_contexts_reject_missing_else_void_mismatched_arms_and_bad_conditions() {
    for expression in [
        "if true { 1 }",
        "{ if true { 1 } }",
        "if true { 1 } else if false { 2 }",
        "if { return 1; } { return 2; }",
    ] {
        let statement = format!("let value: number = {expression};");
        let source = format!("fn main() -> number {{ {statement} return value; }}");
        semantic_rejection(&source, |error| {
            matches!(error, SemanticError::MissingElse { .. })
        });
    }
    for expression in ["{}", "{ 1; }", "if true {} else {}"] {
        let source = format!("fn main() -> void {{ let value = {expression}; }}");
        semantic_rejection(
            &source,
            |error| matches!(error, SemanticError::InvalidVoidType { name, .. } if name == "value"),
        );
    }
    for expression in [
        "if true { 1 } else { false }",
        "if true { [1] } else { \"text\" }",
        "if 1 { 2 } else { 3 }",
        "if true { 1 } else {}",
        "if true {} else { 1 }",
    ] {
        let source = format!("fn main() -> void {{ let value = {expression}; }}");
        semantic_rejection(&source, |error| {
            matches!(error, SemanticError::TypeMismatch { .. })
        });
    }
    for initializer in ["{ return 1; }", "if true { return 1; } else { return 2; }"] {
        let statement = format!("let value = {initializer};");
        let source = format!("fn main() -> number {{ {statement} }}");
        let start = source.find(&statement).unwrap();
        let span = Span::new(start, start + statement.len());
        assert_eq!(
            Compiler::new().analyze(&source),
            Err(CompileError::Semantic(
                SemanticError::CannotInferNeverType {
                    name: "value".into(),
                    span
                }
            ))
        );
        semantic_rejection(&source, |error| {
            matches!(error, SemanticError::CannotInferNeverType { .. })
        });
    }
}

#[test]
fn hidden_and_abrupt_source_children_still_receive_scope_type_and_return_checks() {
    for body in [
        "let n = { let inside = 1; inside }; inside;",
        "let n = if true { let inside = 1; inside } else { inside };",
        "let n = { let same = 1; let same = 2; same };",
        "let n = { let n = n; n };",
        "let n: number = { return; missing };",
        "false && { missing; true };",
        "let n = if true { 1 } else { missing };",
        "return; { missing };",
        "{ return; } + false;",
        "take({ return; }, false);",
        "[{ return; }, false];",
    ] {
        let source = format!(
            "fn hidden() -> void {{ {body} }} fn take(a: number, b: number) -> void {{}} fn main() -> void {{}}"
        );
        semantic_rejection(&source, |error| {
            matches!(
                error,
                SemanticError::UndefinedVariable { .. }
                    | SemanticError::DuplicateVariable { .. }
                    | SemanticError::TypeMismatch { .. }
                    | SemanticError::InvalidBinaryOperands { .. }
            )
        });
    }
    for body in [
        "while { break; } {}",
        "while true { let n: number = { break; }; return 1; }",
        "false && { return 1; };",
    ] {
        let source = format!("fn main() -> number {{ {body} }}");
        semantic_rejection(&source, |error| {
            matches!(error, SemanticError::MissingReturn { .. })
        });
    }
    semantic_rejection("fn main() -> void { { break; }; }", |error| {
        matches!(error, SemanticError::BreakOutsideLoop { .. })
    });
    semantic_rejection("fn main() -> void { { continue; }; }", |error| {
        matches!(error, SemanticError::ContinueOutsideLoop { .. })
    });
}

#[test]
fn analysis_succeeds_but_compilation_checks_overflowing_literals_in_hidden_regions() {
    let literal = "9".repeat(400);
    for body in [
        format!("let n = if true {{ 1 }} else {{ {literal} }};"),
        format!("false && {{ {literal}; true }};"),
        format!("{{ return; {literal} }};"),
    ] {
        let source = format!("fn hidden() -> void {{ {body} }} fn main() -> void {{}}");
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        let program = compiler.analyze(&source).unwrap();
        let snapshot = program.clone();
        let error = compiler.compile(&program).unwrap_err();
        assert!(
            matches!(&error, CodegenError::NumberLiteralOutOfRange { function, literal: actual, .. } if function == "hidden" && actual == &literal)
        );
        assert!(error.span().is_some());
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap_err(), error);
            assert_eq!(
                compiler.compile_source(&source).unwrap_err(),
                CompileError::Codegen(compiler.compile(&program).unwrap_err())
            );
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn public_forged_region_results_and_missing_else_are_rejected_without_mutation() {
    for source in [
        "fn hidden() -> void { { 1 }; } fn main() -> void {}",
        "fn hidden() -> void { return; { 1 }; } fn main() -> void {}",
    ] {
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        let original = compiler.analyze(source).unwrap();
        for found in [
            ExprResult::Never,
            ExprResult::Void,
            ExprResult::Value(Type::Void),
        ] {
            let mut program = original.clone();
            let statement = program.functions[0].body.last_mut().unwrap();
            let span = statement.span;
            let TypedStatementKind::Expression(value) = &mut statement.kind else {
                panic!("expected expression");
            };
            value.ty = found;
            let snapshot = program.clone();
            let expected = if found == ExprResult::Void {
                CodegenError::TypeMismatch {
                    function: "hidden".into(),
                    expected: Type::Number,
                    found: Type::Void,
                    span,
                }
            } else {
                CodegenError::ExprResultMismatch {
                    function: "hidden".into(),
                    expected: ExprResult::Value(Type::Number),
                    found,
                    span,
                }
            };
            for level in LEVELS {
                compiler.set_optimization_level(level);
                assert_eq!(compiler.compile(&program).unwrap_err(), expected);
                assert_eq!(program, snapshot);
            }
        }
    }
    let mut compiler = Compiler::new();
    let mut program = compiler
        .analyze("fn hidden() -> number { return if true { 1 } else { 2 }; } fn main() -> void {}")
        .unwrap();
    compiler.set_exports(ExportOptions::None);
    let statement = &mut program.functions[0].body[0];
    let span = statement.span;
    let TypedStatementKind::Return(Some(TypedExpr {
        kind: TypedExprKind::If { else_, .. },
        ..
    })) = &mut statement.kind
    else {
        panic!("expected returned if");
    };
    *else_ = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::MissingElse {
                function: "hidden".into(),
                span
            })
        );
    }
}

#[test]
fn forged_literals_in_skipped_arms_short_circuits_and_unreachable_tails_are_checked() {
    for body in [
        "let n = if true { 1 } else { { 2 } };",
        "false && { { 2 }; true };",
        "{ return; { 2 } };",
    ] {
        let mut compiler = Compiler::new();
        let source = format!("fn hidden() -> void {{ {body} }} fn main() -> void {{}}");
        let mut program = compiler.analyze(&source).unwrap();
        let value = match &mut program.functions[0].body[0].kind {
            TypedStatementKind::Let { value, .. } | TypedStatementKind::Expression(value) => value,
            _ => panic!("expected value"),
        };
        let block = match &mut value.kind {
            TypedExprKind::If {
                else_: Some(block), ..
            } => block,
            TypedExprKind::Binary { right, .. } => {
                let TypedExprKind::Block(block) = &mut right.kind else {
                    panic!("expected RHS block");
                };
                block
            }
            TypedExprKind::Block(block) => block,
            _ => panic!("expected region"),
        };
        // The short-circuit case keeps the forged literal in a discarded child.
        let inner = match block
            .statements
            .last_mut()
            .map(|statement| &mut statement.kind)
        {
            Some(TypedStatementKind::Expression(expr)) => expr,
            _ => block.tail.as_mut().expect("expected tail").as_mut(),
        };
        let TypedExprKind::Block(inner) = &mut inner.kind else {
            panic!("expected inner block");
        };
        let tail = inner.tail.as_mut().unwrap();
        tail.kind = TypedExprKind::Number("NaN".into());
        let span = inner.span;
        let snapshot = program.clone();
        for exports in [
            ExportOptions::None,
            ExportOptions::Only(vec!["main".into()]),
        ] {
            compiler.set_exports(exports);
            for level in LEVELS {
                compiler.set_optimization_level(level);
                assert_eq!(
                    compiler.compile(&program),
                    Err(CodegenError::InvalidNumberLiteral {
                        function: "hidden".into(),
                        literal: "NaN".into(),
                        span
                    })
                );
                assert_eq!(program, snapshot);
            }
        }
    }
}

#[test]
fn legacy_statement_if_followed_by_unary_minus_keeps_independent_mvp_bytes() {
    // Fixed MVP encoding: empty statement-if, f64.const 1 / neg / drop,
    // then f64.const 2 / return. No encoder or current compiler builds the oracle.
    const LEGACY: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 124, 2, 1, 0, 3, 2, 1, 0, 7, 8, 1, 4, 109,
        97, 105, 110, 0, 0, 10, 30, 1, 28, 0, 65, 1, 4, 64, 11, 68, 0, 0, 0, 0, 0, 0, 240, 63, 154,
        26, 68, 0, 0, 0, 0, 0, 0, 0, 64, 15, 11,
    ];
    validate(LEGACY);
    let mut compiler = Compiler::new();
    for source in [
        "fn main() -> number { if true {} -1; return 2; }",
        "fn main() -> number { if true {}\n-1; return 2; }",
        "fn main() -> number { if true {} /* boundary */ -1; return 2; }",
    ] {
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(
                compiler.compile_source(source).unwrap(),
                LEGACY,
                "{source}, {level:?}"
            );
        }
    }
}

fn run_node(bytes: &[u8], script: &str) {
    validate(bytes);
    let output = test_process::output(
        Command::new("node").arg("-e").arg(script),
        Some(bytes),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_selected_arms_short_circuit_values_scopes_and_divergence_have_observable_effects() {
    let source = r#"
        fn choose(flag: bool) -> number {
            let outer = 10;
            let result = if { mark(1); flag } {
                mark(2); let outer = outer + 1; { let inner = outer + 2; inner }
            } else { mark(3); return 40; };
            mark(4);
            return outer + result;
        }
        fn lazy() -> bool {
            let a = false && { mark(90); return false; };
            let b = true || { mark(91); return false; };
            let c = true && { mark(5); true };
            let d = false || { mark(6); false };
            return !a && b && c && !d;
        }
        fn numeric(flag: bool) -> number { return if flag { -0 } else { 3.5 }; }
        fn boolean(flag: bool) -> bool { return { let x = flag; !x }; }
        fn text(flag: bool) -> string { return if flag { "é" } else if false { "dead" } else { "世界" }; }
        fn items(flag: bool) -> number[] { return if flag { { let a = [1, 2]; a } } else { [] }; }
        fn explicit() -> number { let never: string = { mark(7); return 17; }; mark(92); return 0; }
        fn voids() -> void { (if true { mark(8); } else { mark(93); }); { mark(9); 2; }; }
    "#;
    let script = r#"
        const assert = require('node:assert/strict');
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            effects: { mark(n) { events.push(n); } }, other: { unused() { return 0; } }
        }).exports;
        assert.equal(e.choose(1), 23);
        assert.deepEqual(events, [1, 2, 4]);
        events.length = 0;
        assert.equal(e.choose(0), 40);
        assert.deepEqual(events, [1, 3]);
        events.length = 0;
        assert.equal(e.lazy(), 1);
        assert.deepEqual(events, [5, 6]);
        assert.ok(Object.is(e.numeric(1), -0));
        assert.equal(e.numeric(0), 3.5);
        assert.equal(e.boolean(1), 0);
        assert.equal(e.boolean(0), 1);
        function descriptor(h) {
            const v = new DataView(e.memory.buffer);
            return [v.getUint32(h, true), v.getUint32(h + 4, true)];
        }
        for (const [flag, expected] of [[1, 'é'], [0, '世界']]) {
            const [p, n] = descriptor(e.text(flag));
            assert.equal(new TextDecoder().decode(new Uint8Array(e.memory.buffer, p, n)), expected);
        }
        const first = e.items(1), second = e.items(1);
        assert.notEqual(first, second);
        const [p, n] = descriptor(first);
        assert.equal(n, 2);
        assert.deepEqual([0,1].map(i => new DataView(e.memory.buffer).getFloat64(p + i * 8, true)), [1,2]);
        assert.equal(descriptor(e.items(0))[1], 0);
        events.length = 0;
        assert.equal(e.explicit(), 17);
        e.voids();
        assert.deepEqual(events, [7,8,9]);
    "#;
    let mut compiler = compiler();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(&compiler.compile_source(source).unwrap(), script);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_expression_transfers_and_compounds_preserve_original_places_and_trap_order() {
    let source = r#"
        fn conditions() -> number {
            let i = 0;
            let total = 0;
            while { i += 1; mark(i); if i == 2 { continue; } else { if i == 4 { break; } else { true } } } {
                total += i;
            }
            return total * 10 + i;
        }
        fn transfers() -> number {
            let i = 0;
            let total = 0;
            while i < 4 {
                i += 1;
                total += if i == 2 { continue; } else { if i == 4 { break; } else { i } };
                mark(10 + i);
            }
            return total;
        }
        fn condition_return() -> number { while { mark(20); return 6; } { mark(99); } }
        fn scalar() -> number { let n = 4; n += { n = 100; mark(n); 3 }; return n; }
        fn places() -> number {
            let original = [10, 20];
            let target = original;
            let index = 0;
            target[{ mark(30); index }] += {
                mark(31); index = 1; target = [99, 88]; original[0] = 50; 2
            };
            return original[0] + original[1] * 10 + target[0] * 100 + index * 10000;
        }
        fn checked(a: number[]) -> number { a[{ mark(40); 999 }] += { mark(41); 1 }; return 0; }
        fn array() -> number[] { return [5]; }
    "#;
    let script = r#"
        const assert = require('node:assert/strict');
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            effects: { mark(n) { events.push(n); } }, other: { unused() { return 0; } }
        }).exports;
        assert.equal(e.conditions(), 44);
        assert.deepEqual(events, [1,2,3,4]);
        events.length = 0;
        assert.equal(e.transfers(), 4);
        assert.deepEqual(events, [11,13]);
        events.length = 0;
        assert.equal(e.condition_return(), 6);
        assert.deepEqual(events, [20]);
        events.length = 0;
        assert.equal(e.scalar(), 7);
        assert.deepEqual(events, [100]);
        events.length = 0;
        assert.equal(e.places(), 20112);
        assert.deepEqual(events, [30,31]);
        events.length = 0;
        assert.throws(() => e.checked(e.array()), WebAssembly.RuntimeError);
        assert.deepEqual(events, [40]);
    "#;
    let mut compiler = compiler();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(&compiler.compile_source(source).unwrap(), script);
    }
}
