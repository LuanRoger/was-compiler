use was_compiler::{
    CompileError, Compiler, HostRegistrationError, Type,
    ast::{BinaryOp, Expr, Program, StatementKind, UnaryOp},
    lexer::Lexer,
    parser::{ParseError, Parser},
    semantic::SemanticError,
    tokens::{Span, TokenKind},
    typed_ast::{ExprResult, TypedExprKind, TypedStatementKind},
};

fn parse(source: &str) -> Result<Program, ParseError> {
    let tokens = Lexer::new(source).tokenize();
    Parser::new(&tokens).parse_program()
}

fn expression(source: &str) -> Expr {
    let mut program = parse(&format!("fn main() -> void {{ {source}; }}")).unwrap();
    let StatementKind::Expression(expr) = program.functions[0].body.remove(0).kind else {
        panic!("expected expression statement");
    };
    expr
}

fn number(value: &str) -> Expr {
    Expr::Number(value.into())
}

fn binary(left: Expr, op: BinaryOp, right: Expr) -> Expr {
    Expr::Binary {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }
}

fn range(start: Expr, end: Expr, step: Option<Expr>, inclusive: bool) -> Expr {
    Expr::Range {
        start: Box::new(start),
        end: Box::new(end),
        step: step.map(Box::new),
        inclusive,
    }
}

fn semantic_error(source: &str) -> SemanticError {
    match Compiler::new().analyze(source).unwrap_err() {
        CompileError::Semantic(error) => error,
        error => panic!("expected semantic error for {source}: {error:?}"),
    }
}

#[test]
fn range_punctuation_preserves_numbers_and_utf8_byte_spans() {
    let source = "/*é🦀*/1..=2.5 by -2";
    let tokens = Lexer::new(source).tokenize();
    assert_eq!(tokens[0].kind, TokenKind::NumberLiteral("1".into()));
    assert_eq!(tokens[0].span, Span::new(10, 11));
    assert_eq!(tokens[1].kind, TokenKind::DotDotEqual);
    assert_eq!(tokens[1].span, Span::new(11, 14));
    assert_eq!(tokens[2].kind, TokenKind::NumberLiteral("2.5".into()));
    assert_eq!(tokens[2].span, Span::new(14, 17));
    assert!(matches!(&tokens[3].kind, TokenKind::Identifier(name) if name == "by"));
    assert_eq!(tokens[3].span, Span::new(18, 20));
    assert_eq!(tokens[4].kind, TokenKind::Minus);
    assert_eq!(tokens[4].span, Span::new(21, 22));
    assert_eq!(tokens[5].span, Span::new(22, 23));
    assert_eq!(tokens[6].span, Span::new(23, 23));
}

#[test]
fn decimals_stop_at_ranges_without_accepting_other_malformed_decimals() {
    for (source, first, punctuation, end) in [
        ("1..25", "1", TokenKind::DotDot, "25"),
        ("1.25..=2.5", "1.25", TokenKind::DotDotEqual, "2.5"),
    ] {
        let tokens = Lexer::new(source).tokenize();
        assert_eq!(tokens[0].kind, TokenKind::NumberLiteral(first.into()));
        assert_eq!(tokens[1].kind, punctuation);
        assert_eq!(tokens[2].kind, TokenKind::NumberLiteral(end.into()));
        assert_eq!(tokens[3].kind, TokenKind::Eof);
    }
    for source in [".5", "1.", "1.2.3", ".1.2"] {
        let tokens = Lexer::new(source).tokenize();
        assert_eq!(tokens.len(), 2, "{source}");
        assert_eq!(tokens[0].kind, TokenKind::Invalid, "{source}");
        assert_eq!(tokens[0].span, Span::new(0, source.len()));
        assert_eq!(tokens[1].kind, TokenKind::Eof);
    }
    let tokens = Lexer::new("1...2").tokenize();
    assert_eq!(tokens[0].kind, TokenKind::NumberLiteral("1".into()));
    assert_eq!(tokens[1].kind, TokenKind::DotDot);
    assert_eq!(tokens[2].kind, TokenKind::Invalid);
    assert_eq!(tokens[2].span, Span::new(3, 5));
}

#[test]
fn range_tokens_preserve_trivia_string_and_keyword_boundaries() {
    let tokens = Lexer::new("range range1 by by2 \"..= by range\" ./*é*/. ../*x*/=").tokenize();
    let kinds: Vec<_> = tokens.iter().map(|token| token.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Range,
            TokenKind::Identifier("range1".into()),
            TokenKind::Identifier("by".into()),
            TokenKind::Identifier("by2".into()),
            TokenKind::StringLiteral("..= by range".into()),
            TokenKind::Invalid,
            TokenKind::Invalid,
            TokenKind::DotDot,
            TokenKind::Equal,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn parses_range_annotations_arithmetic_and_contextual_steps() {
    let source =
        "fn by(by: number) -> range { let r: range = by + 1..=10 * 2 by -by + 2; return r; }";
    let program = parse(source).unwrap();
    assert_eq!(program.functions[0].return_type, Some(Type::Range));
    let StatementKind::Let { ty, value, .. } = &program.functions[0].body[0].kind else {
        panic!("expected local declaration");
    };
    assert_eq!(*ty, Some(Type::Range));
    assert_eq!(
        *value,
        range(
            binary(Expr::Identifier("by".into()), BinaryOp::Add, number("1")),
            binary(number("10"), BinaryOp::Multiply, number("2")),
            Some(binary(
                Expr::Unary {
                    op: UnaryOp::Negate,
                    expr: Box::new(Expr::Identifier("by".into())),
                },
                BinaryOp::Add,
                number("2")
            )),
            true,
        )
    );
    assert_eq!(
        expression("0..by by by"),
        range(
            number("0"),
            Expr::Identifier("by".into()),
            Some(Expr::Identifier("by".into())),
            false,
        )
    );
    assert!(parse("fn copy(r: range) -> range { return r; }").is_ok());
    Compiler::new().analyze(source).unwrap();
    assert!(parse("fn range() -> void {}").is_err());
}

#[test]
fn ranges_bind_below_arithmetic_above_comparison_and_equality() {
    assert_eq!(
        expression("1 + 2..3 * 4 by 5 - 6 < 7 == false"),
        binary(
            binary(
                range(
                    binary(number("1"), BinaryOp::Add, number("2")),
                    binary(number("3"), BinaryOp::Multiply, number("4")),
                    Some(binary(number("5"), BinaryOp::Subtract, number("6"))),
                    false,
                ),
                BinaryOp::LessThan,
                number("7")
            ),
            BinaryOp::Equal,
            Expr::Bool(false),
        )
    );
    assert_eq!(
        expression("1 < 2..3"),
        binary(
            number("1"),
            BinaryOp::LessThan,
            range(number("2"), number("3"), None, false)
        )
    );
    assert_eq!(
        expression("0..1 < 2..3"),
        binary(
            range(number("0"), number("1"), None, false),
            BinaryOp::LessThan,
            range(number("2"), number("3"), None, false),
        )
    );
    assert_eq!(
        expression("0..(1..2)"),
        range(
            number("0"),
            range(number("1"), number("2"), None, false),
            None,
            false
        )
    );
    assert_eq!(
        expression("(0..1)..2"),
        range(
            range(number("0"), number("1"), None, false),
            number("2"),
            None,
            false
        )
    );
    // Existing equality/comparison chains remain left-associative syntax.
    assert_eq!(
        expression("1 < 2 < 3"),
        binary(
            binary(number("1"), BinaryOp::LessThan, number("2")),
            BinaryOp::LessThan,
            number("3")
        )
    );
    assert_eq!(
        expression("true == false != true"),
        binary(
            binary(Expr::Bool(true), BinaryOp::Equal, Expr::Bool(false)),
            BinaryOp::NotEqual,
            Expr::Bool(true)
        )
    );
}

#[test]
fn malformed_ranges_and_chains_are_parser_errors_with_locations() {
    for expr in [
        "0..1..2",
        "0..=1..2",
        "0..1 by 2..3",
        "0..1 by 2 by 3",
        "..1",
        "0..",
        "0..=",
        "0..1 by",
        "0 by 1",
        "0. .1",
        "0.. =1",
    ] {
        let source = format!("fn main() -> void {{ {expr}; }}");
        let error = parse(&source).unwrap_err();
        assert!(error.span().is_some(), "{expr}: {error:?}");
    }
    for source in ["fn main() -> void { 0..", "fn main() -> void { 0..1 by"] {
        let error = parse(source).unwrap_err();
        assert_eq!(error.span(), Some(Span::new(source.len(), source.len())));
    }
    let source = "fn main() -> void { /*é🦀*/ 0..1..2; }";
    let error = parse(source).unwrap_err();
    let start = source.rfind("..").unwrap();
    assert_eq!(error.span(), Some(Span::new(start, start + 2)));
}

#[test]
fn range_locals_copies_assignment_and_internal_calls_have_concrete_types() {
    let source = r#"
        fn main() -> void {
            let by = 2;
            let inferred = 0..10 by by;
            let annotated: range = inferred;
            annotated = copy(inferred);
            let called = make();
        }
        fn copy(input: range) -> range { return input; }
        fn make() -> range { return -3..=3; }
    "#;
    let program = Compiler::new().analyze(source).unwrap();
    for statement in [
        &program.functions[0].body[1],
        &program.functions[0].body[2],
        &program.functions[0].body[4],
    ] {
        let TypedStatementKind::Let { ty, value, .. } = &statement.kind else {
            panic!("expected local");
        };
        assert_eq!(*ty, Type::Range);
        assert_eq!(value.ty, ExprResult::Value(Type::Range));
    }
    let TypedStatementKind::Let { value, .. } = &program.functions[0].body[1].kind else {
        unreachable!();
    };
    let TypedExprKind::Range {
        start,
        end,
        step,
        inclusive,
    } = &value.kind
    else {
        panic!("expected range");
    };
    assert_eq!(start.ty, ExprResult::Value(Type::Number));
    assert_eq!(end.ty, ExprResult::Value(Type::Number));
    assert_eq!(step.as_ref().unwrap().ty, ExprResult::Value(Type::Number));
    assert!(!inclusive);
    assert_eq!(program.functions[1].params[0].ty, Type::Range);
    assert_eq!(program.functions[1].return_type, Type::Range);
}

#[test]
fn range_blocks_ifs_and_loops_join_exact_concrete_results() {
    let source = r#"
        fn main(flag: bool) -> range {
            let block = { let r = 0..2; r };
            let branch = if flag { block } else { 2..=4 by 2 };
            let value_loop = (loop { if flag { break branch; } break 0..3; });
            let never_arm = if flag { value_loop } else { loop {} };
            return never_arm;
        }
    "#;
    let program = Compiler::new().analyze(source).unwrap();
    for statement in &program.functions[0].body[..4] {
        let TypedStatementKind::Let { ty, value, .. } = &statement.kind else {
            panic!("expected local");
        };
        assert_eq!(*ty, Type::Range);
        assert_eq!(value.ty, ExprResult::Value(Type::Range));
    }
    for body in [
        "let r = if true { 0..1 } else { 2 };",
        "let r = (loop { if true { break 0..1; } break 2; });",
        "let r: number = 0..1;",
        "let r: range = 1;",
        "let r = 0..1; r = 2;",
        "let r = 0..1; r += 2;",
        "let r = (0..1)..2;",
        "let r = 0..(1..2);",
    ] {
        assert!(
            matches!(
                semantic_error(&format!("fn main() -> void {{ {body} }}")),
                SemanticError::TypeMismatch { .. }
            ),
            "{body}"
        );
    }
}

#[test]
fn range_operand_types_are_checked_even_after_divergence_or_in_skipped_code() {
    for (body, found) in [
        ("true..2;", Type::Bool),
        ("0..\"end\";", Type::String),
        ("0..2 by [];", Type::NumberArray),
        ("0..2 by noop();", Type::Void),
        ("(loop {})..true;", Type::Bool),
        ("0..(loop {}) by false;", Type::Bool),
        ("return; 0..true;", Type::Bool),
        ("false && { 0..true; true };", Type::Bool),
        ("if true {} else { 0..2 by false; }", Type::Bool),
    ] {
        let source = format!("fn main() -> void {{ {body} }} fn noop() -> void {{}}");
        assert!(
            matches!(semantic_error(&source), SemanticError::TypeMismatch {
            expected: Type::Number, found: actual, ..
        } if actual == found),
            "{body}"
        );
    }
    for body in ["(loop {})..unknown;", "0..(loop {}) by unknown;"] {
        assert!(
            matches!(semantic_error(&format!("fn main() -> void {{ {body} }}")),
            SemanticError::UndefinedVariable { name, .. } if name == "unknown")
        );
    }
    let source = "fn main() -> void { /*é🦀*/ 0..true; }";
    let start = source.find("0..").unwrap();
    assert_eq!(
        semantic_error(source),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: Span::new(start, source.find(';').unwrap() + 1),
        }
    );
}

#[test]
fn divergent_range_operands_produce_never_without_hiding_later_checks() {
    for expr in ["(loop {})..2", "0..(loop {})", "0..2 by (loop {})"] {
        let source = format!("fn main() -> number {{ let r: range = {expr}; }}");
        let program = Compiler::new().analyze(&source).unwrap();
        let TypedStatementKind::Let { ty, value, .. } = &program.functions[0].body[0].kind else {
            panic!("expected local");
        };
        assert_eq!(*ty, Type::Range);
        assert_eq!(value.ty, ExprResult::Never);
        assert!(matches!(value.kind, TypedExprKind::Range { .. }));
        let source = format!("fn main() -> void {{ let r = {expr}; }}");
        assert!(
            matches!(semantic_error(&source), SemanticError::CannotInferNeverType { name, .. } if name == "r")
        );
    }
    // A transfer in an operand exits the surrounding loop, not range construction.
    let program = Compiler::new()
        .analyze("fn main() -> number { loop { 0..{ break; 2 } by 1; } return 7; }")
        .unwrap();
    let TypedStatementKind::Loop { body, .. } = &program.functions[0].body[0].kind else {
        panic!("expected loop");
    };
    let TypedStatementKind::Expression(value) = &body[0].kind else {
        panic!("expected range");
    };
    assert_eq!(value.ty, ExprResult::Never);
}

#[test]
fn range_equality_and_inequality_have_no_identity_or_sequence_contract() {
    for op in ["==", "!="] {
        for expr in [
            format!("(0..1) {op} (0..1)"),
            format!("(0..1) {op} (loop {{}})"),
            format!("(loop {{}}) {op} (0..1)"),
        ] {
            for body in [
                format!("{expr};"),
                format!("return; {expr};"),
                format!("false && ({expr});"),
            ] {
                let error = semantic_error(&format!("fn main() -> void {{ {body} }}"));
                assert!(
                    matches!(error, SemanticError::InvalidBinaryOperands {
                    operator, left: Type::Range, right: Type::Range, ..
                } if operator == op),
                    "{body}: {error:?}"
                );
            }
        }
    }
}

#[test]
fn invalid_numeric_domains_are_not_new_static_errors() {
    for expr in [
        "0.5..2",
        "0..2 by 0",
        "0..2 by -0",
        "-0..=0",
        "9007199254740992..0",
        "0..-9007199254740992 by 1",
        "(0 / 0)..1",
        "0..(1 / 0)",
        "0..0 by (0 / 0)",
        "10..0",
        "0..0 by 0.5",
    ] {
        let source = format!("fn main() -> void {{ {expr}; }}");
        Compiler::new().analyze(&source).unwrap();
    }
}

#[test]
fn range_inference_preserves_initializer_scope_and_first_error_order() {
    let source = "fn main() -> void { let r = 0..1; if true { let r = r; r = 1..2; } }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::If { then, .. } = &program.functions[0].body[1].kind else {
        panic!("expected statement if");
    };
    let TypedStatementKind::Let { ty, value, .. } = &then[0].kind else {
        panic!("expected shadowing local");
    };
    assert_eq!(*ty, Type::Range);
    assert_eq!(value.ty, ExprResult::Value(Type::Range));

    assert!(matches!(semantic_error("fn main() -> void { let r = r; }"),
        SemanticError::UndefinedVariable { name, .. } if name == "r"));
    assert!(matches!(
        semantic_error("fn main() -> void { true..unknown by missing; }"),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        }
    ));
    assert!(
        matches!(semantic_error("fn main() -> void { 0..unknown by false; }"),
        SemanticError::UndefinedVariable { name, .. } if name == "unknown")
    );
    assert!(
        matches!(semantic_error("fn main() -> void { let r = 0..1; let r = 2..3; }"),
        SemanticError::DuplicateVariable { name, .. } if name == "r")
    );
}

#[test]
fn membership_of_a_number_in_a_range_analyzes_as_bool() {
    let program = Compiler::new()
        .analyze("fn main() -> bool { let contains = 6 in (0..10 by 2); return contains; }")
        .unwrap();
    let TypedStatementKind::Let { ty, value, .. } = &program.functions[0].body[0].kind else {
        panic!("expected membership local");
    };
    assert_eq!(*ty, Type::Bool);
    assert_eq!(value.ty, ExprResult::Value(Type::Bool));
    let TypedExprKind::Binary { left, op, right } = &value.kind else {
        panic!("expected generic binary membership");
    };
    assert_eq!(*op, BinaryOp::In);
    assert_eq!(left.ty, ExprResult::Value(Type::Number));
    assert_eq!(right.ty, ExprResult::Value(Type::Range));
}

#[test]
fn membership_keyword_reserves_source_names_but_not_host_import_names() {
    for source in [
        "fn in() -> void {}",
        "fn main(in: number) -> void {}",
        "fn main() -> void { let in = 1; }",
    ] {
        let error = parse(source).unwrap_err();
        assert!(matches!(
            error,
            ParseError::UnexpectedToken { found: "in", .. }
        ));
    }

    let mut compiler = Compiler::new();
    assert_eq!(
        compiler.register_host_function("in", "env", "call", vec![], Type::Void),
        Err(HostRegistrationError::ReservedName { name: "in".into() })
    );
    compiler
        .register_host_function("inside", "in", "in", vec![], Type::Void)
        .unwrap();
    compiler.analyze("fn main() -> void { inside(); }").unwrap();
}

#[test]
fn membership_tokens_preserve_keyword_trivia_and_utf8_boundaries() {
    let tokens = Lexer::new("/*é🦀*/in/*x*/inside in1 _in In by \"in\" i/*x*/n").tokenize();
    assert_eq!(tokens[0].span, Span::new(10, 12));
    assert_eq!(
        tokens
            .iter()
            .map(|token| token.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TokenKind::In,
            TokenKind::Identifier("inside".into()),
            TokenKind::Identifier("in1".into()),
            TokenKind::Identifier("_in".into()),
            TokenKind::Identifier("In".into()),
            TokenKind::Identifier("by".into()),
            TokenKind::StringLiteral("in".into()),
            TokenKind::Identifier("i".into()),
            TokenKind::Identifier("n".into()),
            TokenKind::Eof,
        ]
    );
    assert_eq!(
        expression("1/*x*/in/*é*/0..3"),
        binary(
            number("1"),
            BinaryOp::In,
            range(number("0"), number("3"), None, false)
        )
    );
}

#[test]
fn membership_uses_comparison_precedence_and_left_associativity() {
    let membership = binary(
        binary(number("1"), BinaryOp::Add, number("2")),
        BinaryOp::In,
        range(
            number("0"),
            binary(number("3"), BinaryOp::Multiply, number("4")),
            Some(binary(number("5"), BinaryOp::Subtract, number("2"))),
            true,
        ),
    );
    assert_eq!(
        expression("1 + 2 in 0..=3 * 4 by 5 - 2 == true && false || true"),
        binary(
            binary(
                binary(membership, BinaryOp::Equal, Expr::Bool(true)),
                BinaryOp::And,
                Expr::Bool(false)
            ),
            BinaryOp::Or,
            Expr::Bool(true)
        )
    );
    assert_eq!(
        expression("1 < 2 in 0..3"),
        binary(
            binary(number("1"), BinaryOp::LessThan, number("2")),
            BinaryOp::In,
            range(number("0"), number("3"), None, false)
        )
    );
    assert_eq!(
        expression("1 in 0..3 < 4"),
        binary(
            binary(
                number("1"),
                BinaryOp::In,
                range(number("0"), number("3"), None, false)
            ),
            BinaryOp::LessThan,
            number("4")
        )
    );
    assert_eq!(
        expression("1 in 0..3 in 0..4"),
        binary(
            binary(
                number("1"),
                BinaryOp::In,
                range(number("0"), number("3"), None, false)
            ),
            BinaryOp::In,
            range(number("0"), number("4"), None, false)
        )
    );
}

#[test]
fn malformed_membership_reports_the_offending_token() {
    for (body, found) in [
        ("1 in;", ";"),
        ("in 0..3;", "in"),
        ("1 in in 0..3;", "in"),
        ("1 in 0..;", ";"),
        ("1 in 0..3 by;", ";"),
    ] {
        let source = format!("fn main() -> void {{ {body} }}");
        let error = parse(&source).unwrap_err();
        let ParseError::UnexpectedToken {
            found: actual,
            span,
            ..
        } = error
        else {
            panic!("expected unexpected token");
        };
        assert_eq!(actual, found, "{body}");
        assert_eq!(&source[span.start()..span.end()], found, "{body}");
    }
}

#[test]
fn membership_requires_exact_asymmetric_operand_types_even_in_skipped_code() {
    for (expr, left, right) in [
        ("true in 0..3", Type::Bool, Type::Range),
        ("\"1\" in 0..3", Type::String, Type::Range),
        ("[] in 0..3", Type::NumberArray, Type::Range),
        ("(0..1) in 0..3", Type::Range, Type::Range),
        ("noop() in 0..3", Type::Void, Type::Range),
        ("1 in 2", Type::Number, Type::Number),
        ("1 in true", Type::Number, Type::Bool),
        ("1 in \"range\"", Type::Number, Type::String),
        ("1 in []", Type::Number, Type::NumberArray),
        ("1 in noop()", Type::Number, Type::Void),
        ("(loop {}) in true", Type::Number, Type::Bool),
        ("true in (loop {})", Type::Bool, Type::Range),
        ("1 in 0..3 in 0..4", Type::Bool, Type::Range),
    ] {
        for body in [
            format!("{expr};"),
            format!("return; {expr};"),
            format!("false && ({expr});"),
            format!("if true {{}} else {{ {expr}; }}"),
        ] {
            let source = format!("fn main() -> void {{ {body} }} fn noop() -> void {{}}");
            assert!(
                matches!(semantic_error(&source), SemanticError::InvalidBinaryOperands {
                operator: "in", left: actual_left, right: actual_right, ..
            } if actual_left == left && actual_right == right),
                "{body}"
            );
        }
    }
    let source = "fn main() -> void { /*é🦀*/ 1 in true; }";
    assert_eq!(
        semantic_error(source),
        SemanticError::InvalidBinaryOperands {
            operator: "in",
            left: Type::Number,
            right: Type::Bool,
            span: Span::new(source.find("1 in").unwrap(), source.find(';').unwrap() + 1),
        }
    );
}

#[test]
fn membership_checks_both_children_and_preserves_first_error_order() {
    for (expr, name) in [
        ("missing in unknown", "missing"),
        ("(loop {}) in unknown", "unknown"),
        ("true in unknown", "unknown"),
        ("{ return; 1 } in unknown", "unknown"),
        ("1 in { loop {} (0..missing) }", "missing"),
    ] {
        for body in [
            format!("{expr};"),
            format!("return; {expr};"),
            format!("false && ({expr});"),
        ] {
            assert!(
                matches!(semantic_error(&format!("fn main() -> void {{ {body} }}")),
                SemanticError::UndefinedVariable { name: actual, .. } if actual == name),
                "{body}"
            );
        }
    }
}

#[test]
fn divergent_membership_operands_use_never_compatibility() {
    for expr in [
        "(loop {}) in 0..3",
        "1 in (loop {})",
        "(loop {}) in (loop {})",
    ] {
        let program = Compiler::new()
            .analyze(&format!(
                "fn main() -> number {{ let contains: bool = {expr}; }}"
            ))
            .unwrap();
        let TypedStatementKind::Let { ty, value, .. } = &program.functions[0].body[0].kind else {
            panic!("expected local");
        };
        assert_eq!(*ty, Type::Bool);
        assert_eq!(value.ty, ExprResult::Never);
        assert!(matches!(
            value.kind,
            TypedExprKind::Binary {
                op: BinaryOp::In,
                ..
            }
        ));
        assert!(
            matches!(semantic_error(&format!("fn main() -> void {{ let contains = {expr}; }}")),
            SemanticError::CannotInferNeverType { name, .. } if name == "contains")
        );
    }
    assert!(matches!(
        semantic_error("fn main() -> number { false && ((loop {}) in 0..3); }"),
        SemanticError::MissingReturn { .. }
    ));
}

#[test]
fn membership_flow_sequences_left_before_right_without_collecting_unreachable_breaks() {
    for (body, return_type, ty) in [
        (
            "{ break 7; 1 } in { break true; 0..3 };",
            "number",
            Type::Number,
        ),
        (
            "{ break true; 1 } in { break 7; 0..3 };",
            "bool",
            Type::Bool,
        ),
        ("1 in { break 7; 0..3 };", "number", Type::Number),
    ] {
        let source = format!(
            "fn main() -> {return_type} {{ let result = (loop {{ {body} }}); return result; }}"
        );
        let program = Compiler::new().analyze(&source).unwrap();
        let TypedStatementKind::Let { ty: actual, .. } = &program.functions[0].body[0].kind else {
            panic!("expected local");
        };
        assert_eq!(*actual, ty, "{body}");
    }
    let source =
        "fn main() -> number { let result: bool = (loop { (loop {}) in { break true; 0..3 }; }); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Let { value, .. } = &program.functions[0].body[0].kind else {
        panic!("expected local");
    };
    assert_eq!(value.ty, ExprResult::Never);
}

#[test]
fn membership_numeric_domains_and_internal_range_calls_are_not_static_errors() {
    for candidate in [
        "0.5",
        "0 / 0",
        "1 / 0",
        "9007199254740992",
        "-9007199254740992",
        "-0",
    ] {
        let source = format!("fn main() -> bool {{ return ({candidate}) in 0..=10 by 2; }}");
        Compiler::new().analyze(&source).unwrap();
    }
    Compiler::new().analyze(
        "fn main(x: number) -> bool { return x in bounds(); } fn bounds() -> range { return 10..0 by -2; }"
    ).unwrap();
}

#[test]
fn ranges_do_not_enable_other_range_operations() {
    assert!(parse("fn main() -> void { let r: range[] = 0..3; }").is_err());
    for body in [
        "let r = 0..3; len(r);",
        "let r = 0..3; r[0];",
        "let r = 0..3; r + r;",
        "let r = 0..3; r < r;",
        "let r = 0..3; !r;",
        "let r = 0..3; -r;",
    ] {
        let error = semantic_error(&format!("fn main() -> void {{ {body} }}"));
        assert!(
            matches!(
                error,
                SemanticError::TypeMismatch { .. } | SemanticError::InvalidBinaryOperands { .. }
            ),
            "{body}: {error:?}"
        );
    }
}
