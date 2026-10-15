use was_compiler::{
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::{ContractExpression, Expr, Program},
    host::{HostFunctions, HostRegistrationError},
    lexer::Lexer,
    parser::Parser,
    semantic::{SemanticAnalyzer, SemanticError},
    tokens::{Span, TokenKind},
    typed_ast::{ExprResult, FunctionId, TypedExpr, TypedExprKind, TypedStatementKind, VariableId},
};

fn parse(source: &str) -> Program {
    Parser::new(&Lexer::new(source).tokenize())
        .parse_program()
        .unwrap()
}

fn error(source: &str) -> SemanticError {
    let Err(CompileError::Semantic(error)) = Compiler::new().analyze(source) else {
        panic!("expected semantic error for {source}");
    };
    error
}

fn span(source: &str, fragment: &str) -> Span {
    let start = source.find(fragment).unwrap();
    Span::new(start, start + fragment.len())
}

fn identifier(value: &TypedExpr) -> VariableId {
    let TypedExprKind::Identifier { id, .. } = value.kind else {
        panic!("expected parameter read");
    };
    id
}

#[test]
fn ordered_clauses_preserve_expression_only_utf8_spans_and_parentheses() {
    let source = "/*é*/ fn f(s: string, n: number = 2) -> number\r\n\trequires /*λ*/ (s == \"🦀\") /* trailing */ requires (n > 0) // end\r\n { return n; }";
    let raw = parse(source);
    let typed = Compiler::new().analyze(source).unwrap();
    assert_eq!(raw.functions[0].contracts.len(), 2);
    assert_eq!(typed.functions[0].contracts.len(), 2);
    assert_eq!(
        raw.functions[0].span,
        span(source, &source["/*é*/ ".len()..])
    );
    for (index, text) in ["(s == \"🦀\")", "(n > 0)"].iter().enumerate() {
        assert_eq!(raw.functions[0].contracts[index].span, span(source, text));
        assert_eq!(typed.functions[0].contracts[index].span, span(source, text));
        assert_eq!(
            typed.functions[0].contracts[index].value.ty,
            ExprResult::Value(Type::Bool)
        );
    }
}

#[test]
fn contracts_parse_with_or_without_result_annotations_and_without_newlines() {
    for result in ["", " -> void"] {
        let source = format!("fn f(){result} requires true requires false {{}}");
        let raw = parse(&source);
        assert_eq!(
            raw.functions[0].return_type,
            if result.is_empty() {
                None
            } else {
                Some(Type::Void)
            }
        );
        let typed = Compiler::new().analyze(&source).unwrap();
        assert_eq!(typed.functions[0].return_type, Type::Void);
        assert_eq!(
            typed.functions[0].contracts[0].value.kind,
            TypedExprKind::Bool(true)
        );
        assert_eq!(
            typed.functions[0].contracts[1].value.kind,
            TypedExprKind::Bool(false)
        );
    }
    assert!(parse("fn f() -> void {}").functions[0].contracts.is_empty());
}

#[test]
fn requires_is_reserved_but_longer_identifiers_and_string_text_are_not() {
    let tokens = Lexer::new("requires requires_more \"requires\"").tokenize();
    assert_eq!(tokens[0].kind, TokenKind::Requires);
    assert!(matches!(&tokens[1].kind, TokenKind::Identifier(name) if name == "requires_more"));
    assert_eq!(tokens[2].kind, TokenKind::StringLiteral("requires".into()));
    for source in [
        "fn requires() {}",
        "fn f(requires: bool) {}",
        "fn f() { let requires = true; }",
        "fn f() { requires; }",
    ] {
        assert!(
            matches!(Compiler::new().analyze(source), Err(CompileError::Parse(_))),
            "{source}"
        );
    }
    Compiler::new()
        .analyze("fn requires_more() -> string { return \"requires\"; }")
        .unwrap();
}

#[test]
fn requires_reserves_only_host_language_names_and_failed_registration_is_atomic() {
    let mut hosts = HostFunctions::new();
    assert_eq!(
        hosts.register("requires", "env", "call", vec![], Type::Void),
        Err(HostRegistrationError::ReservedName {
            name: "requires".into()
        })
    );
    let id = hosts
        .register("allowed", "requires", "requires", vec![], Type::Void)
        .unwrap();
    assert_eq!(id.0, 0);
    assert_eq!(hosts.functions()[0].module, "requires");
    assert_eq!(hosts.functions()[0].field, "requires");
}

#[test]
fn malformed_clauses_remain_parser_errors() {
    for source in [
        "fn f() requires",
        "fn f() requires requires true {}",
        "fn f() requires true; {}",
        "fn f() requires true -> bool {}",
        "fn f() requires (true {}",
        "fn f() requires true",
        "fn f() requires x = true {}",
        "fn f() requires x += 1 {}",
        "fn f() requires true, false {}",
        "requires true fn f() {}",
        "fn f() {} requires true",
        "fn f() requires /* unfinished",
    ] {
        assert!(
            matches!(Compiler::new().analyze(source), Err(CompileError::Parse(_))),
            "{source}"
        );
    }
}

#[test]
fn truncated_contract_syntax_never_panics() {
    let source = "fn f(a: number = 1) -> number requires (a > 0) requires true { return a; }";
    for end in 0..=source.len() {
        let tokens = Lexer::new(&source[..end]).tokenize();
        let _ = Parser::new(&tokens).parse_program();
    }
}

#[test]
fn whitelist_accepts_all_existing_scalar_operators_and_handle_equality() {
    for condition in [
        "true",
        "false",
        "flag",
        "!flag",
        "-a < b",
        "a + b > 0",
        "a - b <= 0",
        "a * b >= 0",
        "a / b == 1",
        "a % b != 0",
        "a > b",
        "a == b",
        "a != b",
        "flag == true",
        "flag != false",
        "flag && true",
        "flag || false",
        "s == t",
        "s != \"é\"",
        "xs == ys",
        "xs != ys",
        "len(xs) >= 0",
        "(len(xs) + len(ys)) == a",
        "(flag || false) && !false",
    ] {
        let source = format!(
            "fn f(a: number, b: number, flag: bool, s: string, t: string, xs: number[], ys: number[]) requires {condition} {{}}"
        );
        let typed = Compiler::new().analyze(&source).unwrap();
        assert_eq!(
            typed.functions[0].contracts[0].value.ty,
            ExprResult::Value(Type::Bool),
            "{condition}"
        );
    }
}

#[test]
fn all_formals_are_available_and_contract_ids_match_actual_callee_parameters() {
    let source = "fn f(first: number = 1, last: number = first + 1) requires last > first { let copy = last; return copy; }
        fn main() { let last = 100; return f(last: 4); }";
    let typed = Compiler::new().analyze(source).unwrap();
    let function = &typed.functions[0];
    let TypedExprKind::Binary { left, right, .. } = &function.contracts[0].value.kind else {
        panic!("comparison")
    };
    assert_eq!(identifier(left), function.params[1].id);
    assert_eq!(identifier(right), function.params[0].id);
    let TypedStatementKind::Let { id, .. } = &function.body[0].kind else {
        panic!("let")
    };
    assert_eq!(*id, VariableId(2));
    assert_eq!(function.return_type, Type::Number);
    assert_eq!(typed.functions[1].return_type, Type::Number);
}

#[test]
fn contract_scope_does_not_include_body_caller_or_other_function_parameters() {
    for source in [
        "fn f() requires local > 0 { let local = 1; }",
        "fn caller(local: number) { f(); } fn f() requires local > 0 {}",
        "fn first(local: number) {} fn second() requires local > 0 {}",
    ] {
        assert!(
            matches!(error(source), SemanticError::UndefinedVariable { name, .. } if name == "local")
        );
    }
}

#[test]
fn lexical_parameter_shadowing_of_builtin_len_is_preserved() {
    let source = "fn f(len: bool, xs: number[]) requires len(xs) > 0 {}";
    assert_eq!(
        error(source),
        SemanticError::NotCallable {
            name: "len".into(),
            span: span(source, "len(xs) > 0")
        }
    );
    Compiler::new()
        .analyze("fn f(len: bool) requires len {}")
        .unwrap();
}

#[test]
fn rejects_every_nonwhitelisted_construct_even_in_skipped_operands() {
    for (condition, reason) in [
        ("[1] == xs", "array construction is not allowed"),
        ("1..2", "range construction is not allowed"),
        ("1 in r", "range membership is not allowed"),
        ("xs[0] > 0", "indexing is not allowed"),
        ("{ true }", "blocks are not allowed"),
        (
            "if true { true } else { false }",
            "if expressions are not allowed",
        ),
        ("loop { break true; }", "loop expressions are not allowed"),
        ("other()", "only positional builtin len calls are allowed"),
        (
            "other(value: true)",
            "only positional builtin len calls are allowed",
        ),
        (
            "len(value: xs) > 0",
            "only positional builtin len calls are allowed",
        ),
        ("len([1]) > 0", "array construction is not allowed"),
    ] {
        for prefix in ["", "false && ", "true || "] {
            let expression = format!("{prefix}({condition})");
            let source = format!(
                "fn f(xs: number[], r: range) requires {expression} {{}} fn other(value: bool = true) -> bool {{ return value; }}"
            );
            assert_eq!(
                error(&source),
                SemanticError::InvalidContractExpression {
                    reason,
                    span: span(&source, &expression)
                },
                "{source}"
            );
        }
    }
}

#[test]
fn host_calls_are_rejected_without_argument_typing_even_when_skipped() {
    let mut hosts = HostFunctions::new();
    hosts
        .register("check", "env", "check", vec![], Type::Bool)
        .unwrap();
    for condition in [
        "check()",
        "false && check(missing)",
        "true || check(value: missing)",
    ] {
        let source = format!("fn f() requires {condition} {{}}");
        let result = SemanticAnalyzer::with_host_functions(&hosts).analyze(&parse(&source));
        assert_eq!(
            result,
            Err(SemanticError::InvalidContractExpression {
                reason: "only positional builtin len calls are allowed",
                span: span(&source, condition)
            })
        );
    }
}

#[test]
fn contract_results_require_exact_bool_without_coercions() {
    for (condition, found) in [
        ("1", Type::Number),
        ("\"text\"", Type::String),
        ("xs", Type::NumberArray),
        ("r", Type::Range),
        ("len(xs)", Type::Number),
    ] {
        let source = format!("fn f(xs: number[], r: range) requires {condition} {{}}");
        assert_eq!(
            error(&source),
            SemanticError::TypeMismatch {
                expected: Type::Bool,
                found,
                span: {
                    let start = source.find("requires ").unwrap() + "requires ".len();
                    Span::new(start, start + condition.len())
                }
            }
        );
    }
}

#[test]
fn permitted_constructs_still_receive_existing_operand_and_arity_checks() {
    for condition in ["true + 1 == 2", "xs == 1", "r == r", "s < s", "false && 1"] {
        let source = format!("fn f(xs: number[], r: range, s: string) requires {condition} {{}}");
        let error = error(&source);
        assert!(
            matches!(error, SemanticError::InvalidBinaryOperands { .. }),
            "{source}: {error:?}"
        );
        assert_eq!(error.span(), Some(span(&source, condition)));
    }
    for (condition, found) in [("len() > 0", 0), ("len(xs, xs) > 0", 2)] {
        let source = format!("fn f(xs: number[]) requires {condition} {{}}");
        assert_eq!(
            error(&source),
            SemanticError::ArgumentCountMismatch {
                name: "len".into(),
                expected: 1,
                found,
                span: span(&source, condition)
            }
        );
    }
    let source = "fn f() requires len(1) > 0 {}";
    assert_eq!(
        error(source),
        SemanticError::TypeMismatch {
            expected: Type::NumberArray,
            found: Type::Number,
            span: span(source, "len(1) > 0")
        }
    );
}

#[test]
fn unused_unexported_contracts_are_checked_before_any_body() {
    for use_site in [
        "",
        "hidden();",
        "return; hidden();",
        "false && { hidden(); true };",
    ] {
        let source = format!(
            "fn main() -> void {{ {use_site} }} fn hidden() -> void requires missing_first {{}} fn later() -> void requires missing_second {{}}"
        );
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        assert_eq!(
            compiler.analyze(&source),
            Err(CompileError::Semantic(SemanticError::UndefinedVariable {
                name: "missing_first".into(),
                span: span(&source, "missing_first")
            }))
        );
    }
    let source =
        "fn main() -> void { unknown_body; } fn hidden() -> void requires unknown_contract {}";
    assert_eq!(
        error(source),
        SemanticError::UndefinedVariable {
            name: "unknown_contract".into(),
            span: span(source, "unknown_contract")
        }
    );
}

#[test]
fn parameter_types_and_defaults_precede_same_function_contracts() {
    let source = "fn f(value: void) requires missing_contract {}";
    assert_eq!(
        error(source),
        SemanticError::InvalidVoidType {
            name: "value".into(),
            span: span(source, "value: void")
        }
    );
    let source = "fn f(value: bool = missing_default) requires missing_contract {}";
    assert_eq!(
        error(source),
        SemanticError::UndefinedVariable {
            name: "missing_default".into(),
            span: span(source, "missing_default")
        }
    );
    let source = "fn f(a: bool = true, b: bool) requires missing_contract {}";
    assert!(matches!(error(source), SemanticError::NonTrailingDefault { name, .. } if name == "b"));
    let source = "fn f(a: bool, a: bool) requires missing_contract {}";
    assert!(matches!(error(source), SemanticError::DuplicateVariable { name, .. } if name == "a"));
}

#[test]
fn declarations_stage_per_function_not_as_separate_global_default_and_contract_passes() {
    for later in [
        "fn later(a: void) {}",
        "fn later(a: bool = missing_default) {}",
        "fn first() {}",
    ] {
        let source = format!("fn first() requires missing_contract {{}} {later}");
        assert_eq!(
            error(&source),
            SemanticError::UndefinedVariable {
                name: "missing_contract".into(),
                span: span(&source, "missing_contract")
            }
        );
    }
    let source = "fn first() requires true requires missing_second requires missing_third {}";
    assert_eq!(
        error(source),
        SemanticError::UndefinedVariable {
            name: "missing_second".into(),
            span: span(source, "missing_second")
        }
    );
}

#[test]
fn invalid_contracts_precede_recursion_and_inference_errors() {
    for body in [
        "return f();",
        "return if true { 1 } else { false };",
        "return loop {};",
        "missing_body;",
    ] {
        let source = format!("fn f() requires missing_contract {{ {body} }}");
        assert_eq!(
            error(&source),
            SemanticError::UndefinedVariable {
                name: "missing_contract".into(),
                span: span(&source, "missing_contract")
            }
        );
    }
    let source = "fn recursive() { return recursive(); } fn later() requires missing_contract {}";
    assert_eq!(
        error(source),
        SemanticError::UndefinedVariable {
            name: "missing_contract".into(),
            span: span(source, "missing_contract")
        }
    );
}

#[test]
fn false_contract_does_not_change_completion_or_excuse_invalid_body() {
    assert!(matches!(
        error("fn f() -> number requires false {}"),
        SemanticError::MissingReturn { .. }
    ));
    assert!(matches!(
        error("fn f() requires false { missing; }"),
        SemanticError::UndefinedVariable { .. }
    ));
    assert!(matches!(
        error("fn f() requires false { return f(); }"),
        SemanticError::RecursiveResultAnnotationRequired { .. }
    ));
    assert!(matches!(
        error("fn f() requires false { return loop {}; }"),
        SemanticError::CannotInferFunctionResult { .. }
    ));
    assert!(matches!(
        error("fn f() -> bool requires false { return 1; }"),
        SemanticError::TypeMismatch { .. }
    ));
    assert_eq!(
        Compiler::new()
            .analyze("fn f() requires false {}")
            .unwrap()
            .functions[0]
            .return_type,
        Type::Void
    );
}

#[test]
fn inferred_forward_diamonds_keep_source_order_and_contracts_in_final_output() {
    let source = "fn main() requires false { return left() + right(); }
        fn right() requires true { return leaf(); }
        fn left() requires true { return leaf(); }
        fn leaf() requires true { return 4; }";
    let typed = Compiler::new().analyze(source).unwrap();
    for (index, function) in typed.functions.iter().enumerate() {
        assert_eq!(function.id, FunctionId(index));
        assert_eq!(function.return_type, Type::Number);
        assert_eq!(function.contracts.len(), 1);
        assert_eq!(
            function.contracts[0].value.ty,
            ExprResult::Value(Type::Bool)
        );
    }
    assert_eq!(
        typed.functions[0].contracts[0].value.kind,
        TypedExprKind::Bool(false)
    );
}

#[test]
fn defaults_keep_earlier_formal_scope_and_their_original_diagnostics() {
    let source = "fn f(a: number = b, b: number = 1) requires b >= a {}";
    assert_eq!(
        error(source),
        SemanticError::UndefinedVariable {
            name: "b".into(),
            span: {
                let start = source.find("b, b").unwrap();
                Span::new(start, start + 1)
            }
        }
    );
}

#[test]
fn defaults_and_contracts_share_whitelist_but_not_error_variants() {
    let source = "fn f(a: bool = { true }) requires true {}";
    assert_eq!(
        error(source),
        SemanticError::InvalidDefaultExpression {
            reason: "blocks are not allowed",
            span: span(source, "{ true }")
        }
    );
    let source = "fn f(a: bool = true) requires { true } {}";
    assert_eq!(
        error(source),
        SemanticError::InvalidContractExpression {
            reason: "blocks are not allowed",
            span: span(source, "{ true }")
        }
    );
}

#[test]
fn invalid_and_nonfinite_raw_contract_literals_use_declaration_semantic_errors() {
    for literal in ["NaN", "inf", "1e2", ".5", "1.", "", "-1", "1_000"] {
        let mut raw = parse("fn f() requires true {}");
        let origin = Span::new(91, 102);
        raw.functions[0].contracts[0] = ContractExpression {
            value: Expr::Number(literal.into()),
            span: origin,
        };
        assert_eq!(
            SemanticAnalyzer::new().analyze(&raw),
            Err(SemanticError::InvalidContractNumberLiteral {
                literal: literal.into(),
                span: origin
            })
        );
    }
    let literal = "9".repeat(400);
    let source = format!("fn main() {{}} fn hidden() requires false && ({literal} > 0) {{}}");
    assert_eq!(
        error(&source),
        SemanticError::ContractNumberLiteralOutOfRange {
            literal: literal.clone(),
            span: span(&source, &format!("false && ({literal} > 0)"))
        }
    );
}

#[test]
fn underflow_and_nonfinite_arithmetic_are_not_constant_folded_or_rejected() {
    let tiny = format!("0.{}1", "0".repeat(400));
    for condition in [
        format!("{tiny} == 0"),
        "1 / 0 > 0".into(),
        "0 / 0 == 0".into(),
        "1 % 0 == 0".into(),
        "-0 == 0".into(),
    ] {
        let source = format!("fn f() requires {condition} {{}}");
        Compiler::new().analyze(&source).unwrap();
    }
}

#[test]
fn numeric_errors_preserve_default_and_ordinary_body_staging() {
    let literal = "9".repeat(400);
    let source = format!("fn f(a: number = {literal}) requires false {{}}");
    assert_eq!(
        error(&source),
        SemanticError::DefaultNumberLiteralOutOfRange {
            literal: literal.clone(),
            span: span(&source, &literal)
        }
    );
    Compiler::new()
        .analyze(&format!("fn f() requires true {{ {literal}; }}"))
        .unwrap();
}

#[test]
fn contract_metadata_is_owned_and_analyzer_recovers_after_declaration_errors() {
    let mut analyzer = SemanticAnalyzer::new();
    assert!(
        analyzer
            .analyze(&parse("fn bad(x: number) requires missing {}"))
            .is_err()
    );
    let typed = {
        let source = "fn f(s: string) requires s == \"é\" { return s; }".to_owned();
        analyzer.analyze(&parse(&source)).unwrap()
    };
    let TypedExprKind::Binary { left, right, .. } = &typed.functions[0].contracts[0].value.kind
    else {
        panic!("equality")
    };
    assert_eq!(identifier(left), typed.functions[0].params[0].id);
    assert_eq!(right.kind, TypedExprKind::String("é".into()));
    assert_eq!(
        analyzer
            .analyze(&parse("fn clean() {} "))
            .unwrap()
            .functions[0]
            .contracts,
        vec![]
    );
}

#[test]
fn optimization_options_do_not_change_analysis_contracts_or_diagnostics() {
    let source = "fn f(n: number = 1) requires n > 0 { return n; }";
    let expected = Compiler::new().analyze(source).unwrap();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        assert_eq!(compiler.analyze(source).unwrap(), expected);
        let invalid = "fn f() requires 1 {}";
        assert_eq!(
            compiler.analyze(invalid),
            Err(CompileError::Semantic(error(invalid)))
        );
    }
}
