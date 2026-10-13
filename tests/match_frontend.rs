use was_compiler::{
    CompileError, Compiler, Type,
    ast::{BinaryOp, Expr, MatchPatternKind, Program, StatementKind},
    host::{HostFunctions, HostRegistrationError},
    lexer::Lexer,
    parser::Parser,
    semantic::{SemanticAnalyzer, SemanticError},
    tokens::{Span, TokenKind},
    typed_ast::{ExprResult, TypedExprKind, TypedStatementKind},
};

fn parse(source: &str) -> Program {
    Parser::new(&Lexer::new(source).tokenize())
        .parse_program()
        .unwrap()
}

fn error(source: &str) -> SemanticError {
    let Err(CompileError::Semantic(error)) = Compiler::new().analyze(source) else {
        panic!("expected semantic error: {source}");
    };
    error
}

fn span(source: &str, text: &str) -> Span {
    let start = source.find(text).unwrap();
    Span::new(start, start + text.len())
}

#[test]
fn patterns_and_arms_preserve_utf8_byte_spans_and_signed_spellings() {
    let source = "/*é*/ fn f(n: number) -> number { return match n { - /*λ*/ 0 => 1, < -2.5 => { 2 }, >= 3 => 3, -1.5..=2.5 => 4, 4..4 => 5, _ => 6, }; }";
    let raw = parse(source);
    let StatementKind::Return(Some(Expr::Match { arms, .. })) = &raw.functions[0].body[0].kind
    else {
        panic!()
    };
    assert_eq!(arms.len(), 6);
    assert_eq!(arms[0].pattern.kind, MatchPatternKind::Number("-0".into()));
    assert_eq!(
        arms[1].pattern.kind,
        MatchPatternKind::Relational {
            op: BinaryOp::LessThan,
            value: "-2.5".into()
        }
    );
    assert_eq!(
        arms[3].pattern.kind,
        MatchPatternKind::Interval {
            start: "-1.5".into(),
            end: "2.5".into(),
            inclusive: true
        }
    );
    let typed = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &typed.functions[0].body[0].kind else {
        panic!()
    };
    let TypedExprKind::Match {
        arms: typed_arms, ..
    } = &value.kind
    else {
        panic!()
    };
    for (index, (pattern, arm)) in [
        ("- /*λ*/ 0", "- /*λ*/ 0 => 1"),
        ("< -2.5", "< -2.5 => { 2 }"),
        (">= 3", ">= 3 => 3"),
        ("-1.5..=2.5", "-1.5..=2.5 => 4"),
        ("4..4", "4..4 => 5"),
        ("_", "_ => 6"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(arms[index].pattern.span, span(source, pattern));
        assert_eq!(arms[index].span, span(source, arm));
        assert_eq!(typed_arms[index].pattern, arms[index].pattern);
        assert_eq!(typed_arms[index].span, arms[index].span);
    }
}

#[test]
fn all_pattern_domains_coverage_and_duplicates_are_accepted() {
    for (ty, arms) in [
        (
            "number",
            "< -2 => 0, <= 0 => 1, > 1 => 2, >= 2 => 3, -0 => 4, 0 => 5, 3..1 => 6, 2..2 => 7, 0.5..=2.5 => 8, _ => 9",
        ),
        ("string", "\"🦀\" => 1, \"🦀\" => 2, _ => 3"),
        ("bool", "true => 1, false => 2, true => 3"),
        ("bool", "false => 1, _ => 2"),
        ("bool", "_ => 1"),
    ] {
        Compiler::new()
            .analyze(&format!(
                "fn f(x: {ty}) -> number {{ return match x {{ {arms} }}; }}"
            ))
            .unwrap();
    }
}

#[test]
fn statement_matches_discard_heterogeneous_results_in_every_bare_context() {
    let source = "fn f() -> void { match true { true => 1, false => \"s\" }; match false { _ => [] } -1; let n = { match true { true => true, false => 2 }; match false { _ => \"s\" } 4 }; if true { match true { true => 1, false => false } } }";
    let raw = parse(source);
    assert!(matches!(
        raw.functions[0].body[0].kind,
        StatementKind::Match { .. }
    ));
    assert!(matches!(
        raw.functions[0].body[1].kind,
        StatementKind::Match { .. }
    ));
    assert!(matches!(
        raw.functions[0].body[2].kind,
        StatementKind::Expression(Expr::Unary { .. })
    ));
    Compiler::new().analyze(source).unwrap();
}

#[test]
fn final_matches_are_value_tails_and_parentheses_allow_continuation() {
    for source in [
        "fn f() -> number { return { match true { true => 1, false => 2 } }; }",
        "fn f() -> number { return match true { _ => 2 } + 3; }",
        "fn f() -> number { return { (match true { _ => 2 }) + 3 }; }",
        "fn f() -> void { (match true { _ => 2 }) + 3; }",
    ] {
        Compiler::new().analyze(source).unwrap();
    }
    assert!(matches!(
        error("fn f() -> number { return { match true { true => 1, false => false } }; }"),
        SemanticError::TypeMismatch { .. }
    ));
    assert!(matches!(
        error("fn f() -> void { (match true { true => 1, false => false }); }"),
        SemanticError::TypeMismatch { .. }
    ));
}

#[test]
fn value_results_join_all_concrete_categories_void_and_never() {
    for (ty, first, second) in [
        ("number", "1", "2"),
        ("bool", "true", "false"),
        ("string", "\"a\"", "\"b\""),
        ("number[]", "[1]", "[]"),
        ("range", "0..2", "1..=3"),
    ] {
        Compiler::new().analyze(&format!("fn f(b: bool) -> {ty} {{ return match b {{ true => {first}, false => {second} }}; }}")).unwrap();
        Compiler::new().analyze(&format!("fn f(b: bool) -> {ty} {{ return match b {{ true => loop {{}}, false => {first} }}; }}")).unwrap();
    }
    let typed = Compiler::new()
        .analyze("fn f() -> void { (match true { true => {}, false => {} }); }")
        .unwrap();
    let TypedStatementKind::Expression(value) = &typed.functions[0].body[0].kind else {
        panic!()
    };
    assert_eq!(value.ty, ExprResult::Void);
    let typed = Compiler::new()
        .analyze("fn f() -> number { return match true { true => loop {}, false => loop {} }; }")
        .unwrap();
    let TypedStatementKind::Return(Some(value)) = &typed.functions[0].body[0].kind else {
        panic!()
    };
    assert_eq!(value.ty, ExprResult::Never);
}

#[test]
fn mismatched_arm_results_use_offending_arm_span() {
    let source = "fn f() -> number { return match true { true => 1, false => \"é\" }; }";
    let error = error(source);
    assert_eq!(
        error,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::String,
            span: span(source, "false => \"é\"")
        }
    );
    assert_eq!(error.span(), Some(span(source, "false => \"é\"")));
    assert!(matches!(
        error_for_void_join(),
        SemanticError::TypeMismatch { .. }
    ));
}

fn error_for_void_join() -> SemanticError {
    error("fn f() -> void { (match true { true => {}, false => 1 }); }")
}

#[test]
fn coverage_and_scrutinee_errors_use_enclosing_location() {
    for expression in [
        "match 1 { 1 => 2 }",
        "match \"x\" { \"x\" => 2 }",
        "match true { true => 2 }",
        "match true {}",
        "match [] { _ => 2 }",
        "match 0..1 { _ => 2 }",
        "match {} { _ => 2 }",
    ] {
        let source = format!("fn f() -> number {{ return {expression}; }}");
        let error = error(&source);
        assert!(
            matches!(error, SemanticError::InvalidMatch { .. }),
            "{error:?}"
        );
        assert_eq!(
            error.span(),
            Some(span(&source, &format!("return {expression};")))
        );
        assert!(error.to_string().starts_with("Invalid match:"));
    }
}

#[test]
fn pattern_domain_and_wildcard_errors_use_pattern_location() {
    for (expression, offending) in [
        ("match 1 { true => 2, _ => 3 }", "true"),
        ("match true { 1 => 2, _ => 3 }", "1"),
        ("match \"x\" { < 1 => 2, _ => 3 }", "< 1"),
        ("match true { _ => 1, false => 2 }", "false"),
    ] {
        let source = format!("fn f() -> number {{ return {expression}; }}");
        assert_eq!(error(&source).span(), Some(span(&source, offending)));
    }
    assert!(matches!(
        error("fn f() -> void { match true { _ => 1, _ => 2 } }"),
        SemanticError::InvalidMatch { .. }
    ));
}

#[test]
fn never_scrutinees_still_check_domains_coverage_results_and_bodies() {
    for source in [
        "fn f() -> number { return match (loop {}) { 1 => 2 }; }",
        "fn f() -> number { return match (loop {}) { 1 => 2, true => 2, _ => 2 }; }",
        "fn f() -> number { return match (loop {}) { true => 1, false => false }; }",
    ] {
        assert!(Compiler::new().analyze(source).is_err(), "{source}");
    }
    assert!(matches!(
        error("fn f() -> number { return match (loop {}) { _ => missing }; }"),
        SemanticError::UndefinedVariable { .. }
    ));
    for arms in [
        "_ => 1",
        "true => 1, false => 2",
        "\"s\" => 1, _ => 2",
        "1 => 1, _ => 2",
    ] {
        let typed = Compiler::new()
            .analyze(&format!(
                "fn f() -> number {{ return match (loop {{}}) {{ {arms} }}; }}"
            ))
            .unwrap();
        let TypedStatementKind::Return(Some(value)) = &typed.functions[0].body[0].kind else {
            panic!()
        };
        assert_eq!(value.ty, ExprResult::Never);
    }
    Compiler::new()
        .analyze("fn f() -> number { match (loop {}) { true => 1, false => false } }")
        .unwrap();
}

#[test]
fn arms_are_scoped_and_all_arms_are_checked_even_after_coverage_or_transfer() {
    Compiler::new().analyze("fn f() -> number { let n = 4; return match true { true => { let n = 1; n }, false => { let n = 2; n } }; }").unwrap();
    for source in [
        "fn f() -> number { return match true { true => { let n = 1; n }, false => n }; }",
        "fn f() -> void { match true { _ => { let n = 1; n } } n; }",
        "fn f() -> number { return match true { true => 1, false => 2, true => missing }; }",
        "fn f() -> void { return; match true { _ => missing } }",
        "fn f() -> void { false && (match true { _ => missing }); }",
    ] {
        assert!(matches!(
            error(source),
            SemanticError::UndefinedVariable { .. }
        ));
    }
}

#[test]
fn inference_discovers_all_arm_calls_returns_and_lexical_shadowing() {
    let typed = Compiler::new().analyze("fn f(b: bool) { match b { true => { return left(); }, false => { return right(); }, true => { return last(); } } } fn left() { return 1; } fn right() { return left(); } fn last() { return right(); }").unwrap();
    assert!(
        typed
            .functions
            .iter()
            .all(|f| f.return_type == Type::Number)
    );
    assert!(matches!(
        error("fn f() { match true { true => { return 1; }, false => { return false; } } }"),
        SemanticError::IncompatibleInferredReturns { .. }
    ));
    assert!(matches!(
        error("fn f() { return match true { true => 1, false => 2, true => f() }; }"),
        SemanticError::RecursiveResultAnnotationRequired { .. }
    ));
    assert!(matches!(
        error("fn f() { match (loop {}) { _ => { return 1; } } }"),
        SemanticError::CannotInferFunctionResult { .. }
    ));
    assert!(matches!(
        error("fn f() { match true { true => { let f = 1; f(); }, false => {} } }"),
        SemanticError::NotCallable { .. }
    ));
}

#[test]
fn every_arm_contributes_conservative_return_and_loop_transfer_flow() {
    Compiler::new()
        .analyze(
            "fn f(b: bool) -> number { match b { true => { return 1; }, false => { return 2; } } }",
        )
        .unwrap();
    assert!(matches!(
        error("fn f() -> number { match true { true => { return 1; }, false => 2 } }"),
        SemanticError::MissingReturn { .. }
    ));
    Compiler::new().analyze("fn f(b: bool) -> number { return loop { match b { true => { break 1; }, false => { continue; } } }; }").unwrap();
    assert!(matches!(
        error(
            "fn f() -> number { return loop { match true { true => { break 1; }, false => { break false; } } }; }"
        ),
        SemanticError::TypeMismatch { .. }
    ));
    Compiler::new().analyze("fn f() -> void { while true { match true { true => { break; }, false => { continue; } } } }").unwrap();
    assert!(matches!(
        error("fn f() -> void { match true { _ => { break; } } }"),
        SemanticError::BreakOutsideLoop { .. }
    ));
}

#[test]
fn match_is_excluded_from_default_and_contract_whitelists_even_when_skipped() {
    for source in [
        "fn f(n: number = match 1 { _ => 2 }) -> void {}",
        "fn f(b: bool = false && (match true { _ => true })) -> void {}",
    ] {
        assert!(matches!(
            error(source),
            SemanticError::InvalidDefaultExpression {
                reason: "match expressions are not allowed",
                ..
            }
        ));
    }
    for source in [
        "fn f() -> void requires match true { _ => true } {}",
        "fn f() -> void requires true || (match true { _ => true }) {}",
    ] {
        assert!(matches!(
            error(source),
            SemanticError::InvalidContractExpression {
                reason: "match expressions are not allowed",
                ..
            }
        ));
    }
    Compiler::new().analyze("fn f(n: number = 2) { return match n { _ => g(value: n) }; } fn g(value: number) { return value; }").unwrap();
}

#[test]
fn malformed_match_syntax_is_rejected_and_truncations_do_not_panic() {
    for arms in [
        "_ -> 1",
        "_ = > 1",
        "_ => return 1;",
        "_ => break;",
        "_ => continue;",
        "_ => 1 _ => 2",
        "_ => {} _ => {}",
        ",",
        "name => 1",
        "(1) => 1",
        "+1 => 1",
        "--1 => 1",
        "1 + 2 => 1",
        "call() => 1",
        "1..2 by 1 => 1",
        "1..2..3 => 1",
        "< true => 1",
        "== 1 => 1",
        "1e2 => 1",
        ".5 => 1",
        "1. => 1",
    ] {
        let source = format!("fn f() -> void {{ match 1 {{ {arms} }} }}");
        assert!(
            Parser::new(&Lexer::new(&source).tokenize())
                .parse_program()
                .is_err(),
            "{source}"
        );
    }
    let source = "fn f() -> number { return match 1 { -2.5..=3 => { 1 }, _ => 2 }; }";
    for end in 0..source.len() {
        assert!(
            Parser::new(&Lexer::new(&source[..end]).tokenize())
                .parse_program()
                .is_err(),
            "prefix {end}"
        );
    }
    assert!(
        Parser::new(&Lexer::new("fn f() -> void { match true { _ => 1 } + 2; }").tokenize())
            .parse_program()
            .is_err()
    );
}

#[test]
fn match_and_wildcard_are_reserved_only_as_exact_source_names() {
    let tokens = Lexer::new("match _ _temp match_more => = >").tokenize();
    assert_eq!(tokens[0].kind, TokenKind::Match);
    assert_eq!(tokens[1].kind, TokenKind::Wildcard);
    assert!(matches!(&tokens[2].kind, TokenKind::Identifier(name) if name == "_temp"));
    assert_eq!(tokens[4].kind, TokenKind::FatArrow);
    assert_eq!(tokens[5].kind, TokenKind::Equal);
    assert_eq!(tokens[6].kind, TokenKind::Greater);
    for name in ["match", "_"] {
        for source in [
            format!("fn {name}() {{}}"),
            format!("fn f({name}: number) {{}}"),
            format!("fn f() {{ let {name} = 1; }}"),
            format!("fn f() {{ for {name} in [] {{}} }}"),
            format!("fn f() {{ {name}; }}"),
        ] {
            assert!(
                Parser::new(&Lexer::new(&source).tokenize())
                    .parse_program()
                    .is_err(),
                "{source}"
            );
        }
        let mut host = HostFunctions::new();
        assert_eq!(
            host.register(name, "env", "run", vec![], Type::Void),
            Err(HostRegistrationError::ReservedName { name: name.into() })
        );
        host.register("_temp", name, name, vec![], Type::Void)
            .unwrap();
    }
    Compiler::new()
        .analyze("fn _temp(match_more: number) { let _value = match_more; return _value; }")
        .unwrap();
}

#[test]
fn forged_raw_patterns_validate_literals_operators_and_domains() {
    let source = "fn f() -> number { return match 1 { 1 => 2, _ => 3 }; }";
    for kind in [
        MatchPatternKind::Number("+1".into()),
        MatchPatternKind::Number("1e2".into()),
        MatchPatternKind::Number("--1".into()),
        MatchPatternKind::Number("NaN".into()),
        MatchPatternKind::Number("9".repeat(400)),
        MatchPatternKind::Relational {
            op: BinaryOp::Equal,
            value: "1".into(),
        },
        MatchPatternKind::Interval {
            start: "1".into(),
            end: "inf".into(),
            inclusive: true,
        },
        MatchPatternKind::Bool(true),
    ] {
        let mut program = parse(source);
        let StatementKind::Return(Some(Expr::Match { arms, .. })) =
            &mut program.functions[0].body[0].kind
        else {
            panic!()
        };
        arms[0].pattern.kind = kind;
        let expected_span = arms[0].pattern.span;
        let error = SemanticAnalyzer::new().analyze(&program).unwrap_err();
        assert!(matches!(error, SemanticError::InvalidMatch { .. }));
        assert_eq!(error.span(), Some(expected_span));
    }
}

#[test]
fn source_pattern_overflow_is_static_but_underflow_and_large_fractions_are_valid() {
    let huge = "9".repeat(400);
    let source = format!("fn f() -> number {{ return match 1 {{ -{huge} => 1, _ => 2 }}; }}");
    assert_eq!(
        error(&source).span(),
        Some(span(&source, &format!("-{huge}")))
    );
    let tiny = format!("0.{}1", "0".repeat(400));
    Compiler::new().analyze(&format!("fn f() -> number {{ return match 1 {{ -{tiny} => 1, 9007199254740992.5..=9007199254740994.5 => 2, _ => 3 }}; }}")).unwrap();
}
