use was_compiler::{
    CompileError, Compiler, ExportOptions, Type,
    ast::Expr,
    lexer::Lexer,
    parser::Parser,
    semantic::SemanticError,
    tokens::Span,
    typed_ast::{TypedExprKind, TypedStatementKind},
};

#[test]
fn reordered_named_arguments_have_written_bindings_and_formal_order_reads() {
    let program = Compiler::new()
        .analyze(
            "fn subtract(a: number, b: number) -> number { return a - b; }
         fn main() -> number { return subtract(b: 2, a: 9); }",
        )
        .unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized call")
    };
    assert_eq!(sequence.bindings.len(), 2);
    assert_eq!(
        sequence.formal_to_binding,
        [sequence.bindings[1].id, sequence.bindings[0].id]
    );
    assert_eq!(sequence.bindings[0].ty, Type::Number);
    assert!(matches!(&sequence.bindings[0].value.kind, TypedExprKind::Number(n) if n == "2"));
    assert!(matches!(&sequence.bindings[1].value.kind, TypedExprKind::Number(n) if n == "9"));
}

fn semantic_error(source: &str) -> SemanticError {
    let Err(CompileError::Semantic(error)) = Compiler::new().analyze(source) else {
        panic!("expected semantic error for {source}");
    };
    error
}

#[test]
fn raw_arguments_retain_names_full_utf8_origins_and_nested_values() {
    let source = "fn f(a: string, b: number) -> void {} fn main() -> void { f(b /*trivia*/ : (2 + 3), a: \"é\"); }";
    let tokens = Lexer::new(source).tokenize();
    let program = Parser::new(&tokens).parse_program().unwrap();
    let was_compiler::ast::StatementKind::Expression(Expr::Call { args, .. }) =
        &program.functions[1].body[0].kind
    else {
        panic!("call")
    };
    assert_eq!(args[0].name.as_deref(), Some("b"));
    assert_eq!(
        &source[args[0].span.start()..args[0].span.end()],
        "b /*trivia*/ : (2 + 3)"
    );
    assert_eq!(
        &source[args[1].span.start()..args[1].span.end()],
        "a: \"é\""
    );
    assert!(matches!(&args[1].value, Expr::String(value) if value == "é"));
}

#[test]
fn named_structure_errors_precede_expression_errors_in_written_order() {
    let cases = [
        ("f(a: missing, z: missing)", "unknown", "z: missing"),
        ("f(a: missing, a: missing)", "duplicate", "a: missing"),
        ("f(missing, a: missing)", "duplicate", "a: missing"),
        ("f(a: missing, missing)", "after", "missing"),
        (
            "f(missing, missing, missing, a: missing)",
            "excess",
            "missing",
        ),
        ("f(a: missing)", "missing", ""),
        (
            "f(z: missing, a: missing, a: missing)",
            "unknown",
            "z: missing",
        ),
        (
            "f(a: missing, a: missing, z: missing)",
            "duplicate",
            "a: missing",
        ),
    ];
    for (call, kind, excerpt) in cases {
        let source =
            format!("fn f(a: number, b: number) -> void {{}} fn main() -> void {{ {call}; }}");
        let error = semantic_error(&source);
        match kind {
            "unknown" => assert!(
                matches!(&error, SemanticError::UnknownArgument { argument, .. } if argument == "z")
            ),
            "duplicate" => assert!(
                matches!(&error, SemanticError::DuplicateArgument { argument, .. } if argument == "a")
            ),
            "after" => assert!(matches!(&error, SemanticError::PositionalAfterNamed { .. })),
            "excess" => assert!(matches!(
                &error,
                SemanticError::TooManyPositionalArguments { .. }
            )),
            "missing" => assert!(
                matches!(&error, SemanticError::MissingArgument { argument, .. } if argument == "b")
            ),
            _ => unreachable!(),
        }
        let span = error.span().unwrap();
        if !excerpt.is_empty() {
            assert_eq!(&source[span.start()..span.end()], excerpt);
        }
    }
}

#[test]
fn expression_checks_follow_written_order_with_exact_types_and_origins() {
    let source = "fn f(a: number, b: bool) -> void {} fn main() -> void { f(b: 1, a: missing); }";
    let start = source.find("b: 1").unwrap();
    assert_eq!(
        semantic_error(source),
        SemanticError::TypeMismatch {
            expected: Type::Bool,
            found: Type::Number,
            span: Span::new(start, start + 4),
        }
    );
    let source =
        "fn f(a: number, b: bool) -> void {} fn main() -> void { f(b: missing_b, a: missing_a); }";
    assert!(
        matches!(semantic_error(source), SemanticError::UndefinedVariable { name, .. } if name == "missing_b")
    );
}

#[test]
fn lexical_shadowing_precedes_named_target_rejection() {
    assert!(
        matches!(semantic_error("fn main() -> void { let len = 1; len(xs: []); }"), SemanticError::NotCallable { name, .. } if name == "len")
    );
    assert!(matches!(
        semantic_error("fn main() -> void { absent(a: missing); }"),
        SemanticError::UndefinedFunction { .. }
    ));
    assert!(
        matches!(semantic_error("fn main() -> void { len(xs: missing); }"), SemanticError::NamedArgumentsNotSupported { name, .. } if name == "len")
    );
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("host", "env", "host", vec![Type::Number], Type::Void)
        .unwrap();
    assert!(matches!(
        compiler.analyze("fn main() -> void { host(value: missing); }"),
        Err(CompileError::Semantic(
            SemanticError::NamedArgumentsNotSupported { .. }
        ))
    ));
}

#[test]
fn mixed_prefix_forward_recursive_nested_and_all_value_types_compile() {
    let source = r#"
        fn main() -> number { return f(9, flag: true, xs: [2], text: "é", r: 0..3); }
        fn f(a: number, text: string, xs: number[], flag: bool, r: range) -> number {
            if flag { return f(a: f(1, r: r, flag: false, text: text, xs: xs), r: r, flag: false, xs: xs, text: text); }
            return if 2 in r && text == "é" { a + xs[0] } else { 0 };
        }
    "#;
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let program = compiler.analyze(source).unwrap();
    let bytes = compiler.compile(&program).unwrap();
    wasmparser::Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn unreachable_skipped_and_unselected_calls_still_receive_binding_checks() {
    for body in [
        "return; f(a: 1);",
        "false && { f(a: 1); true };",
        "if false { f(a: 1); }",
        "f(a: { return; }, b: true);",
    ] {
        let source = format!(
            "fn f(a: number, b: number) -> void {{}} fn main() -> void {{}} fn hidden() -> void {{ {body} }}"
        );
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        assert!(
            matches!(
                compiler.analyze(&source),
                Err(CompileError::Semantic(
                    SemanticError::MissingArgument { .. } | SemanticError::TypeMismatch { .. }
                ))
            ),
            "{source}"
        );
    }
}

#[test]
fn positional_calls_keep_legacy_tree_error_locations_and_bytes() {
    let source = "fn f(a: number, b: number) -> number { return a - b; } fn main() -> number { return f(9, 2); }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    assert!(matches!(&value.kind, TypedExprKind::Call { .. }));
    let bytes = compiler.compile(&program).unwrap();
    let equivalent = compiler
        .compile_source(&source.replace("f(9, 2)", "f(/*first*/9, /*second*/2)"))
        .unwrap();
    assert_eq!(bytes, equivalent);
    let source = "fn f(a: number) -> void {} fn main() -> void { f(false); }";
    let start = source.find("f(false)").unwrap();
    assert_eq!(
        semantic_error(source).span(),
        Some(Span::new(start, start + "f(false);".len()))
    );
    assert!(matches!(
        semantic_error("fn f(a: number) -> void {} fn main() -> void { f(missing, missing); }"),
        SemanticError::ArgumentCountMismatch { .. }
    ));
}

#[test]
fn malformed_named_syntax_remains_rejected() {
    for source in [
        "fn f(a) -> void {}",
        "fn main() -> void { f(a:); }",
        "fn main() -> void { f(a: 1,); }",
        "fn main() -> void { f(: 1); }",
        "fn main() -> void { f((a): 1); }",
        "fn main() -> void { f(a: 1: 2); }",
        "fn main() -> void { [a: 1]; }",
    ] {
        assert!(
            matches!(Compiler::new().analyze(source), Err(CompileError::Parse(_))),
            "{source}"
        );
    }
}
