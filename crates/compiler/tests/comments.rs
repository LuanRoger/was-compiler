use was_compiler::{
    CompileError, Compiler, OptimizationLevel, Type, diagnostic,
    lexer::Lexer,
    parser::{ParseError, Parser},
    semantic::SemanticError,
    tokens::{Span, TokenKind},
};
use wasmparser::Validator;

#[test]
fn comments_between_every_token_preserve_emission_at_all_levels() {
    let source = include_str!("../../../tests/fixtures/emitter/control.was");
    let golden = include_bytes!("../../../tests/fixtures/emitter/control.wasm");
    let mut commented = String::new();

    for token in Lexer::new(source).tokenize() {
        if token.kind == TokenKind::Eof {
            break;
        }

        commented.push_str("/* é /* non-nesting */ // ignored\r\n");
        commented.push_str(&source[token.span.start()..token.span.end()]);
    }

    commented.push_str("// trailing comment at EOF");

    let mut compiler = Compiler::new();

    compiler
        .register_host_function("marker", "env", "marker", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("signal", "env", "signal", vec![Type::Bool], Type::Bool)
        .unwrap();

    let program = compiler.analyze(&commented).unwrap();

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);

        let wasm = compiler.compile(&program).unwrap();

        Validator::new().validate_all(&wasm).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(wasm, golden, "baseline typed compilation");
        }
        assert_eq!(wasm, compiler.compile_source(source).unwrap());
        assert_eq!(wasm, compiler.compile(&program).unwrap());
        assert_eq!(
            compiler.compile_source(&commented).unwrap(),
            wasm,
            "source compilation at {level:?}"
        );
    }
}

#[test]
fn comments_do_not_join_identifier_or_operator_fragments_in_source() {
    for source in [
        "fn ma/**/in() -> void {}",
        "fn main() -/**/> void {}",
        "fn main() -> bool { return true &/**/& false; }",
        "fn main() -> bool { return 1 =/**/= 1; }",
    ] {
        assert!(
            matches!(Compiler::new().analyze(source), Err(CompileError::Parse(_))),
            "{source}"
        );
    }
}

#[test]
fn comment_only_source_still_requires_a_function() {
    let source = "// é\r\n/* multiline\n comment */ // EOF";
    let tokens = Lexer::new(source).tokenize();

    assert_eq!(
        Parser::new(&tokens).parse_program().unwrap_err(),
        ParseError::UnexpectedToken {
            expected: "fn",
            found: "end of input",
            span: Span::new(source.len(), source.len()),
        }
    );
}

#[test]
fn unterminated_block_comments_report_original_spans_through_parse_errors() {
    for (source, expected) in [
        ("// é\r\n/* unfinished", "fn"),
        (
            "fn main() -> number { return /* é\n unfinished",
            "expression",
        ),
        ("fn main() -> void {} /* trailing *", "fn"),
    ] {
        let start = source.find("/*").unwrap();
        let span = Span::new(start, source.len());
        let compiler = Compiler::new();
        let error = compiler.compile_source(source).unwrap_err();

        assert_eq!(
            error,
            CompileError::Parse(ParseError::UnexpectedToken {
                expected,
                found: "invalid token",
                span,
            })
        );
        assert_eq!(error.span(), Some(span));
        assert!(compiler.compile_source("fn main() -> void {}").is_ok());
    }
}

#[test]
fn errors_after_comments_keep_original_utf8_offsets_and_columns() {
    let source = "// é\r\nfn main() -> number {\r\n    /*🙂*/ return missing;\r\n}";
    let error = Compiler::new().compile_source(source).unwrap_err();

    assert_eq!(
        error,
        CompileError::Semantic(SemanticError::UndefinedVariable {
            name: "missing".into(),
            span: Span::new(43, 58),
        })
    );

    let rendered = diagnostic::render("comments.was", source, &error);

    assert!(rendered.starts_with("comments.was:3:11: semantic error:"));
    assert!(rendered.contains("3 |     /*🙂*/ return missing;"));

    let source = "/*é*/\r\nfn main() -> number {\r\n    return 1 /*🙂*/\r\n}";
    let error = Compiler::new().compile_source(source).unwrap_err();

    assert_eq!(
        error,
        CompileError::Parse(ParseError::UnexpectedToken {
            expected: ";",
            found: "}",
            span: Span::new(54, 55),
        })
    );
    assert!(
        diagnostic::render("comments.was", source, &error)
            .starts_with("comments.was:4:1: parse error:")
    );
}
