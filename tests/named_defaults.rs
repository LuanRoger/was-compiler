use was_compiler::{
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::{DefaultExpression, Expr},
    codegen::CodegenError,
    lexer::Lexer,
    parser::Parser,
    semantic::{SemanticAnalyzer, SemanticError},
    tokens::Span,
    typed_ast::{
        CallBindingOrigin, ExprResult, FunctionRef, TypedExpr, TypedExprKind, TypedNormalizedCall,
        TypedStatementKind, VariableId,
    },
};

fn semantic_error(source: &str) -> SemanticError {
    let Err(CompileError::Semantic(error)) = Compiler::new().analyze(source) else {
        panic!("expected semantic error for {source}")
    };

    error
}

fn source_span(source: &str, text: &str) -> Span {
    let start = source.find(text).expect("source fragment");

    Span::new(start, start + text.len())
}

fn normalized(value: &TypedExpr) -> &TypedNormalizedCall {
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized call")
    };

    sequence
}

fn identifier_id(value: &TypedExpr) -> VariableId {
    let TypedExprKind::Identifier { id, .. } = value.kind else {
        panic!("identifier")
    };

    id
}

#[test]
fn omitted_defaults_follow_written_arguments_and_remap_earlier_formals() {
    let source = "fn f(a: number, b: number = a + 1, c: number = b * 2) -> number { return c; }
        fn main() -> number { let a = 99; return f(c: 8, a: 3); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[1].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized call")
    };
    assert_eq!(sequence.bindings.len(), 3);
    assert_eq!(
        sequence.bindings[0].origin,
        CallBindingOrigin::Explicit { argument: 0 }
    );
    assert_eq!(
        sequence.bindings[1].origin,
        CallBindingOrigin::Explicit { argument: 1 }
    );
    assert_eq!(
        sequence.bindings[2].origin,
        CallBindingOrigin::Default { formal: 1 }
    );
    assert_eq!(
        sequence.formal_to_binding,
        [
            sequence.bindings[1].id,
            sequence.bindings[2].id,
            sequence.bindings[0].id
        ]
    );
    let TypedExprKind::Binary { left, .. } = &sequence.bindings[2].value.kind else {
        panic!("default addition")
    };
    assert!(
        matches!(left.kind, TypedExprKind::Identifier { id, .. } if id == sequence.bindings[1].id)
    );
    let start = source.find("a + 1").unwrap();
    assert_eq!(sequence.bindings[2].span.start(), start);
    assert_eq!(sequence.bindings[2].span.end(), start + 5);
}

#[test]
fn every_declared_default_is_checked_before_bodies_and_in_source_order() {
    for use_site in [
        "",
        "hidden(2);",
        "return; hidden(2);",
        "false && { hidden(2); true };",
    ] {
        let source = format!(
            "fn main() -> void {{ {use_site} }}
             fn hidden(value: number = missing_first) -> void {{}}
             fn later(value: number = missing_second) -> void {{}}"
        );
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        assert_eq!(
            compiler.analyze(&source),
            Err(CompileError::Semantic(SemanticError::UndefinedVariable {
                name: "missing_first".into(),
                span: source_span(&source, "missing_first"),
            })),
            "{source}"
        );
    }

    let source = "fn main() -> void { unknown_body; }
        fn hidden(value: number = unknown_default) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "unknown_default".into(),
            span: source_span(source, "unknown_default"),
        }
    );
}

#[test]
fn earlier_default_error_precedes_a_later_functions_invalid_parameter_type() {
    let source = "fn first(value: number = missing_default) -> void {}
        fn later(value: void) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "missing_default".into(),
            span: source_span(source, "missing_default"),
        }
    );
}

#[test]
fn earlier_default_error_precedes_a_later_duplicate_function() {
    let source = "fn first(value: number = missing_default) -> void {}
        fn first() -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "missing_default".into(),
            span: source_span(source, "missing_default"),
        }
    );
}

#[test]
fn earlier_default_error_precedes_a_later_parameter_type_in_the_same_function() {
    let source = "fn f(first: number = missing_default, later: void = 1) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "missing_default".into(),
            span: source_span(source, "missing_default"),
        }
    );
}

#[test]
fn each_parameter_type_is_checked_before_its_own_default_whitelist() {
    for expression in ["missing_default", "[]"] {
        let parameter = format!("value: void = {expression}");
        let source = format!("fn f({parameter}) -> void {{}}");
        assert_eq!(
            semantic_error(&source),
            SemanticError::InvalidVoidType {
                name: "value".into(),
                span: source_span(&source, &parameter),
            }
        );
    }
}

#[test]
fn feature_path_checks_parameter_types_in_declarations_without_defaults() {
    let source = "fn first(value: void) -> void {}
        fn later(value: number = missing_default) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::InvalidVoidType {
            name: "value".into(),
            span: source_span(source, "value: void"),
        }
    );
}

#[test]
fn no_default_programs_keep_registration_errors_before_earlier_body_errors() {
    let source = "fn first() -> void { missing_body; }
        fn later(value: void) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::InvalidVoidType {
            name: "value".into(),
            span: source_span(source, "value: void"),
        }
    );

    let source = "fn first() -> void { missing_body; }
        fn first() -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::DuplicateFunction {
            name: "first".into(),
            span: source_span(source, "fn first() -> void {}"),
        }
    );
}

#[test]
fn whitelist_rejects_allocations_reads_control_flow_writes_and_nonbuiltin_calls() {
    for expression in [
        "xs[0]",
        "[]",
        "[1]",
        "0..1",
        "0..=1 by 2",
        "1 in r",
        "{ 1 }",
        "if true { 1 } else { 2 }",
        "loop { break 1; }",
        "{ a = 2; a }",
        "{ a += 2; a }",
        "{ let local = 2; local }",
        "{ return; 1 }",
        "{ loop {} 1 }",
        "helper(a)",
        "hidden(a)",
        "absent(a)",
        "len(values: xs)",
        "len([])",
    ] {
        let source = format!(
            "fn hidden(a: number, xs: number[], r: range, value: number = {expression}) -> void {{}}
             fn helper(a: number) -> number {{ return a; }}"
        );
        let error = semantic_error(&source);
        assert!(
            matches!(error, SemanticError::InvalidDefaultExpression { .. }),
            "{expression}: {error:?}"
        );
        let marker = "value: number = ";
        let start = source.find(marker).expect("default declaration") + marker.len();
        assert_eq!(
            error.span(),
            Some(Span::new(start, start + expression.len()))
        );
    }

    let mut compiler = Compiler::new();
    compiler
        .register_host_function("host", "env", "host", vec![], Type::Number)
        .unwrap();
    let source = "fn hidden(value: number = host()) -> void {}";
    assert!(matches!(
        compiler.analyze(source),
        Err(CompileError::Semantic(
            SemanticError::InvalidDefaultExpression { .. }
        ))
    ));
}

#[test]
fn whitelist_checks_skipped_children_before_the_ordinary_expression_checker() {
    for expression in [
        "false && (len(xs) == helper())",
        "true || (len(xs) == xs[0])",
        "len() + xs[0]",
        "true + len([])",
    ] {
        let source = format!(
            "fn hidden(xs: number[], value: bool = {expression}) -> void {{}}
             fn helper() -> number {{ return 1; }}"
        );
        let error = semantic_error(&source);
        assert!(
            matches!(error, SemanticError::InvalidDefaultExpression { .. }),
            "{expression}: {error:?}"
        );
        assert_eq!(error.span(), Some(source_span(&source, expression)));
    }
}

#[test]
fn defaults_resolve_only_earlier_formals_not_self_later_body_or_caller_names() {
    for (parameters, missing) in [
        ("a: number = a", "a"),
        ("a: number = b, b: number = 1", "b"),
        ("a: number = local", "local"),
        ("a: number = caller", "caller"),
    ] {
        let source = format!(
            "fn hidden({parameters}) -> void {{ let local = 1; }}
             fn main() -> void {{ let caller = 2; hidden(a: 3); }}"
        );
        let error = semantic_error(&source);
        assert!(
            matches!(&error, SemanticError::UndefinedVariable { name, .. } if name == missing),
            "{source}: {error:?}"
        );
        let start = source.find("= ").unwrap() + 2;
        assert_eq!(error.span(), Some(Span::new(start, start + missing.len())));
    }

    let source = "fn f(xs: number[], len: number, value: number = len(xs)) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::NotCallable {
            name: "len".into(),
            span: source_span(source, "len(xs)"),
        }
    );
}

#[test]
fn nontrailing_defaults_and_invalid_parameter_declarations_are_rejected() {
    let source = "fn f(a: number = 1, b: number, c: number = 2) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::NonTrailingDefault {
            name: "b".into(),
            span: source_span(source, "b: number"),
        }
    );

    let source = "fn f(a: number, a: number = 1) -> void {}";
    assert!(matches!(
        semantic_error(source),
        SemanticError::DuplicateVariable { .. }
    ));
    let source = "fn f(a: void = 1) -> void {}";
    assert!(matches!(
        semantic_error(source),
        SemanticError::InvalidVoidType { .. }
    ));
}

#[test]
fn defaults_reuse_exact_existing_expression_types_at_declaration_origins() {
    for (ty, expression, expected, found) in [
        ("number", "true", Type::Number, Type::Bool),
        ("bool", "1", Type::Bool, Type::Number),
        ("string", "1", Type::String, Type::Number),
        ("number[]", "1", Type::NumberArray, Type::Number),
        ("range", "1", Type::Range, Type::Number),
        ("number", "-true", Type::Number, Type::Bool),
        ("bool", "!1", Type::Bool, Type::Number),
        ("number", "len(1)", Type::NumberArray, Type::Number),
        ("number", "len(\"é\")", Type::NumberArray, Type::String),
    ] {
        let source = format!("fn f(value: {ty} = {expression}) -> void {{}}");
        assert_eq!(
            semantic_error(&source),
            SemanticError::TypeMismatch {
                expected,
                found,
                span: source_span(&source, expression),
            },
            "{source}"
        );
    }

    for expression in ["true + 1", "1 == false", "\"a\" < \"b\"", "r == r"] {
        let source = format!("fn f(r: range, value: bool = {expression}) -> void {{}}");
        let error = semantic_error(&source);
        assert!(
            matches!(error, SemanticError::InvalidBinaryOperands { .. }),
            "{source}: {error:?}"
        );
        assert_eq!(error.span(), Some(source_span(&source, expression)));
    }

    let source = "fn f(xs: number[], value: number = len(xs, xs)) -> void {}";
    assert_eq!(
        semantic_error(source),
        SemanticError::ArgumentCountMismatch {
            name: "len".into(),
            expected: 1,
            found: 2,
            span: source_span(source, "len(xs, xs)"),
        }
    );
}

#[test]
fn whitelist_accepts_all_existing_scalar_operators_without_constant_folding() {
    for (ty, expression) in [
        ("number", "-a"),
        ("bool", "!flag"),
        ("number", "a + 1"),
        ("number", "a - 1"),
        ("number", "a * 2"),
        ("number", "a / 0"),
        ("number", "a % 0"),
        ("bool", "a < 1"),
        ("bool", "a <= 1"),
        ("bool", "a > 1"),
        ("bool", "a >= 1"),
        ("bool", "a == 1"),
        ("bool", "a != 1"),
        ("bool", "flag == true"),
        ("bool", "flag != false"),
        ("bool", "flag && false"),
        ("bool", "flag || true"),
        ("bool", "text == \"é\""),
        ("bool", "text != \"é\""),
        ("number", "((a + 1) * 2)"),
        ("number", "0 / 0"),
    ] {
        let source = format!(
            "fn f(a: number, flag: bool, text: string, value: {ty} = {expression}) -> void {{}}"
        );
        Compiler::new()
            .analyze(&source)
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    }

    let source = "fn f(value: number = -0) -> void {} fn main() -> void { f(); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Expression(value) = &program.functions[1].body[0].kind else {
        panic!("call")
    };
    let TypedExprKind::Unary { expr, .. } = &normalized(value).bindings[0].value.kind else {
        panic!("unfolded negation")
    };
    assert!(matches!(&expr.kind, TypedExprKind::Number(value) if value == "0"));
}

#[test]
fn earlier_handle_reads_content_equality_and_builtin_len_are_allowed_and_remapped() {
    let source = r#"
        fn f(xs: number[], r: range, copy: number[] = xs, rcopy: range = r,
             same: bool = xs == copy, different: bool = xs != copy,
             size: number = len(copy), text: string = "é", flag: bool = !same) -> void {}
        fn main() -> void { f(xs: [1], r: 0..2); }
    "#;
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Expression(value) = &program.functions[1].body[0].kind else {
        panic!("call")
    };
    let sequence = normalized(value);
    assert_eq!(sequence.bindings.len(), 9);
    assert_eq!(
        identifier_id(&sequence.bindings[2].value),
        sequence.bindings[0].id
    );
    assert_eq!(
        identifier_id(&sequence.bindings[3].value),
        sequence.bindings[1].id
    );
    for formal in [4, 5] {
        let TypedExprKind::Binary { left, right, .. } = &sequence.bindings[formal].value.kind
        else {
            panic!("content equality")
        };
        assert_eq!(identifier_id(left), sequence.bindings[0].id);
        assert_eq!(identifier_id(right), sequence.bindings[2].id);
    }
    let TypedExprKind::Call { func, args, .. } = &sequence.bindings[6].value.kind else {
        panic!("builtin len")
    };
    assert!(matches!(func, FunctionRef::Builtin(_)));
    assert_eq!(identifier_id(&args[0]), sequence.bindings[2].id);
    assert!(matches!(&sequence.bindings[7].value.kind, TypedExprKind::String(text) if text == "é"));
    let TypedExprKind::Unary { expr, .. } = &sequence.bindings[8].value.kind else {
        panic!("boolean negation")
    };
    assert_eq!(identifier_id(expr), sequence.bindings[4].id);
    for formal in 2..9 {
        assert_eq!(
            sequence.bindings[formal].origin,
            CallBindingOrigin::Default { formal }
        );
    }
}

#[test]
fn chained_defaults_use_invocation_bindings_and_formal_order() {
    let source = "fn f(a: number = 4, b: number = a + 1, c: number = b + a) -> number { return c; }
        fn main() -> number { let a = 99; let b = 88; let c = 77; return f(); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[3].kind else {
        panic!("return")
    };
    let sequence = normalized(value);
    assert_eq!(
        sequence.formal_to_binding,
        [VariableId(3), VariableId(4), VariableId(5)]
    );
    for (formal, binding) in sequence.bindings.iter().enumerate() {
        assert_eq!(binding.origin, CallBindingOrigin::Default { formal });
    }
    let TypedExprKind::Binary { left, .. } = &sequence.bindings[1].value.kind else {
        panic!("first chain link")
    };
    assert_eq!(identifier_id(left), VariableId(3));
    let TypedExprKind::Binary { left, right, .. } = &sequence.bindings[2].value.kind else {
        panic!("second chain link")
    };
    assert_eq!(identifier_id(left), VariableId(4));
    assert_eq!(identifier_id(right), VariableId(3));
    let TypedExprKind::Call { args, .. } = &sequence.call.kind else {
        panic!("canonical call")
    };
    assert_eq!(
        args.iter().map(identifier_id).collect::<Vec<_>>(),
        [VariableId(3), VariableId(4), VariableId(5)]
    );
}

#[test]
fn explicit_later_formals_are_available_before_an_omitted_default_and_supersede_templates() {
    let source = "fn f(a: number = 1, b: number = a + 1, c: number = b + 1) -> number { return c; }
        fn main() -> number { return f(b: 8); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let sequence = normalized(value);
    assert_eq!(
        sequence.formal_to_binding,
        [VariableId(1), VariableId(0), VariableId(2)]
    );
    assert_eq!(
        sequence.bindings[0].origin,
        CallBindingOrigin::Explicit { argument: 0 }
    );
    assert_eq!(
        sequence.bindings[1].origin,
        CallBindingOrigin::Default { formal: 0 }
    );
    assert_eq!(
        sequence.bindings[2].origin,
        CallBindingOrigin::Default { formal: 2 }
    );
    let TypedExprKind::Binary { left, .. } = &sequence.bindings[2].value.kind else {
        panic!("default")
    };
    assert_eq!(identifier_id(left), VariableId(0));
}

#[test]
fn overflowing_default_literals_are_checked_even_when_unused_supplied_or_skipped() {
    let literal = "9".repeat(400);
    for (ty, expression, call) in [
        ("number", literal.clone(), ""),
        ("number", literal.clone(), "hidden(1);"),
        (
            "bool",
            format!("false && ({literal} == 0)"),
            "hidden(true);",
        ),
        ("bool", format!("true || ({literal} == 0)"), ""),
    ] {
        let source = format!(
            "fn hidden(value: {ty} = {expression}) -> void {{}} fn main() -> void {{ {call} }}"
        );
        assert_eq!(
            semantic_error(&source),
            SemanticError::DefaultNumberLiteralOutOfRange {
                literal: literal.clone(),
                span: source_span(&source, &expression),
            }
        );
    }
}

#[test]
fn underflowing_defaults_retain_literals_and_ordinary_body_overflow_keeps_its_stage() {
    let underflow = format!("0.{}1", "0".repeat(400));
    let source =
        format!("fn f(value: number = {underflow}) -> void {{}} fn main() -> void {{ f(); }}");
    let program = Compiler::new().analyze(&source).unwrap();
    let TypedStatementKind::Expression(value) = &program.functions[1].body[0].kind else {
        panic!("call")
    };
    assert!(
        matches!(&normalized(value).bindings[0].value.kind, TypedExprKind::Number(value) if value == &underflow)
    );

    let overflow = "9".repeat(400);
    let source = format!("fn f(value: number = 1) -> number {{ return {overflow}; }}");
    let compiler = Compiler::new();
    let program = compiler.analyze(&source).unwrap();
    assert!(
        matches!(compiler.compile(&program), Err(CodegenError::NumberLiteralOutOfRange { literal, .. }) if literal == overflow)
    );
    assert!(matches!(
        compiler.compile_source(&source),
        Err(CompileError::Codegen(
            CodegenError::NumberLiteralOutOfRange { .. }
        ))
    ));
}

#[test]
fn forged_raw_default_literals_use_the_shared_lexical_rules_and_preserve_origins() {
    let tokens = Lexer::new("fn f(value: number) -> void {}").tokenize();
    let mut raw = Parser::new(&tokens).parse_program().unwrap();
    let mut analyzer = SemanticAnalyzer::new();
    let span = Span::new(71, 93);

    for literal in [
        "", "NaN", "Infinity", "inf", "-1", "+1", "1e3", ".5", "1.", "1.2.3", "1_000", " 1", "1 ",
        "١",
    ] {
        raw.functions[0].params[0].default = Some(DefaultExpression {
            value: Expr::Number(literal.into()),
            span,
        });
        assert_eq!(
            analyzer.analyze(&raw),
            Err(SemanticError::InvalidDefaultNumberLiteral {
                literal: literal.into(),
                span,
            })
        );
    }

    let overflow = "9".repeat(400);
    raw.functions[0].params[0].default = Some(DefaultExpression {
        value: Expr::Number(overflow.clone()),
        span,
    });
    assert_eq!(
        analyzer.analyze(&raw),
        Err(SemanticError::DefaultNumberLiteralOutOfRange {
            literal: overflow,
            span,
        })
    );

    for literal in [
        "0".into(),
        "0001.25".into(),
        format!("0.{}1", "0".repeat(400)),
    ] {
        raw.functions[0].params[0].default = Some(DefaultExpression {
            value: Expr::Number(literal),
            span,
        });
        analyzer.analyze(&raw).unwrap();
    }
}

#[test]
fn bound_call_structure_and_missing_required_errors_precede_explicit_expressions() {
    for (call, kind, argument, excerpt) in [
        ("f(a: missing, z: missing)", "unknown", "z", "z: missing"),
        ("f(a: missing, a: missing)", "duplicate", "a", "a: missing"),
        ("f(missing, a: missing)", "duplicate", "a", "a: missing"),
        ("f(a: missing, missing)", "after", "", "missing"),
        (
            "f(missing, missing, missing, missing, a: missing)",
            "excess",
            "",
            "missing",
        ),
        ("f(c: missing)", "required", "a", ""),
        ("f(missing)", "required", "b", ""),
        ("f(a: missing)", "required", "b", ""),
        ("f()", "required", "a", ""),
        (
            "f(z: missing, a: missing, a: missing)",
            "unknown",
            "z",
            "z: missing",
        ),
        (
            "f(a: missing, a: missing, z: missing)",
            "duplicate",
            "a",
            "a: missing",
        ),
    ] {
        let source = format!(
            "fn f(a: number, b: number, c: number = 1) -> void {{}} fn main() -> void {{ {call}; }}"
        );
        let error = semantic_error(&source);
        match kind {
            "unknown" => assert!(
                matches!(&error, SemanticError::UnknownArgument { argument: actual, .. } if actual == argument)
            ),
            "duplicate" => assert!(
                matches!(&error, SemanticError::DuplicateArgument { argument: actual, .. } if actual == argument)
            ),
            "after" => assert!(matches!(error, SemanticError::PositionalAfterNamed { .. })),
            "excess" => assert!(matches!(
                error,
                SemanticError::TooManyPositionalArguments { .. }
            )),
            "required" => assert!(
                matches!(&error, SemanticError::MissingArgument { argument: actual, .. } if actual == argument)
            ),
            _ => unreachable!(),
        }
        let span = error.span().unwrap();
        if excerpt.is_empty() {
            assert_eq!(span, source_span(&source, &format!("{call};")));
        } else {
            assert_eq!(&source[span.start()..span.end()], excerpt);
        }
    }
}

#[test]
fn bound_call_expression_checks_follow_written_order_and_call_site_origins() {
    let source =
        "fn f(a: number, b: number = 1) -> void {} fn main() -> void { f(b: false, a: missing); }";
    assert_eq!(
        semantic_error(source),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: source_span(source, "b: false"),
        }
    );
    let source = "fn f(a: number, b: number = 1) -> void {} fn main() -> void { f(b: missing_b, a: missing_a); }";
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "missing_b".into(),
            span: source_span(source, "b: missing_b"),
        }
    );
    let source = "fn f(a: number, b: number = 1) -> void {} fn main() -> void { f(false); }";
    assert_eq!(
        semantic_error(source),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: source_span(source, "false"),
        }
    );
}

#[test]
fn explicit_arguments_and_following_statements_cannot_lookup_synthetic_formals() {
    for body in ["f(a: 1, b: a);", "f(1); a;"] {
        let source =
            format!("fn f(a: number, b: number = a) -> void {{}} fn main() -> void {{ {body} }}");
        assert!(
            matches!(semantic_error(&source), SemanticError::UndefinedVariable { name, .. } if name == "a")
        );
    }

    let source =
        "fn f(a: number, b: number = a) -> void {} fn main() -> void { let a = 99; f(a: a); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Expression(value) = &program.functions[1].body[1].kind else {
        panic!("call")
    };
    let sequence = normalized(value);
    assert_eq!(identifier_id(&sequence.bindings[0].value), VariableId(0));
    assert_eq!(
        identifier_id(&sequence.bindings[1].value),
        sequence.bindings[0].id
    );
    assert_ne!(sequence.bindings[0].id, VariableId(0));
}

#[test]
fn generated_binding_ids_follow_declaration_preorder_across_nested_calls() {
    let source = "fn f(a: number, b: number = a + 1, c: number = b + 1) -> number { return c; }
        fn main() -> number { return f(a: { let a = 3; a }, c: f(4)); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let outer = normalized(value);
    assert_eq!(
        outer.formal_to_binding,
        [VariableId(0), VariableId(6), VariableId(2)]
    );
    let TypedExprKind::Block(block) = &outer.bindings[0].value.kind else {
        panic!("argument block")
    };
    let TypedStatementKind::Let { id, .. } = block.statements[0].kind else {
        panic!("local")
    };
    assert_eq!(id, VariableId(1));
    let inner = normalized(&outer.bindings[1].value);
    assert_eq!(
        inner.formal_to_binding,
        [VariableId(3), VariableId(4), VariableId(5)]
    );
    let TypedExprKind::Binary { left, .. } = &outer.bindings[2].value.kind else {
        panic!("outer default")
    };
    assert_eq!(identifier_id(left), VariableId(0));
    let TypedExprKind::Binary { left, .. } = &inner.bindings[1].value.kind else {
        panic!("inner default")
    };
    assert_eq!(identifier_id(left), VariableId(3));
}

#[test]
fn explicit_transfers_keep_the_existing_loop_and_never_result_without_skipping_checks() {
    let source = "fn f(a: number, b: number = a) -> number { return b; }
        fn main() -> number { return loop { f(a: { break 7; }); }; }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::Loop { id, body } = &value.kind else {
        panic!("value loop")
    };
    let TypedStatementKind::Expression(value) = &body[0].kind else {
        panic!("call")
    };
    let sequence = normalized(value);
    assert_eq!(value.ty, ExprResult::Never);
    assert_eq!(sequence.call.ty, ExprResult::Value(Type::Number));
    assert_eq!(sequence.bindings[0].value.ty, ExprResult::Never);
    assert_eq!(
        sequence.bindings[1].value.ty,
        ExprResult::Value(Type::Number)
    );
    let TypedExprKind::Block(block) = &sequence.bindings[0].value.kind else {
        panic!("argument block")
    };
    let TypedStatementKind::BreakValue { target, .. } = &block.statements[0].kind else {
        panic!("break")
    };
    assert_eq!(target, id);
    assert_eq!(
        identifier_id(&sequence.bindings[1].value),
        sequence.bindings[0].id
    );

    let source = source.replace("f(a: { break 7; })", "f(a: { break 7; }, b: missing)");
    assert_eq!(
        semantic_error(&source),
        SemanticError::UndefinedVariable {
            name: "missing".into(),
            span: source_span(&source, "b: missing"),
        }
    );
}

#[test]
fn unreachable_skipped_and_unselected_bound_calls_still_check_required_arguments() {
    for body in [
        "return; f();",
        "false && { f(); true };",
        "if false { f(); }",
        "f(a: loop {}, b: missing);",
    ] {
        let source = format!(
            "fn f(a: number, b: number = 1) -> void {{}}
             fn hidden() -> void {{ {body} }} fn main() -> void {{}}"
        );
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        assert!(
            matches!(
                compiler.analyze(&source),
                Err(CompileError::Semantic(
                    SemanticError::MissingArgument { .. } | SemanticError::UndefinedVariable { .. }
                ))
            ),
            "{source}"
        );
    }
}

#[test]
fn forward_recursive_nested_and_void_default_calls_preserve_canonical_targets() {
    let source = "fn main() -> number { return recurse(a: recurse(2)); }
        fn recurse(a: number, b: number = a + 1) -> number {
            if a <= 0 { return b; } return recurse(a - 1);
        }
        fn nothing(a: number = 1) -> void { nothing(); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[0].body[0].kind else {
        panic!("return")
    };
    let sequence = normalized(value);
    let inner = normalized(&sequence.bindings[0].value);
    for call in [&sequence.call, &inner.call] {
        assert!(matches!(
            call.kind,
            TypedExprKind::Call {
                func: FunctionRef::User(was_compiler::typed_ast::FunctionId(1)),
                ..
            }
        ));
        assert_eq!(call.ty, ExprResult::Value(Type::Number));
    }
    let TypedStatementKind::Expression(value) = &program.functions[2].body[0].kind else {
        panic!("void call")
    };
    assert_eq!(value.ty, ExprResult::Void);
    assert_eq!(normalized(value).call.ty, ExprResult::Void);
}

#[test]
fn full_positional_calls_keep_the_direct_tree_legacy_diagnostics_and_bytes() {
    let source = "fn f(a: number, b: number = a + 1) -> number { return a + b; }
        fn main() -> number { return f(3, 8); }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    assert!(matches!(value.kind, TypedExprKind::Call { .. }));
    let bytes = compiler.compile(&program).unwrap();
    wasmparser::Validator::new().validate_all(&bytes).unwrap();
    assert_eq!(
        bytes,
        compiler
            .compile_source(&source.replace(" = a + 1", ""))
            .unwrap()
    );

    let source = "fn f(a: number, b: number = 1) -> void {} fn main() -> void { f(false, 2); }";
    assert_eq!(
        semantic_error(source),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: source_span(source, "f(false, 2);"),
        }
    );
    assert!(matches!(
        semantic_error(&source.replace("f(false, 2)", "f(missing, missing, missing)")),
        SemanticError::ArgumentCountMismatch {
            expected: 2,
            found: 3,
            ..
        }
    ));
}

#[test]
fn analysis_trees_and_diagnostics_are_deterministic_across_optimization_levels() {
    let source = "fn f(a: number, b: number = a + 1, c: number = b + 1) -> number { return c; }
        fn main() -> number { return f(c: 8, a: 3); }";
    let expected = Compiler::new().analyze(source).unwrap();
    let invalid: [String; 8] = [
        "fn f(a: number = missing) -> void {}".into(),
        "fn f(a: number = 1, b: number) -> void {}".into(),
        "fn f(a: number = true) -> void {}".into(),
        "fn f(a: bool = false && { true }) -> void {}".into(),
        format!("fn f(a: number = {}) -> void {{}}", "9".repeat(400)),
        "fn f(a: number, b: number = a) -> void {} fn main() -> void { f(b: missing); }".into(),
        "fn f(a: number = 1) -> void {} fn main() -> void { f(z: missing); }".into(),
        "fn f(a: number = 1) -> void {} fn main() -> void { f(a: false); }".into(),
    ];
    let expected_errors: Vec<_> = invalid
        .iter()
        .map(|source| semantic_error(source))
        .collect();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        assert_eq!(compiler.analyze(source).unwrap(), expected);
        assert_eq!(compiler.analyze(source).unwrap(), expected);
        for (source, error) in invalid.iter().zip(&expected_errors) {
            let Err(CompileError::Semantic(actual)) = compiler.analyze(source) else {
                panic!("expected semantic error for {source}")
            };
            assert_eq!(&actual, error);
            let Err(CompileError::Semantic(actual)) = compiler.compile_source(source) else {
                panic!("expected semantic error for {source}")
            };
            assert_eq!(&actual, error);
        }
    }
}

#[test]
fn declaration_scopes_do_not_leak_between_functions_or_inherit_caller_builtin_shadowing() {
    let source = "fn first(a: number = 1) -> void {}
        fn second(b: number = a) -> void {}";
    let start = source.rfind("a)").unwrap();
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "a".into(),
            span: Span::new(start, start + 1),
        }
    );

    let source = "fn f(xs: number[], size: number = len(xs)) -> number { return size; }
        fn main() -> number { let len = 99; return f([2]); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[1].kind else {
        panic!("return")
    };
    let sequence = normalized(value);
    assert!(matches!(
        sequence.bindings[1].value.kind,
        TypedExprKind::Call {
            func: FunctionRef::Builtin(was_compiler::typed_ast::BuiltinFunction::Len),
            ..
        }
    ));

    let source = "fn f(a: number = 1) -> void {} fn main() -> void { let f = 2; f(z: missing); }";
    assert!(
        matches!(semantic_error(source), SemanticError::NotCallable { name, .. } if name == "f")
    );
}

#[test]
fn default_and_explicit_origins_retain_parentheses_comments_and_utf8_byte_offsets() {
    let source = "fn f(text: string = (\"é\")) -> string { return text; }
        fn main() -> string { f(text /*é*/ : (\"λ\")); return f(); }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Expression(explicit) = &program.functions[1].body[0].kind else {
        panic!("explicit call")
    };
    let TypedStatementKind::Return(Some(default)) = &program.functions[1].body[1].kind else {
        panic!("default call")
    };
    assert_eq!(
        normalized(explicit).bindings[0].span,
        source_span(source, "text /*é*/ : (\"λ\")")
    );
    assert_eq!(
        normalized(default).bindings[0].span,
        source_span(source, "(\"é\")")
    );
}

#[test]
fn read_only_declaration_scope_does_not_change_body_parameter_mutability() {
    let source = "fn f(a: number = 1, b: number = a + 1) -> number {
        a = b; a += 2; return a;
    } fn main() -> number { return f(); }";
    Compiler::new().analyze(source).unwrap();
}

#[test]
fn malformed_default_syntax_remains_a_parse_error() {
    for source in [
        "fn f(a: number = ) -> void {}",
        "fn f(a: number = , b: number = 1) -> void {}",
        "fn f(a: number = 1 = 2) -> void {}",
        "fn f(a = 1) -> void {}",
        "fn f(a: number = 1, ) -> void {}",
        "fn f(a: number = 1) {}",
    ] {
        assert!(
            matches!(Compiler::new().analyze(source), Err(CompileError::Parse(_))),
            "{source}"
        );
    }
}
