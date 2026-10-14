use was_compiler::{
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::{BinaryOp, Expr, StatementKind},
    lexer::Lexer,
    parser::Parser,
    semantic::SemanticError,
    tokens::{Span, TokenKind},
    typed_ast::{CallBindingOrigin, ExprResult, TypedExprKind, TypedStatementKind},
};

fn error(source: &str) -> SemanticError {
    match Compiler::new().analyze(source).unwrap_err() {
        CompileError::Semantic(error) => error,
        other => panic!("expected semantic error, got {other:?}"),
    }
}

fn span(source: &str, text: &str) -> Span {
    let start = source.find(text).unwrap();
    Span::new(start, start + text.len())
}

#[test]
fn positional_stages_normalize_with_distinct_injection_and_written_indices() {
    let source = "fn pair(a: number, b: number) { return a + b; } fn identity(x: number) { return x; } fn main() { return 1 |> pair(2) |> identity(); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[2].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(outer) = &value.kind else {
        panic!("normalized")
    };
    assert_eq!(outer.bindings[0].origin, CallBindingOrigin::PipelineInput);
    let TypedExprKind::NormalizedCall(inner) = &outer.bindings[0].value.kind else {
        panic!("nested normalized")
    };
    assert_eq!(inner.bindings[0].origin, CallBindingOrigin::PipelineInput);
    assert_eq!(
        inner.bindings[1].origin,
        CallBindingOrigin::Explicit { argument: 0 }
    );
    assert_eq!(
        inner.formal_to_binding,
        inner.bindings.iter().map(|b| b.id).collect::<Vec<_>>()
    );
    wasmparser::Validator::new()
        .validate_all(&Compiler::new().compile(&program).unwrap())
        .unwrap();
}

#[test]
fn raw_stages_preserve_parenthesized_input_argument_and_comment_byte_spans() {
    let source = "/*🦀*/ fn f() { return (\"é\") /*λ*/ |> first /*β*/ |> second(1, x: 2); }";
    let tokens = Lexer::new(source).tokenize();
    let raw = Parser::new(&tokens).parse_program().unwrap();
    let StatementKind::Return(Some(Expr::Pipeline {
        input,
        input_span,
        stages,
    })) = &raw.functions[0].body[0].kind
    else {
        panic!()
    };
    assert_eq!(**input, Expr::String("é".into()));
    assert_eq!(*input_span, span(source, "(\"é\")"));
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0].span, span(source, "first"));
    assert_eq!(stages[1].span, span(source, "second(1, x: 2)"));
    assert_eq!(stages[1].args[0].span, span(source, "1"));
    assert_eq!(stages[1].args[1].span, span(source, "x: 2"));
}

#[test]
fn pipeline_precedence_is_below_or_and_stages_are_left_associative() {
    let source = "fn f() { return true || false && true |> first |> second(); }";
    let raw = Parser::new(&Lexer::new(source).tokenize())
        .parse_program()
        .unwrap();
    let StatementKind::Return(Some(Expr::Pipeline { input, stages, .. })) =
        &raw.functions[0].body[0].kind
    else {
        panic!()
    };
    assert!(matches!(
        **input,
        Expr::Binary {
            op: BinaryOp::Or,
            ..
        }
    ));
    assert_eq!(
        stages.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert!(stages.iter().all(|s| s.args.is_empty()));
    let source = "fn identity(x: bool) { return x; } fn f() { return true || false |> identity |> identity(); }";
    Compiler::new().compile_source(source).unwrap();
}

#[test]
fn malformed_stage_syntax_and_split_operator_bytes_are_rejected() {
    for expression in [
        "1 | > f",
        "1 |/*é*/> f",
        "1 |> ",
        "1 |> (f)",
        "1 |> _",
        "1 |> 2",
        "1 |> f[0]",
        "1 |> f()()",
        "1 |> f + 2",
        "1 |> f(,)",
        "1 |> f(1,)",
        "1 |> f(",
    ] {
        let source = format!("fn main() {{ return {expression}; }}");
        assert!(
            matches!(
                Compiler::new().analyze(&source),
                Err(CompileError::Parse(_))
            ),
            "{expression}"
        );
    }
    let tokens = Lexer::new("|> || | > |/*x*/>").tokenize();
    assert_eq!(tokens[0].kind, TokenKind::Pipeline);
    assert_eq!(tokens[0].span, Span::new(0, 2));
    assert_eq!(tokens[1].kind, TokenKind::OrOr);
    assert_eq!(tokens[2].kind, TokenKind::Invalid);
    assert_eq!(tokens[3].kind, TokenKind::Greater);
    assert_eq!(tokens[4].kind, TokenKind::Invalid);
    assert_eq!(tokens[5].kind, TokenKind::Greater);
}

#[test]
fn bare_control_classification_stays_unchanged_and_parentheses_allow_pipelines() {
    for expression in [
        "if true { 1 } else { 2 }",
        "match true { _ => 1 }",
        "loop { break 1; }",
    ] {
        let declarations = "fn identity(x: number) -> number { return x; }";
        let bare = format!("{declarations} fn f() {{ let x = {{ {expression} |> identity }}; }}");
        assert!(
            matches!(Compiler::new().analyze(&bare), Err(CompileError::Parse(_))),
            "{expression}"
        );
        let parenthesized = format!(
            "{declarations} fn f() -> number {{ return {{ ({expression}) |> identity }}; }}"
        );
        Compiler::new().compile_source(&parenthesized).unwrap();
    }
}

#[test]
fn stage_eligibility_shadowing_and_subset_restrictions_are_explicit() {
    for (source, message) in [
        (
            "fn f() { return [1] |> len; }",
            "stages must name user-defined functions",
        ),
        (
            "fn zero() -> number { return 1; } fn f() { return 1 |> zero; }",
            "a stage requires an input parameter",
        ),
        (
            "fn discard(x: number) {} fn f() { 1 |> discard; }",
            "void stages are not supported in this pipeline subset",
        ),
        (
            "fn discard(x: number) {} fn f() -> number { return (loop {}) |> discard |> missing; }",
            "void stages are not supported in this pipeline subset",
        ),
    ] {
        let err = error(source);
        assert!(
            matches!(err, SemanticError::InvalidPipeline { message: found, .. } if found == message),
            "{err:?}"
        );
        assert!(err.to_string().starts_with("Invalid pipeline:"));
    }
    assert!(matches!(
        error("fn f() { return 1 |> missing; }"),
        SemanticError::UndefinedFunction { .. }
    ));
    assert!(matches!(
        error("fn id(x: number) { return x; } fn f() { let id = 1; return 1 |> id; }"),
        SemanticError::NotCallable { .. }
    ));
    assert!(
        matches!(error("fn pair(a: number, b: number = 2) { return a + b; } fn f() { return 1 |> pair; }"), SemanticError::MissingArgument { argument, .. } if argument == "b")
    );
    assert!(matches!(
        error("fn pair(a: number, b: number) { return a + b; } fn f() { return 1 |> pair(b: 2); }"),
        SemanticError::NamedArgumentsNotSupported { .. }
    ));
    assert!(matches!(
        error("fn id(a: number) { return a; } fn f() { return 1 |> id(2); }"),
        SemanticError::TooManyPositionalArguments { .. }
    ));
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("host", "env", "host", vec![Type::Number], Type::Number)
        .unwrap();
    assert!(matches!(
        compiler.analyze("fn f() { return 1 |> host; }"),
        Err(CompileError::Semantic(
            SemanticError::InvalidPipeline { .. }
        ))
    ));
}

#[test]
fn injected_type_errors_use_prefix_and_written_errors_use_argument_spans() {
    for (source, origin) in [
        (
            "fn id(a: number) { return a; } fn f() { return (\"é\") |> id; }",
            "(\"é\")",
        ),
        (
            "fn text(a: number) { return \"s\"; } fn id(a: number) { return a; } fn f() { return 1 |> text |> id; }",
            "1 |> text",
        ),
        (
            "fn pair(a: number, b: number) { return a + b; } fn f() { return 1 |> pair(\"é\"); }",
            "\"é\"",
        ),
    ] {
        let err = error(source);
        assert!(matches!(err, SemanticError::TypeMismatch { .. }));
        assert_eq!(err.span(), Some(span(source, origin)));
    }
}

#[test]
fn input_then_structural_then_written_then_result_error_order_is_preserved() {
    assert!(
        matches!(error("fn pair(a: number, b: number) -> number { return a; } fn f() { return unknown |> missing; }"), SemanticError::UndefinedVariable { name, .. } if name == "unknown")
    );
    assert!(matches!(
        error(
            "fn pair(a: number, b: number) -> number { return a; } fn f() { return 1 |> pair(unknown, 2); }"
        ),
        SemanticError::TooManyPositionalArguments { .. }
    ));
    assert!(
        matches!(error("fn discard(a: number, b: number) {} fn f() { 1 |> discard(unknown); }"), SemanticError::UndefinedVariable { name, .. } if name == "unknown")
    );
    assert!(matches!(
        error(
            "fn id(x: number) -> number { return x; } fn f() -> number { return (loop {}) |> id(unknown); }"
        ),
        SemanticError::TooManyPositionalArguments { .. }
    ));
}

#[test]
fn defaults_and_contracts_reject_raw_pipeline_even_in_skipped_operands() {
    for declaration in [
        "fn f(x: bool = false && (true |> id)) {}",
        "fn f(x: bool) requires false && (true |> id) {}",
    ] {
        let source = format!("fn id(x: bool) -> bool {{ return x; }} {declaration}");
        let err = error(&source);
        assert!(
            matches!(
                err,
                SemanticError::InvalidDefaultExpression {
                    reason: "pipelines are not allowed",
                    ..
                } | SemanticError::InvalidContractExpression {
                    reason: "pipelines are not allowed",
                    ..
                }
            ),
            "{err:?}"
        );
    }
}

#[test]
fn inferred_forward_nested_chains_and_diamonds_keep_source_order() {
    let source = "fn root() { return 2 |> left |> right; } fn left(x: number) { return x |> leaf; } fn right(x: number) { return x |> leaf; } fn leaf(x: number) { return x + 1; } fn nested() { return leaf(1 |> left) |> right; }";
    let mut compiler = Compiler::new();
    let typed = compiler.analyze(source).unwrap();
    assert!(
        typed
            .functions
            .iter()
            .all(|f| f.return_type == Type::Number)
    );
    assert_eq!(
        typed
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["root", "left", "right", "leaf", "nested"]
    );
    let baseline = compiler.compile(&typed).unwrap();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile_source(source).unwrap(), baseline);
    }
}

#[test]
fn pipeline_only_recursion_includes_unreachable_and_skipped_syntax() {
    for source in [
        "fn f(x: number) { return x |> f; }",
        "fn f(x: number) { return 1; x |> f; }",
        "fn f(x: bool) { return false && (x |> f); }",
        "fn f(x: number) { return x |> g; } fn g(x: number) -> number { return x |> f; }",
    ] {
        assert!(
            matches!(
                error(source),
                SemanticError::RecursiveResultAnnotationRequired { .. }
            ),
            "{source}"
        );
    }
    Compiler::new()
        .compile_source(
            "fn f(x: number) -> number { if x <= 0 { return 0; } return (x - 1) |> f; }",
        )
        .unwrap();
}

#[test]
fn all_concrete_types_and_supplied_defaults_preserve_full_arity() {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    for (ty, input) in [
        ("number", "1"),
        ("bool", "true"),
        ("string", "\"é\""),
        ("number[]", "[1]"),
        ("range", "1..3"),
    ] {
        let source =
            format!("fn id(x: {ty}) {{ return x; }} fn f() {{ return {input} |> id |> id(); }}");
        let bytes = compiler.compile_source(&source).unwrap();
        wasmparser::Validator::new().validate_all(&bytes).unwrap();
    }
    compiler.compile_source("fn pair(a: number = 7, b: number = a + 1) { return a + b; } fn f() { return 1 |> pair(2); }").unwrap();
}

#[test]
fn never_and_unreachable_pipeline_children_remain_statically_checked() {
    for source in [
        "fn id(x: number) -> number { return x; } fn f() -> number { return (loop {}) |> missing; }",
        "fn pair(a: number, b: number) -> number { return a; } fn f() -> number { return (loop {}) |> pair(unknown); }",
        "fn f() -> number { return 1; 2 |> missing; }",
        "fn f() -> bool { return false && (true |> missing); }",
    ] {
        assert!(
            matches!(
                error(source),
                SemanticError::UndefinedFunction { .. } | SemanticError::UndefinedVariable { .. }
            ),
            "{source}"
        );
    }
    let source = "fn id(x: number) -> number { return x; } fn f() -> number { return ({ return 7; 1 }) |> id; }";
    let typed = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &typed.functions[1].body[0].kind else {
        panic!()
    };
    assert_eq!(value.ty, ExprResult::Never);
    wasmparser::Validator::new()
        .validate_all(&Compiler::new().compile(&typed).unwrap())
        .unwrap();
}

#[test]
fn pipeline_compilation_uses_owned_import_snapshot_after_registry_changes() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("effect", "env", "effect", vec![Type::Number], Type::Number)
        .unwrap();
    let source = "fn wrap(x: number) { return effect(x); } fn main() { return 1 |> wrap; }";
    let typed = compiler.analyze(source).unwrap();
    let baseline = compiler.compile(&typed).unwrap();
    compiler
        .register_host_function("later", "extra", "later", vec![], Type::Void)
        .unwrap();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&typed).unwrap();
        assert_eq!(bytes, baseline);
        wasmparser::Validator::new().validate_all(&bytes).unwrap();
        let mut imports = Vec::new();
        for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
            if let wasmparser::Payload::ImportSection(section) = payload.unwrap() {
                for import in section.into_imports() {
                    let import = import.unwrap();
                    imports.push((import.module, import.name));
                }
            }
        }
        assert_eq!(imports, [("env", "effect")]);
    }
    assert_eq!(typed.host_functions.len(), 1);
    let reanalyzed = compiler.analyze(source).unwrap();
    assert_eq!(reanalyzed.host_functions.len(), 2);
    assert_ne!(compiler.compile(&reanalyzed).unwrap(), baseline);
}
