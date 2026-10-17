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
fn named_defaults_preserve_mapping_origins_prefixes_and_declaration_spans() {
    let source = "/*🦀*/ fn stage(a: number = 99, b: number = a + 1, c: number = b + 1, d: number = c + 1) { return d; } fn main() { return (2) /*λ*/ |> stage(d: 8, b: 4) |> stage(); }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(outer) = &value.kind else {
        panic!("outer stage")
    };
    assert_eq!(
        outer.bindings[0].span,
        span(source, "(2) /*λ*/ |> stage(d: 8, b: 4)")
    );
    let TypedExprKind::NormalizedCall(inner) = &outer.bindings[0].value.kind else {
        panic!("inner stage")
    };
    assert_eq!(inner.bindings[0].span, span(source, "(2)"));
    assert_eq!(inner.bindings[1].span, span(source, "d: 8"));
    assert_eq!(inner.bindings[2].span, span(source, "b: 4"));
    assert_eq!(inner.bindings[3].span, span(source, "b + 1"));
    assert_eq!(
        inner.bindings.iter().map(|b| b.origin).collect::<Vec<_>>(),
        [
            CallBindingOrigin::PipelineInput,
            CallBindingOrigin::Explicit { argument: 0 },
            CallBindingOrigin::Explicit { argument: 1 },
            CallBindingOrigin::Default { formal: 2 },
        ]
    );
    assert_eq!(
        inner.formal_to_binding,
        [
            inner.bindings[0].id,
            inner.bindings[2].id,
            inner.bindings[3].id,
            inner.bindings[1].id
        ]
    );
    let TypedExprKind::Binary { left, .. } = &inner.bindings[3].value.kind else {
        panic!("default")
    };
    assert!(
        matches!(left.kind, TypedExprKind::Identifier { id, .. } if id == inner.bindings[2].id)
    );
    assert_eq!(
        outer.bindings.iter().map(|b| b.origin).collect::<Vec<_>>(),
        [
            CallBindingOrigin::PipelineInput,
            CallBindingOrigin::Default { formal: 1 },
            CallBindingOrigin::Default { formal: 2 },
            CallBindingOrigin::Default { formal: 3 },
        ]
    );
    wasmparser::Validator::new()
        .validate_all(&compiler.compile(&program).unwrap())
        .unwrap();
    let bare = source.replace("|> stage();", "|> stage;");
    assert_eq!(
        compiler.compile_source(source).unwrap(),
        compiler.compile_source(&bare).unwrap()
    );
}

#[test]
fn named_pipeline_binding_errors_keep_written_spans_and_structural_priority() {
    enum BindingError {
        Duplicate,
        Unknown,
        PositionalAfterNamed,
        TooMany,
        Type,
    }

    let declaration =
        "fn stage(a: number = 9, b: number = a, c: number = b) -> number { return c; }";
    for (args, origin, expected) in [
        ("a: unknown", "a: unknown", BindingError::Duplicate),
        ("b: unknown, b: 2", "b: 2", BindingError::Duplicate),
        ("bad: unknown", "bad: unknown", BindingError::Unknown),
        ("b: unknown, 2", "2)", BindingError::PositionalAfterNamed),
        ("1, 2, unknown", "unknown", BindingError::TooMany),
        ("b: true", "b: true", BindingError::Type),
    ] {
        let source =
            format!("{declaration} fn main() -> number {{ return (2) |> stage({args}); }}");
        let err = error(&source);
        assert!(
            match expected {
                BindingError::Duplicate => matches!(err, SemanticError::DuplicateArgument { .. }),
                BindingError::Unknown => matches!(err, SemanticError::UnknownArgument { .. }),
                BindingError::PositionalAfterNamed =>
                    matches!(err, SemanticError::PositionalAfterNamed { .. }),
                BindingError::TooMany =>
                    matches!(err, SemanticError::TooManyPositionalArguments { .. }),
                BindingError::Type => matches!(err, SemanticError::TypeMismatch { .. }),
            },
            "{err:?}"
        );
        let origin = if matches!(expected, BindingError::PositionalAfterNamed) {
            Span::new(
                source.rfind(origin).unwrap(),
                source.rfind(origin).unwrap() + 1,
            )
        } else {
            span(&source, origin)
        };
        assert_eq!(err.span(), Some(origin));
    }
    for stage in ["required", "required()", "required(c: unknown)"] {
        let source = format!(
            "fn required(a: number, b: number, c: number = 3) -> number {{ return a; }} fn main() -> number {{ return 1 |> {stage}; }}"
        );
        let err = error(&source);
        assert!(matches!(&err, SemanticError::MissingArgument { argument, .. } if argument == "b"));
        let start = source.rfind(stage).unwrap();
        assert_eq!(err.span(), Some(Span::new(start, start + stage.len())));
    }
}

#[test]
fn named_defaults_keep_declaration_resolution_hygiene_and_inference_dependencies() {
    let source = "fn root() { let x = 99; return 2 |> left(extra: 4) |> right; } fn left(x: number, extra: number = x + 1) { return x |> leaf(add: extra); } fn right(x: number, extra: number = x + 1) { return x |> leaf(add: extra); } fn leaf(x: number, add: number = x + 1) { return x + add; } fn nested() { return 1 |> leaf(add: 2 |> right); }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    assert!(
        program
            .functions
            .iter()
            .all(|f| f.return_type == Type::Number)
    );
    assert_eq!(
        program
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["root", "left", "right", "leaf", "nested"]
    );
    wasmparser::Validator::new()
        .validate_all(&compiler.compile(&program).unwrap())
        .unwrap();
    for source in [
        "fn f(x: number, y: number = x) { return x |> f(y: y); }",
        "fn f(x: number, y: number = x) { return 1; x |> g; } fn g(x: number, y: number = x) -> number { return x |> f(y: y); }",
    ] {
        assert!(matches!(
            error(source),
            SemanticError::RecursiveResultAnnotationRequired { .. }
        ));
    }
    Compiler::new().compile_source("fn f(x: number, y: number = x) -> number { if x <= 0 { return y; } return (x - 1) |> f; }").unwrap();
    assert!(
        matches!(error("fn stage(a: number, b: number = a) -> number { return b; } fn main() -> number { return 1 |> stage(b: a); }"), SemanticError::UndefinedVariable { name, .. } if name == "a")
    );
}

#[test]
fn named_and_default_children_are_checked_after_never_and_in_skipped_source() {
    for source in [
        "fn stage(a: number, b: number = a) -> number { return b; } fn main() -> number { return (loop {}) |> stage(b: unknown); }",
        "fn stage(a: number, b: number = a) -> number { return b; } fn main() -> number { return (loop {}) |> stage |> stage(b: unknown); }",
        "fn stage(a: bool, b: bool = a) -> bool { return b; } fn main() -> bool { return false && (true |> stage(b: unknown)); }",
        "fn stage(a: number, b: number = a) -> number { return b; } fn main() -> number { return 1; 2 |> stage(b: unknown); }",
    ] {
        assert!(
            matches!(error(source), SemanticError::UndefinedVariable { name, .. } if name == "unknown")
        );
    }
    let compiler = Compiler::new();
    let source = "fn stage(a: number, b: number = a) -> number { return b; } fn main() -> number { return ({ return 7; 1 }) |> stage; }";
    wasmparser::Validator::new()
        .validate_all(&compiler.compile_source(source).unwrap())
        .unwrap();
    assert!(
        matches!(error("fn stage(a: number, b: number = unknown) -> number { return b; } fn main() -> number { return (loop {}) |> stage(b: 2); }"), SemanticError::UndefinedVariable { name, .. } if name == "unknown")
    );
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
fn stage_eligibility_shadowing_and_void_restrictions_are_explicit() {
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
            "fn discard(x: number) {} fn f() { let x = 1 |> discard; }",
            "a final void stage requires a discarded result",
        ),
        (
            "fn discard(x: number) {} fn f() -> number { return (loop {}) |> discard |> missing; }",
            "a nonfinal stage cannot return void",
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
    Compiler::new()
        .compile_source(
            "fn pair(a: number, b: number = 2) { return a + b; } fn f() { return 1 |> pair; }",
        )
        .unwrap();
    Compiler::new()
        .compile_source(
            "fn pair(a: number, b: number) { return a + b; } fn f() { return 1 |> pair(b: 2); }",
        )
        .unwrap();
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
fn final_void_stages_inherit_only_discarded_control_tails() {
    for body in [
        "1 |> sink;",
        "((1 |> sink()));",
        "{ { 1 |> sink } };",
        "(if true { 1 |> sink } else { 2 |> sink() });",
        "if true { 1 |> sink; } else { 2 |> sink; }",
        "match true { true => (1 |> sink), false => 7, }",
        "{ match true { true => { 1 |> sink }, false => { 2 |> sink }, } };",
        "{ if true { { 1 |> sink } } else { 2 |> sink } };",
        "return; { 1 |> sink };",
    ] {
        let source = format!("fn sink(x: number, n: number = x + 1) {{}} fn f() {{ {body} }}");
        let program = Compiler::new()
            .analyze(&source)
            .unwrap_or_else(|err| panic!("{body}: {err}"));
        Compiler::new().compile(&program).unwrap();
        assert_eq!(program.functions[1].return_type, Type::Void);
    }
}

#[test]
fn discard_permission_stops_at_values_operands_conditions_and_arguments() {
    for body in [
        "let x = { 1 |> sink };",
        "let x: number = { 1 |> sink };",
        "return { 1 |> sink };",
        "take({ 1 |> sink });",
        "take(if true { 1 |> sink } else { 2 |> sink });",
        "({ 1 |> sink }) |> take;",
        "(1 |> sink) + 2;",
        "-(1 |> sink);",
        "!(1 |> sink);",
        "false && ({ 1 |> sink });",
        "true || ({ 1 |> sink });",
        "[(1 |> sink)];",
        "let xs = [1]; xs[(1 |> sink)];",
        "(1 |> sink)..2;",
        "1..2 by (1 |> sink);",
        "1 in (1 |> sink);",
        "if ({ 1 |> sink }) {}",
        "while ({ 1 |> sink }) {}",
        "for x in ({ 1 |> sink }) {}",
        "match ({ 1 |> sink }) { _ => 0, }",
        "let x = match true { _ => { 1 |> sink }, };",
        "let x = loop { break { 1 |> sink }; };",
        "let xs = [1]; xs[0] = { 1 |> sink };",
        "let x = 1; x += { 1 |> sink };",
    ] {
        let source = format!(
            "fn sink(x: number) {{}} fn take(x: number) -> number {{ return x; }} fn f() {{ {body} }}"
        );
        let stage_start = source.find("|> sink").unwrap() + 3;
        assert_eq!(
            error(&source),
            SemanticError::InvalidPipeline {
                message: "a final void stage requires a discarded result",
                span: Span::new(stage_start, stage_start + 4),
            },
            "{body}"
        );
    }
}

#[test]
fn discarded_expression_joins_stay_structural_and_statement_matches_discard_each_arm() {
    for body in [
        "(if true { 1 |> sink } else { 2 });",
        "{ match true { true => (1 |> sink), false => 2, } };",
        "(match true { true => (1 |> sink), false => 2, });",
    ] {
        let source = format!("fn sink(x: number) {{}} fn f() {{ {body} }}");
        assert!(
            matches!(
                error(&source),
                SemanticError::TypeMismatch {
                    expected: Type::Void,
                    found: Type::Number,
                    ..
                }
            ),
            "{body}"
        );
    }
    Compiler::new()
        .compile_source(
            "fn sink(x: number) {} fn f() { match true { true => { 1 |> sink }, false => 2, }; }",
        )
        .unwrap();
    // Ordinary Void calls retain their prior source semantics in value blocks.
    assert!(matches!(
        error("fn sink(x: number) {} fn f() { let x = { sink(1) }; }"),
        SemanticError::InvalidVoidType { name, .. } if name == "x"
    ));
    Compiler::new()
        .compile_source("fn sink(x: number) {} fn f() { (if true { sink(1) } else { sink(2) }); }")
        .unwrap();
}

#[test]
fn void_stage_context_errors_keep_exact_spans_after_never_and_skipped_children() {
    let declarations =
        "fn sink(x: number, n: number = x + 1) {} fn id(x: number) -> number { return x; }";
    for (body, message, stage) in [
        (
            "let x = (1) /*é🦀*/ |> sink();",
            "a final void stage requires a discarded result",
            "sink()",
        ),
        (
            "let x = ({ return; 1 }) |> sink;",
            "a final void stage requires a discarded result",
            "sink",
        ),
        (
            "return; (1 |> sink) + 2;",
            "a final void stage requires a discarded result",
            "sink",
        ),
        (
            "false && ({ 1 |> sink });",
            "a final void stage requires a discarded result",
            "sink",
        ),
        (
            "(loop {}) |> sink |> id;",
            "a nonfinal stage cannot return void",
            "sink",
        ),
        (
            "1 |> sink(n: { return; 2 }) |> id;",
            "a nonfinal stage cannot return void",
            "sink(n: { return; 2 })",
        ),
        (
            "({ return; 1 }) |> sink |> missing;",
            "a nonfinal stage cannot return void",
            "sink",
        ),
    ] {
        let source = format!("{declarations}\r\n// λ\r\nfn f() {{\r\n\t{body}\r\n}}");
        let start = source.rfind("|> sink").unwrap() + 3;
        let expected_span = Span::new(start, start + stage.len());
        let err = error(&source);
        assert_eq!(
            err,
            SemanticError::InvalidPipeline {
                message,
                span: expected_span
            },
            "{body}"
        );
        assert_eq!(err.span(), Some(expected_span));
        assert_eq!(
            err.to_string(),
            format!("Invalid pipeline: {message} at {expected_span:?}")
        );
    }
    let source = format!("{declarations} fn f() {{ ({{ return; 1 }}) |> sink; }}");
    let program = Compiler::new().analyze(&source).unwrap();
    let TypedStatementKind::Expression(value) = &program.functions[2].body[0].kind else {
        panic!("expression")
    };
    assert_eq!(value.ty, ExprResult::Never);
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized call")
    };
    assert_eq!(sequence.call.ty, ExprResult::Void);
    wasmparser::Validator::new()
        .validate_all(&Compiler::new().compile(&program).unwrap())
        .unwrap();
}

#[test]
fn void_stages_keep_declaration_inference_and_written_error_precedence() {
    for (source, expected_name) in [
        (
            "fn sink(x: number, n: number) {} fn f() { missing |> sink(also_missing); }",
            "missing",
        ),
        (
            "fn sink(x: number, n: number) {} fn f() { 1 |> sink(missing) |> absent; }",
            "missing",
        ),
        (
            "fn sink(x: number, n: number) {} fn f() { let x = 1 |> sink(missing); }",
            "missing",
        ),
        (
            "fn sink(x: number, n: number = missing) {} fn f() { unknown |> sink; }",
            "missing",
        ),
    ] {
        assert!(
            matches!(error(source), SemanticError::UndefinedVariable { name, .. } if name == expected_name),
            "{source}"
        );
    }
    assert!(matches!(
        error("fn sink(x: number, n: number) {} fn f() { let x = 1 |> sink(bad: unknown); }"),
        SemanticError::UnknownArgument { .. }
    ));
    let source = "fn f() { 1 |> sink; } fn sink(x: number, n: number = x + 1) { x |> action; } fn action(x: number) {}";
    let program = Compiler::new().analyze(source).unwrap();
    assert!(
        program
            .functions
            .iter()
            .all(|function| function.return_type == Type::Void)
    );
    Compiler::new().compile(&program).unwrap();
    assert!(matches!(
        error("fn f(x: number) { x |> g; } fn g(x: number) { x |> f; }"),
        SemanticError::RecursiveResultAnnotationRequired { .. }
    ));
    Compiler::new().compile_source(
        "fn f(x: number) -> void { if x > 0 { (x - 1) |> g; } } fn g(x: number) -> void { x |> f; }",
    ).unwrap();
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
        let selected_bytes = compiler.compile_source(source).unwrap();
        wasmparser::Validator::new()
            .validate_all(&selected_bytes)
            .unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(selected_bytes, baseline);
        }
        let baseline = selected_bytes;
        assert_eq!(compiler.compile_source(source).unwrap(), baseline);
        assert_eq!(compiler.compile(&typed).unwrap(), baseline);
        assert_eq!(compiler.analyze(source).unwrap(), typed);
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
        if level == OptimizationLevel::None {
            assert_eq!(bytes, baseline);
        }
        assert_eq!(compiler.compile(&typed).unwrap(), bytes);
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
