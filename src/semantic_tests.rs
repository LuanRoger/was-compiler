use super::*;
use crate::{ast::*, lexer::Lexer, parser::Parser, tokens::Span};

fn parse(source: &str) -> Program {
    let tokens = Lexer::new(source).tokenize();

    Parser::new(&tokens)
        .parse_program()
        .unwrap_or_else(|error| panic!("failed to parse {source:?}: {error:?}"))
}

fn span(source: &str, text: &str) -> Span {
    let start = source.rfind(text).expect("expected source fragment");

    Span::new(start, start + text.len())
}

fn assert_valid(source: &str) {
    let program = parse(source);

    assert_eq!(
        SemanticAnalyzer::new().analyze(&program).map(|_| ()),
        Ok(()),
        "{source}"
    );
}

fn assert_error(source: &str, expected: SemanticError) {
    let program = parse(source);

    assert_eq!(
        SemanticAnalyzer::new().analyze(&program).map(|_| ()),
        Err(expected),
        "{source}"
    );
}

fn assert_body_mismatch(body: &str, statement: &str, expected: Type, found: Type) {
    let source = format!("fn test() -> void {{ {body} }}");
    let error_span = span(&source, statement);

    assert_error(
        &source,
        SemanticError::TypeMismatch {
            expected,
            found,
            span: error_span,
        },
    );
}

#[test]
fn accepts_empty_program_without_an_entrypoint() {
    let program = Program {
        functions: Vec::new(),
    };
    let mut analyzer = SemanticAnalyzer::new();

    assert_eq!(analyzer.analyze(&program).map(|_| ()), Ok(()));
    assert_eq!(
        analyzer
            .analyze(&parse("fn helper() -> void {}"))
            .map(|_| ()),
        Ok(())
    );
    assert_eq!(analyzer.analyze(&program).map(|_| ()), Ok(()));
}

#[test]
fn accepts_arbitrary_main_signatures_and_all_value_types() {
    assert_valid(
        r#"
        fn main(value: string, ready: bool, values: number[]) -> string {
            let count: number = len(values);
            let text: string = value;
            let flag: bool = ready;
            let copy: number[] = values;

            count = 2;
            text = "updated";
            flag = false;
            copy = [];

            return text;
        }

        fn number_value(value: number) -> number { return value; }
        fn bool_value(value: bool) -> bool { return value; }
        fn array_value(value: number[]) -> number[] { return value; }
        fn nothing() -> void { return; }
        "#,
    );
}

#[test]
fn accepts_forward_references_recursion_and_mutual_recursion() {
    assert_valid(
        "
        fn caller() -> number { return later(3); }

        fn later(n: number) -> number {
            if n <= 0 { return 0; }

            return later(n - 1);
        }

        fn even(n: number) -> bool {
            if n == 0 { return true; }

            return odd(n - 1);
        }

        fn odd(n: number) -> bool {
            if n == 0 { return false; }

            return even(n - 1);
        }
        ",
    );
}

#[test]
fn accepts_fibonacci_and_sum_using_builtin_len() {
    assert_valid(
        "
        fn fibonacci(n: number) -> number {
            if n <= 1 { return n; }

            return fibonacci(n - 1) + fibonacci(n - 2);
        }

        fn sum(values: number[]) -> number {
            let total: number = 0;
            let i: number = 0;

            while i < len(values) {
                total = total + values[i];
                i = i + 1;
            }

            return total;
        }

        fn sample() -> number {
            let values: number[] = [fibonacci(5), fibonacci(6), 7];

            values[0] = fibonacci(4);

            return sum(values) + len([]);
        }
        ",
    );
}

#[test]
fn rejects_duplicate_functions_at_the_second_function_span() {
    let source = "fn repeated() -> void {}\nfn repeated(value: number) -> number { return value; }";

    assert_error(
        source,
        SemanticError::DuplicateFunction {
            name: "repeated".into(),
            span: span(
                source,
                "fn repeated(value: number) -> number { return value; }",
            ),
        },
    );
}

#[test]
fn builtin_len_is_reserved_for_every_signature() {
    for source in [
        "fn len() -> void {}",
        "fn len(values: number[]) -> number { return 0; }",
        "fn len(value: string) -> string { return value; }",
    ] {
        assert_error(
            source,
            SemanticError::DuplicateFunction {
                name: "len".into(),
                span: span(source, source),
            },
        );
    }
}

#[test]
fn rejects_duplicate_parameters() {
    let source = "fn test(value: number, value: bool) -> void {}";

    assert_error(
        source,
        SemanticError::DuplicateVariable {
            name: "value".into(),
            span: span(source, "value: bool"),
        },
    );
}

#[test]
fn parameters_and_top_level_body_share_a_scope() {
    let source = "fn test(value: number) -> void { let value: number = 2; }";

    assert_error(
        source,
        SemanticError::DuplicateVariable {
            name: "value".into(),
            span: span(source, "let value: number = 2;"),
        },
    );
}

#[test]
fn rejects_duplicate_variables_in_each_kind_of_scope() {
    for body in [
        "let value: number = 1; let value: number = 2;",
        "if true { let value: number = 1; let value: number = 2; }",
        "if false {} else { let value: number = 1; let value: number = 2; }",
        "while false { let value: number = 1; let value: number = 2; }",
    ] {
        let source = format!("fn test() -> void {{ {body} }}");

        assert_error(
            &source,
            SemanticError::DuplicateVariable {
                name: "value".into(),
                span: span(&source, "let value: number = 2;"),
            },
        );
    }
}

#[test]
fn child_scopes_can_shadow_and_restore_outer_bindings() {
    assert_valid(
        r#"
        fn test(value: number) -> number {
            if true {
                let value: bool = true;

                if value {
                    let value: string = "nested";

                    value = "changed";
                }

                value = false;
            } else {
                let value: number[] = [];

                value = [1];
            }

            while false {
                let value: string = "loop";

                value = "changed";
            }

            value = 4;

            return value;
        }
        "#,
    );
}

#[test]
fn sibling_scopes_and_functions_can_reuse_names() {
    assert_valid(
        "
        fn first(value: number) -> void {
            if true { let shared: bool = true; }
            if false { let shared: string = \"text\"; }
            if true { let shared: number = 1; } else { let shared: number[] = []; }
            while false { let shared: number = 2; }
            while false { let shared: bool = false; }

            let shared: number = value;
        }

        fn second(value: bool) -> void { let shared: bool = value; }
        ",
    );
}

#[test]
fn child_bindings_do_not_leak_to_parents_or_siblings() {
    for body in [
        "if true { let hidden: number = 1; } hidden;",
        "if false {} else { let hidden: number = 1; } hidden;",
        "while false { let hidden: number = 1; } hidden;",
        "if true { let hidden: number = 1; } else { hidden; }",
        "if true { let hidden: number = 1; } if true { hidden; }",
        "while false { let hidden: number = 1; } while false { hidden; }",
    ] {
        let source = format!("fn test() -> void {{ {body} }}");

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "hidden".into(),
                span: span(&source, "hidden;"),
            },
        );
    }
}

#[test]
fn parameters_and_locals_do_not_leak_between_functions() {
    for source in [
        "fn first(hidden: number) -> void {} fn second() -> void { hidden; }",
        "fn first() -> void { let hidden: number = 1; } fn second() -> void { hidden; }",
    ] {
        assert_error(
            source,
            SemanticError::UndefinedVariable {
                name: "hidden".into(),
                span: span(source, "hidden;"),
            },
        );
    }
}

#[test]
fn initializer_is_analyzed_before_registering_the_variable() {
    for body in [
        "let value: number = value;",
        "let value: number = value + 1;",
        "value; let value: number = 1;",
        "let value = value;",
        "let value = value + 1;",
        "value; let value = 1;",
    ] {
        let source = format!("fn test() -> void {{ {body} }}");
        let statement = body.split(';').next().unwrap().to_owned() + ";";

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "value".into(),
                span: span(&source, &statement),
            },
        );
    }

    assert_valid(
        "fn test(value: number) -> number {
            if true { let value: number = value + 1; value = value + 1; }

            return value;
        }",
    );
}

#[test]
fn initializer_can_call_a_function_before_shadowing_it() {
    for declaration in ["let value: number", "let value"] {
        assert_valid(&format!(
            "fn value() -> number {{ return 1; }}
            fn test() -> number {{ {declaration} = value(); return value; }}"
        ));
    }
}

#[test]
fn rejects_undefined_variables_and_functions() {
    for statement in ["missing;", "missing = 1;", "missing[0] = 1;"] {
        let source = format!("fn test() -> void {{ {statement} }}");

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "missing".into(),
                span: span(&source, statement),
            },
        );
    }

    let source = "fn test() -> void { missing(); }";

    assert_error(
        source,
        SemanticError::UndefinedFunction {
            name: "missing".into(),
            span: span(source, "missing();"),
        },
    );
}

#[test]
fn functions_are_not_first_class_variable_values() {
    for name in ["helper", "len"] {
        let source = format!("fn helper() -> void {{}} fn test() -> void {{ {name}; }}");

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: name.into(),
                span: span(&source, &format!("{name};")),
            },
        );
    }
}

#[test]
fn local_and_parameter_bindings_shadow_functions_when_called() {
    for source in [
        "fn helper() -> void {} fn test() -> void { let helper: number = 1; helper(); }",
        "fn helper() -> void {} fn test(helper: bool) -> void { helper(); }",
        "fn test() -> void { let helper: number[] = []; helper(); }",
        "fn test() -> void { let helper: string = \"text\"; helper(); }",
    ] {
        assert_error(
            source,
            SemanticError::NotCallable {
                name: "helper".into(),
                span: span(source, "helper();"),
            },
        );
    }

    let source = "fn test(len: number) -> void { len([]); }";

    assert_error(
        source,
        SemanticError::NotCallable {
            name: "len".into(),
            span: span(source, "len([]);"),
        },
    );
}

#[test]
fn leaving_a_shadowing_scope_restores_function_calls() {
    assert_valid(
        "fn helper() -> number { return 1; }
        fn test() -> number {
            if true { let helper: number = 2; helper = 3; }
            while false { let len: bool = false; len = true; }

            return helper() + len([]);
        }",
    );
}

#[test]
fn rejects_void_parameters_and_variables_with_declaration_spans() {
    let source = "fn test(value: void) -> void {}";

    assert_error(
        source,
        SemanticError::InvalidVoidType {
            name: "value".into(),
            span: span(source, "value: void"),
        },
    );

    for source in [
        "fn noop() -> void {} fn test() -> void { let value: void = noop(); }",
        "fn test() -> void { let value: void = 1; }",
    ] {
        let statement = if source.contains("noop();") {
            "let value: void = noop();"
        } else {
            "let value: void = 1;"
        };

        assert_error(
            source,
            SemanticError::InvalidVoidType {
                name: "value".into(),
                span: span(source, statement),
            },
        );
    }
}

#[test]
fn inferred_bindings_keep_the_initializer_type_on_reassignment() {
    let types = [
        ("1", Type::Number),
        ("true", Type::Bool),
        ("\"text\"", Type::String),
        ("[]", Type::NumberArray),
    ];

    for (initial, expected) in types {
        for (value, found) in types {
            let statement = format!("value = {value};");
            let body = format!("let value = {initial}; {statement}");

            if expected == found {
                assert_valid(&format!("fn test() -> void {{ {body} }}"));
            } else {
                assert_body_mismatch(&body, &statement, expected, found);
            }
        }
    }

    assert_body_mismatch(
        "let values = [1]; values[0] = true;",
        "values[0] = true;",
        Type::Number,
        Type::Bool,
    );
}

#[test]
fn inferred_void_initializers_are_invalid_local_types() {
    for initializer in ["noop()", "(noop())"] {
        let statement = format!("let value = {initializer};");
        let source = format!("fn noop() -> void {{}} fn test() -> void {{ {statement} }}");

        assert_error(
            &source,
            SemanticError::InvalidVoidType {
                name: "value".into(),
                span: span(&source, &statement),
            },
        );
    }
}

#[test]
fn inference_preserves_initializer_type_and_duplicate_error_order() {
    let prefix = "fn noop() -> void {} fn test() -> void { let value = 1; ";
    let at = |statement: &str| Span::new(prefix.len(), prefix.len() + statement.len());

    for (statement, expected) in [
        (
            "let value = missing;",
            SemanticError::UndefinedVariable {
                name: "missing".into(),
                span: at("let value = missing;"),
            },
        ),
        (
            "let value = noop();",
            SemanticError::InvalidVoidType {
                name: "value".into(),
                span: at("let value = noop();"),
            },
        ),
        (
            "let value = \"text\";",
            SemanticError::DuplicateVariable {
                name: "value".into(),
                span: at("let value = \"text\";"),
            },
        ),
        (
            "let value: void = missing;",
            SemanticError::InvalidVoidType {
                name: "value".into(),
                span: at("let value: void = missing;"),
            },
        ),
        (
            "let value: bool = 1;",
            SemanticError::TypeMismatch {
                expected: Type::Bool,
                found: Type::Number,
                span: at("let value: bool = 1;"),
            },
        ),
    ] {
        let source = format!("{prefix}{statement} }}");

        assert_error(&source, expected);
    }
}

#[test]
fn inferred_declarations_preserve_same_scope_duplicate_checks() {
    for source in [
        "fn test(value: number) -> void { let value = 2; }",
        "fn test() -> void { let value: number = 1; let value = 2; }",
        "fn test() -> void { let value = 1; let value = 2; }",
        "fn test() -> void { if true { let value = 1; let value = 2; } }",
        "fn test() -> void { while false { let value = 1; let value = 2; } }",
    ] {
        assert_error(
            source,
            SemanticError::DuplicateVariable {
                name: "value".into(),
                span: span(source, "let value = 2;"),
            },
        );
    }
}

#[test]
fn inferred_initializers_still_require_valid_expression_types() {
    for (statement, expected, found) in [
        ("let value = [true];", Type::Number, Type::Bool),
        ("let value = !1;", Type::Bool, Type::Number),
        (
            "let value = len(\"text\");",
            Type::NumberArray,
            Type::String,
        ),
    ] {
        assert_body_mismatch(statement, statement, expected, found);
    }
}

#[test]
fn inferred_locals_obey_scope_and_unreachable_code_checks() {
    for body in [
        "if true { let value = 1; } let copy = value;",
        "while false { let value = 1; } let copy = value;",
        "if true { let value = 1; } else { let copy = value; }",
    ] {
        let source = format!("fn test() -> void {{ {body} }}");

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "value".into(),
                span: span(&source, "let copy = value;"),
            },
        );
    }

    for body in [
        "let value = missing;",
        "return; let value = missing;",
        "if false { let value = missing; }",
        "while false { let value = missing; }",
        "let value = false && missing;",
    ] {
        let source = format!("fn hidden() -> void {{ {body} }} fn test() -> void {{}}");
        let statement = if body.contains("false &&") {
            "let value = false && missing;"
        } else {
            "let value = missing;"
        };

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "missing".into(),
                span: span(&source, statement),
            },
        );
    }
}

#[test]
fn declarations_require_exact_types_without_coercion() {
    let types = [
        ("number", "1", Type::Number),
        ("bool", "true", Type::Bool),
        ("string", "\"text\"", Type::String),
        ("number[]", "[]", Type::NumberArray),
    ];

    for (annotation, _, expected) in &types {
        for (_, value, found) in &types {
            if expected == found {
                continue;
            }

            let statement = format!("let value: {annotation} = {value};");

            assert_body_mismatch(&statement, &statement, *expected, *found);
        }
    }
}

#[test]
fn assignments_require_the_binding_type() {
    let types = [
        ("number", "1", Type::Number),
        ("bool", "true", Type::Bool),
        ("string", "\"text\"", Type::String),
        ("number[]", "[]", Type::NumberArray),
    ];

    for (annotation, initial, expected) in &types {
        for (_, value, found) in &types {
            if expected == found {
                continue;
            }

            let statement = format!("value = {value};");
            let body = format!("let value: {annotation} = {initial}; {statement}");

            assert_body_mismatch(&body, &statement, *expected, *found);
        }
    }

    assert_body_mismatch(
        "let values: number[] = [1]; values[0] = true;",
        "values[0] = true;",
        Type::Number,
        Type::Bool,
    );
}

#[test]
fn compound_assignments_accept_all_arithmetic_operators_and_number_places() {
    for operator in ["+=", "-=", "*=", "/=", "%="] {
        assert_valid(&format!(
            "fn index() -> number {{ return 0; }}
            fn rhs() -> number {{ return 2; }}
            fn test(parameter: number, values: number[]) -> void {{
                let inferred = 1;
                let annotated: number = 1;
                parameter {operator} rhs();
                inferred {operator} parameter + rhs();
                annotated {operator} -2;
                values[index()] {operator} rhs();
                if true {{ let inferred = 2; inferred {operator} 1; }}
                while false {{ annotated {operator} 1; }}
            }}"
        ));
    }
}

#[test]
fn compound_assignments_require_number_targets_and_rhs_without_coercion() {
    for operator in ["+=", "-=", "*=", "/=", "%="] {
        for (initial, found) in [
            ("true", Type::Bool),
            ("\"text\"", Type::String),
            ("[]", Type::NumberArray),
        ] {
            let statement = format!("value {operator} 1;");
            let body = format!("let value = {initial}; {statement}");

            assert_body_mismatch(&body, &statement, Type::Number, found);

            for target in ["value", "values[0]"] {
                let statement = format!("{target} {operator} {initial};");
                let body = format!("let value = 1; let values = [1]; {statement}");

                assert_body_mismatch(&body, &statement, Type::Number, found);
            }
        }

        for target in ["value", "values[0]"] {
            let statement = format!("{target} {operator} noop();");
            let source = format!(
                "fn noop() -> void {{}} fn test() -> void {{
                    let value = 1; let values = [1]; {statement}
                }}"
            );

            assert_error(
                &source,
                SemanticError::TypeMismatch {
                    expected: Type::Number,
                    found: Type::Void,
                    span: span(&source, &statement),
                },
            );
        }
    }
}

#[test]
fn compound_assignment_checks_target_base_and_index_before_rhs() {
    for operator in ["+=", "-=", "*=", "/=", "%="] {
        for target in ["missing", "missing[also_missing]"] {
            let statement = format!("{target} {operator} rhs_missing;");
            let source = format!("fn test() -> void {{ {statement} }}");

            assert_error(
                &source,
                SemanticError::UndefinedVariable {
                    name: "missing".into(),
                    span: span(&source, &statement),
                },
            );
        }

        for (initial, found) in [
            ("1", Type::Number),
            ("true", Type::Bool),
            ("\"text\"", Type::String),
        ] {
            let statement = format!("value[missing] {operator} rhs_missing;");
            let body = format!("let value = {initial}; {statement}");

            assert_body_mismatch(&body, &statement, Type::NumberArray, found);
        }

        for (initial, found) in [
            ("true", Type::Bool),
            ("\"text\"", Type::String),
            ("[]", Type::NumberArray),
        ] {
            let statement = format!("value {operator} rhs_missing;");
            let body = format!("let value = {initial}; {statement}");

            assert_body_mismatch(&body, &statement, Type::Number, found);

            let statement = format!("values[{initial}] {operator} rhs_missing;");
            let body = format!("let values = [1]; {statement}");

            assert_body_mismatch(&body, &statement, Type::Number, found);
        }

        let statement = format!("values[index_missing] {operator} rhs_missing;");
        let source = format!("fn test(values: number[]) -> void {{ {statement} }}");

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "index_missing".into(),
                span: span(&source, &statement),
            },
        );
    }
}

#[test]
fn compound_assignment_checks_unreachable_code_and_does_not_guarantee_return() {
    for operator in ["+=", "-=", "*=", "/=", "%="] {
        for body in [
            format!("return; value {operator} true;"),
            format!("if false {{ value {operator} true; }}"),
            format!("while false {{ value {operator} true; }}"),
        ] {
            let statement = format!("value {operator} true;");
            let source = format!("fn test(value: number) -> void {{ {body} }}");

            assert_error(
                &source,
                SemanticError::TypeMismatch {
                    expected: Type::Number,
                    found: Type::Bool,
                    span: span(&source, &statement),
                },
            );
        }

        let source = format!("fn test(value: number) -> number {{ value {operator} 1; }}");

        assert_error(
            &source,
            SemanticError::MissingReturn {
                name: "test".into(),
                span: Span::new(0, source.len()),
            },
        );
    }
}

#[test]
fn plain_assignment_keeps_its_existing_target_and_rhs_error_order() {
    let source = "fn test(value: bool) -> void { value = missing; }";

    assert_error(
        source,
        SemanticError::UndefinedVariable {
            name: "missing".into(),
            span: span(source, "value = missing;"),
        },
    );

    let source = "fn test(values: number[]) -> void { values[true] = missing; }";

    assert_error(
        source,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: span(source, "values[true] = missing;"),
        },
    );
}

#[test]
fn raw_compound_assignments_reject_every_nonarithmetic_operator() {
    for op in [
        BinaryOp::Equal,
        BinaryOp::NotEqual,
        BinaryOp::LessThan,
        BinaryOp::GreaterThan,
        BinaryOp::LessThanOrEqual,
        BinaryOp::GreaterThanOrEqual,
        BinaryOp::And,
        BinaryOp::Or,
    ] {
        let mut program = parse("fn test(value: number) -> void { value += 1; }");
        let error_span = Span::new(137, 163);

        program.functions[0].body[0] = Statement {
            kind: StatementKind::CompoundAssignment {
                target: Expr::Identifier("value".into()),
                op,
                value: Expr::Identifier("rhs_missing".into()),
            },
            span: error_span,
        };

        let error = SemanticAnalyzer::new().analyze(&program).unwrap_err();

        assert_eq!(
            error,
            SemanticError::InvalidCompoundAssignmentOperator {
                op,
                span: error_span,
            }
        );
        assert_eq!(error.span(), Some(error_span));
    }
}

#[test]
fn raw_compound_assignments_reuse_identifier_rooted_place_checks() {
    for op in [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
        BinaryOp::Remainder,
    ] {
        for target in [
            Expr::Number("1".into()),
            Expr::Bool(true),
            Expr::String("text".into()),
            Expr::Array(vec![]),
            Expr::Call {
                func: "make".into(),
                args: vec![],
            },
            Expr::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(Expr::Number("1".into())),
            },
            Expr::Binary {
                left: Box::new(Expr::Number("1".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Number("2".into())),
            },
            Expr::Index {
                expr: Box::new(Expr::Array(vec![Expr::Number("1".into())])),
                index: Box::new(Expr::Number("0".into())),
            },
            Expr::Index {
                expr: Box::new(Expr::Call {
                    func: "make".into(),
                    args: vec![],
                }),
                index: Box::new(Expr::Number("0".into())),
            },
            Expr::Index {
                expr: Box::new(Expr::Index {
                    expr: Box::new(Expr::Identifier("values".into())),
                    index: Box::new(Expr::Number("0".into())),
                }),
                index: Box::new(Expr::Number("0".into())),
            },
        ] {
            let mut program = parse("fn test(values: number[]) -> void { values[0] += 1; }");
            let error_span = Span::new(137, 163);

            program.functions[0].body[0] = Statement {
                kind: StatementKind::CompoundAssignment {
                    target,
                    op,
                    value: Expr::Identifier("rhs_missing".into()),
                },
                span: error_span,
            };

            assert_eq!(
                SemanticAnalyzer::new().analyze(&program).map(|_| ()),
                Err(SemanticError::InvalidAssignmentTarget { span: error_span })
            );
        }
    }
}

#[test]
fn calls_enforce_user_function_and_builtin_argument_counts() {
    for (call, name, expected, found) in [
        ("take();", "take", 2, 0),
        ("take(1);", "take", 2, 1),
        ("take(1, true, 3);", "take", 2, 3),
        ("zero(1);", "zero", 0, 1),
        ("len();", "len", 1, 0),
        ("len([], []);", "len", 1, 2),
    ] {
        let source = format!(
            "fn take(n: number, flag: bool) -> void {{}} fn zero() -> void {{}} fn test() -> void {{ {call} }}"
        );

        assert_error(
            &source,
            SemanticError::ArgumentCountMismatch {
                name: name.into(),
                expected,
                found,
                span: span(&source, call),
            },
        );
    }
}

#[test]
fn calls_enforce_every_parameter_type_and_argument_position() {
    for (call, expected, found) in [
        ("take(false, true, \"text\", []);", Type::Number, Type::Bool),
        ("take(1, 2, \"text\", []);", Type::Bool, Type::Number),
        ("take(1, true, [], []);", Type::String, Type::NumberArray),
        (
            "take(1, true, \"text\", 2);",
            Type::NumberArray,
            Type::Number,
        ),
        ("len(1);", Type::NumberArray, Type::Number),
        ("len(true);", Type::NumberArray, Type::Bool),
        ("len(\"text\");", Type::NumberArray, Type::String),
    ] {
        let source = format!(
            "fn take(n: number, flag: bool, text: string, values: number[]) -> void {{}} fn test() -> void {{ {call} }}"
        );

        assert_error(
            &source,
            SemanticError::TypeMismatch {
                expected,
                found,
                span: span(&source, call),
            },
        );
    }

    assert_valid(
        "fn take(n: number, flag: bool, text: string, values: number[]) -> void {}
        fn test() -> void { take(len([1]), true, \"text\", []); }",
    );
}

#[test]
fn checks_expressions_inside_call_arguments() {
    let source = "fn take(n: number) -> void {} fn test() -> void { take(missing); }";

    assert_error(
        source,
        SemanticError::UndefinedVariable {
            name: "missing".into(),
            span: span(source, "take(missing);"),
        },
    );
}

#[test]
fn if_and_while_conditions_must_be_bool() {
    for keyword in ["if", "while"] {
        for (value, found) in [
            ("1", Type::Number),
            ("\"text\"", Type::String),
            ("[]", Type::NumberArray),
        ] {
            let statement = format!("{keyword} {value} {{}}");

            assert_body_mismatch(&statement, &statement, Type::Bool, found);
        }
    }

    assert_valid("fn test() -> void { if !false {} else {} while true && false {} }");
}

#[test]
fn unary_operators_require_their_exact_operand_types() {
    assert_valid(
        "fn test(n: number, flag: bool) -> void {
            let negative: number = -n;
            let positive: number = --n;
            let inverted: bool = !flag;
            let original: bool = !!flag;
        }",
    );

    for (operator, expected) in [("!", Type::Bool), ("-", Type::Number)] {
        for (value, found) in [
            ("1", Type::Number),
            ("true", Type::Bool),
            ("\"text\"", Type::String),
            ("[]", Type::NumberArray),
        ] {
            if expected == found {
                continue;
            }

            let statement = format!("{operator}{value};");

            assert_body_mismatch(&statement, &statement, expected, found);
        }
    }
}

#[test]
fn binary_operators_accept_only_their_supported_operand_pairs() {
    let operands = [
        ("1", Type::Number),
        ("true", Type::Bool),
        ("\"text\"", Type::String),
        ("[]", Type::NumberArray),
        ("noop()", Type::Void),
    ];

    for operator in [
        "+", "-", "*", "/", "%", "<", ">", "<=", ">=", "&&", "||", "==", "!=",
    ] {
        for (left_value, left) in &operands {
            for (right_value, right) in &operands {
                let valid = match operator {
                    "&&" | "||" => *left == Type::Bool && *right == Type::Bool,
                    "==" | "!=" => {
                        left == right
                            && matches!(
                                left,
                                Type::Number | Type::Bool | Type::String | Type::NumberArray
                            )
                    }
                    _ => *left == Type::Number && *right == Type::Number,
                };
                let statement = format!("{left_value} {operator} {right_value};");
                let source = format!("fn noop() -> void {{}} fn test() -> void {{ {statement} }}");

                if valid {
                    assert_valid(&source);
                } else {
                    assert_error(
                        &source,
                        SemanticError::InvalidBinaryOperands {
                            operator,
                            left: *left,
                            right: *right,
                            span: span(&source, &statement),
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn binary_results_have_the_expected_types() {
    for operator in ["+", "-", "*", "/", "%"] {
        assert_valid(&format!(
            "fn test() -> number {{ let result: number = 4 {operator} 2; return result; }}"
        ));
    }

    for operator in ["<", ">", "<=", ">=", "==", "!="] {
        assert_valid(&format!(
            "fn test() -> bool {{ let result: bool = 4 {operator} 2; return result; }}"
        ));
    }

    for expression in [
        "true && false",
        "true || false",
        "true == false",
        "true != false",
        "\"a\" == \"b\"",
        "\"a\" != \"b\"",
        "[1, 2, 3] == [1, 2, 3]",
        "[1, 2, 3] != [1, 2]",
        "[] == []",
        "[] != [1]",
    ] {
        assert_valid(&format!("fn test() -> bool {{ return {expression}; }}"));
    }
}

#[test]
fn array_equality_accepts_parameters_returns_and_boolean_contexts() {
    assert_valid(
        "fn identity(values: number[]) -> number[] { return values; }
        fn test(left: number[], right: number[]) -> bool {
            let same: bool = identity(left) == right;
            let different: bool = left != identity(right);
            if [1, 2, 3] == [1, 2, 3] && !different { return same; }
            while [] != [] { return false; }
            return same || different;
        }",
    );

    for operator in ["==", "!="] {
        let statement = format!("let result: number = [1] {operator} [1];");
        assert_body_mismatch(&statement, &statement, Type::Number, Type::Bool);
    }
}

#[test]
fn array_equality_errors_are_checked_in_unreachable_and_skipped_expressions() {
    for expression in ["[] == 1", "true != []", "[] < []"] {
        for statement in [
            format!("{expression};"),
            format!("false && ({expression});"),
            format!("true || ({expression});"),
        ] {
            let source = format!("fn test() -> void {{ return; {statement} }}");
            let (operator, left, right) = match expression {
                "[] == 1" => ("==", Type::NumberArray, Type::Number),
                "true != []" => ("!=", Type::Bool, Type::NumberArray),
                _ => ("<", Type::NumberArray, Type::NumberArray),
            };
            assert_error(
                &source,
                SemanticError::InvalidBinaryOperands {
                    operator,
                    left,
                    right,
                    span: span(&source, &statement),
                },
            );
        }
    }
}

#[test]
fn arrays_require_number_elements_and_empty_arrays_are_number_arrays() {
    assert_valid(
        "fn one() -> number { return 1; }
        fn test() -> number[] {
            let empty: number[] = [];
            let values: number[] = [1, -2, 3 + 4, one(), len(empty)];

            return values;
        }",
    );

    for (value, found) in [
        ("true", Type::Bool),
        ("\"text\"", Type::String),
        ("[]", Type::NumberArray),
    ] {
        for array in [format!("[{value}]"), format!("[1, {value}, 2]")] {
            let statement = format!("let values: number[] = {array};");

            assert_body_mismatch(&statement, &statement, Type::Number, found);
        }
    }

    assert_body_mismatch(
        "let value: number = [];",
        "let value: number = [];",
        Type::Number,
        Type::NumberArray,
    );
}

#[test]
fn indexing_requires_an_array_base_and_number_index() {
    assert_valid(
        "fn test(values: number[], index: number) -> number {
            let result: number = values[index + 1];

            values[index] = result;

            return values[-1];
        }",
    );

    for (annotation, initial, found) in [
        ("number", "1", Type::Number),
        ("bool", "true", Type::Bool),
        ("string", "\"text\"", Type::String),
    ] {
        for statement in ["value[0];", "value[0] = 1;"] {
            let body = format!("let value: {annotation} = {initial}; {statement}");

            assert_body_mismatch(&body, statement, Type::NumberArray, found);
        }
    }

    for (index, found) in [
        ("true", Type::Bool),
        ("\"text\"", Type::String),
        ("[]", Type::NumberArray),
    ] {
        for statement in [format!("values[{index}];"), format!("values[{index}] = 1;")] {
            let body = format!("let values: number[] = []; {statement}");

            assert_body_mismatch(&body, &statement, Type::Number, found);
        }
    }
}

#[test]
fn checks_nested_array_and_index_expressions() {
    for statement in [
        "let values: number[] = [1, missing];",
        "let values: number[] = []; values[missing];",
        "let values: number[] = []; values[missing] = 1;",
    ] {
        let source = format!("fn test() -> void {{ {statement} }}");
        let failing_statement = statement.rsplit("; ").next().unwrap();

        assert_error(
            &source,
            SemanticError::UndefinedVariable {
                name: "missing".into(),
                span: span(&source, failing_statement),
            },
        );
    }
}

#[test]
fn returns_require_exact_declared_types() {
    let types = [
        ("number", "1", Type::Number),
        ("bool", "true", Type::Bool),
        ("string", "\"text\"", Type::String),
        ("number[]", "[]", Type::NumberArray),
    ];

    for (annotation, _, expected) in &types {
        for (_, value, found) in &types {
            if expected == found {
                continue;
            }

            let statement = format!("return {value};");
            let source = format!("fn test() -> {annotation} {{ {statement} }}");

            assert_error(
                &source,
                SemanticError::TypeMismatch {
                    expected: *expected,
                    found: *found,
                    span: span(&source, &statement),
                },
            );
        }

        let source = format!("fn test() -> {annotation} {{ return; }}");

        assert_error(
            &source,
            SemanticError::TypeMismatch {
                expected: *expected,
                found: Type::Void,
                span: span(&source, "return;"),
            },
        );
    }
}

#[test]
fn void_functions_reject_every_return_value_including_void_calls() {
    for value in ["1", "true", "\"text\"", "[]", "noop()"] {
        let statement = format!("return {value};");
        let source = format!("fn noop() -> void {{}} fn test() -> void {{ {statement} }}");

        assert_error(
            &source,
            SemanticError::UnexpectedReturnValue {
                span: span(&source, &statement),
            },
        );
    }

    assert_valid("fn test() -> void { if true { return; } while false { return; } }");
}

#[test]
fn void_calls_are_legal_expression_statements() {
    assert_valid(
        "fn noop() -> void {}
        fn test() -> void {
            noop();
            if true { noop(); } else { noop(); }
            while false { noop(); }

            return;
        }",
    );
}

#[test]
fn void_calls_are_rejected_in_all_typed_value_contexts() {
    for (body, statement, expected) in [
        (
            "let value: number = noop();",
            "let value: number = noop();",
            Type::Number,
        ),
        (
            "let value: bool = noop();",
            "let value: bool = noop();",
            Type::Bool,
        ),
        (
            "let value: string = noop();",
            "let value: string = noop();",
            Type::String,
        ),
        (
            "let value: number[] = noop();",
            "let value: number[] = noop();",
            Type::NumberArray,
        ),
        (
            "let value: number = 1; value = noop();",
            "value = noop();",
            Type::Number,
        ),
        (
            "let values: number[] = []; values[0] = noop();",
            "values[0] = noop();",
            Type::Number,
        ),
        ("take(noop());", "take(noop());", Type::Number),
        ("len(noop());", "len(noop());", Type::NumberArray),
        ("if noop() {}", "if noop() {}", Type::Bool),
        ("while noop() {}", "while noop() {}", Type::Bool),
        ("!noop();", "!noop();", Type::Bool),
        ("-noop();", "-noop();", Type::Number),
        (
            "let values: number[] = [noop()];",
            "let values: number[] = [noop()];",
            Type::Number,
        ),
        (
            "let values: number[] = []; values[noop()];",
            "values[noop()];",
            Type::Number,
        ),
        (
            "let values: number[] = []; values[noop()] = 1;",
            "values[noop()] = 1;",
            Type::Number,
        ),
    ] {
        let source = format!(
            "fn noop() -> void {{}} fn take(value: number) -> void {{}} fn test() -> void {{ {body} }}"
        );

        assert_error(
            &source,
            SemanticError::TypeMismatch {
                expected,
                found: Type::Void,
                span: span(&source, statement),
            },
        );
    }

    for (annotation, expected) in [
        ("number", Type::Number),
        ("bool", Type::Bool),
        ("string", Type::String),
        ("number[]", Type::NumberArray),
    ] {
        let source =
            format!("fn noop() -> void {{}} fn test() -> {annotation} {{ return noop(); }}");

        assert_error(
            &source,
            SemanticError::TypeMismatch {
                expected,
                found: Type::Void,
                span: span(&source, "return noop();"),
            },
        );
    }
}

#[test]
fn nonvoid_functions_require_a_return_on_every_path() {
    for annotation in ["number", "bool", "string", "number[]"] {
        let source = format!("fn test() -> {annotation} {{}}");

        assert_error(
            &source,
            SemanticError::MissingReturn {
                name: "test".into(),
                span: span(&source, &source),
            },
        );
    }

    for body in [
        "let value: number = 1;",
        "if true { return 1; }",
        "if true { return 1; } else {}",
        "if false {} else { return 1; }",
        "if true { if false { return 1; } } else { return 2; }",
        "while true { return 1; }",
        "while false { return 1; }",
        "while true { if true { return 1; } else { return 2; } }",
        "if true { return 1; } else { while true { return 2; } }",
    ] {
        let source = format!("fn test() -> number {{ {body} }}");

        assert_error(
            &source,
            SemanticError::MissingReturn {
                name: "test".into(),
                span: span(&source, &source),
            },
        );
    }
}

#[test]
fn sequential_returns_and_complete_branches_establish_guaranteed_return() {
    for body in [
        "return 1;",
        "let value: number = 1; return value;",
        "if true { return 1; } return 2;",
        "while true { return 1; } return 2;",
        "if true { return 1; } else { return 2; }",
        "if true { if false { return 1; } else { return 2; } } else { return 3; }",
        "if true { if false { return 1; } return 2; } else { while false {} return 3; }",
        "return 1; let value: number = 2;",
        "if true { return 1; } else { return 2; } while false {}",
        "return 1; if false {}",
    ] {
        assert_valid(&format!("fn test() -> number {{ {body} }}"));
    }
}

#[test]
fn unreachable_statements_and_branches_are_still_typechecked() {
    for body in [
        "return 1; let bad: number = false;",
        "if true { return 1; } else { return 2; } let bad: number = false;",
        "if false { let bad: number = false; } return 1;",
        "while false { let bad: number = false; } return 1;",
    ] {
        let source = format!("fn test() -> number {{ {body} }}");

        assert_error(
            &source,
            SemanticError::TypeMismatch {
                expected: Type::Number,
                found: Type::Bool,
                span: span(&source, "let bad: number = false;"),
            },
        );
    }

    let source = "fn test() -> number { return 1; return false; }";

    assert_error(
        source,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: span(source, "return false;"),
        },
    );
}

#[test]
fn analyzer_can_reanalyze_the_same_program_after_success_or_failure() {
    let valid = parse("fn test(value: number) -> number { return value + len([]); }");
    let source = "fn test() -> void { if true { missing; } }";
    let invalid = parse(source);
    let mut analyzer = SemanticAnalyzer::new();

    for _ in 0..3 {
        assert_eq!(analyzer.analyze(&valid).map(|_| ()), Ok(()));
        assert_eq!(
            analyzer.analyze(&invalid).map(|_| ()),
            Err(SemanticError::UndefinedVariable {
                name: "missing".into(),
                span: span(source, "missing;"),
            })
        );
    }
}

#[test]
fn analyzer_does_not_preserve_functions_or_locals_after_success() {
    let mut analyzer = SemanticAnalyzer::new();
    let first = parse("fn carried(hidden: number) -> void { let local: bool = true; }");

    assert_eq!(analyzer.analyze(&first).map(|_| ()), Ok(()));

    let source = "fn next() -> void { carried(1); }";

    assert_eq!(
        analyzer.analyze(&parse(source)).map(|_| ()),
        Err(SemanticError::UndefinedFunction {
            name: "carried".into(),
            span: span(source, "carried(1);"),
        })
    );

    for name in ["hidden", "local"] {
        assert_eq!(analyzer.analyze(&first).map(|_| ()), Ok(()));

        let source = format!("fn next() -> void {{ {name}; }}");

        assert_eq!(
            analyzer.analyze(&parse(&source)).map(|_| ()),
            Err(SemanticError::UndefinedVariable {
                name: name.into(),
                span: span(&source, &format!("{name};")),
            })
        );
    }

    assert_eq!(
        analyzer
            .analyze(&parse(
                "fn carried(hidden: bool) -> bool { return hidden; }"
            ))
            .map(|_| ()),
        Ok(())
    );
}

#[test]
fn analyzer_cleans_nested_scopes_and_registered_functions_after_failure() {
    let source = "fn carried() -> void {} fn broken(hidden: number) -> void {
        let outer: bool = true;

        if true {
            while false {
                let inner: number = 1;

                missing;
            }
        }
    }";
    let invalid = parse(source);
    let mut analyzer = SemanticAnalyzer::new();

    for name in ["hidden", "outer", "inner"] {
        assert_eq!(
            analyzer.analyze(&invalid).map(|_| ()),
            Err(SemanticError::UndefinedVariable {
                name: "missing".into(),
                span: span(source, "missing;"),
            })
        );

        let next = format!("fn next() -> void {{ {name}; }}");

        assert_eq!(
            analyzer.analyze(&parse(&next)).map(|_| ()),
            Err(SemanticError::UndefinedVariable {
                name: name.into(),
                span: span(&next, &format!("{name};")),
            })
        );
    }

    assert!(analyzer.analyze(&invalid).is_err());

    let next = "fn next() -> void { carried(); }";

    assert_eq!(
        analyzer.analyze(&parse(next)).map(|_| ()),
        Err(SemanticError::UndefinedFunction {
            name: "carried".into(),
            span: span(next, "carried();"),
        })
    );

    assert!(analyzer.analyze(&invalid).is_err());
    assert_eq!(
        analyzer
            .analyze(&parse(
                "fn carried() -> number { return len([]); }
            fn broken(hidden: bool) -> bool { return hidden; }"
            ))
            .map(|_| ()),
        Ok(())
    );
}

#[test]
fn analyzer_recovers_after_registration_parameter_and_return_errors() {
    let mut analyzer = SemanticAnalyzer::new();

    for source in [
        "fn test() -> void {} fn test() -> void {}",
        "fn len() -> void {}",
        "fn test(value: void) -> void {}",
        "fn test(value: number, value: bool) -> void {}",
        "fn test(value: number) -> void { let value: number = 1; }",
        "fn test() -> number {}",
        "fn test() -> void { return 1; }",
        "fn test() -> number { return false; }",
    ] {
        assert!(analyzer.analyze(&parse(source)).is_err(), "{source}");
        assert_eq!(
            analyzer
                .analyze(&parse(
                    "fn test(value: number[]) -> number { return len(value); }"
                ))
                .map(|_| ()),
            Ok(()),
            "after {source}"
        );
    }
}

#[test]
fn rejects_manually_constructed_invalid_assignment_targets() {
    let targets = [
        Expr::Number("1".into()),
        Expr::Bool(true),
        Expr::String("text".into()),
        Expr::Array(Vec::new()),
        Expr::Call {
            func: "len".into(),
            args: vec![Expr::Array(Vec::new())],
        },
        Expr::Unary {
            op: UnaryOp::Negate,
            expr: Box::new(Expr::Number("1".into())),
        },
        Expr::Binary {
            left: Box::new(Expr::Number("1".into())),
            op: BinaryOp::Add,
            right: Box::new(Expr::Number("2".into())),
        },
        Expr::Index {
            expr: Box::new(Expr::Array(vec![Expr::Number("1".into())])),
            index: Box::new(Expr::Number("0".into())),
        },
        Expr::Index {
            expr: Box::new(Expr::Call {
                func: "make".into(),
                args: Vec::new(),
            }),
            index: Box::new(Expr::Number("0".into())),
        },
        Expr::Index {
            expr: Box::new(Expr::Index {
                expr: Box::new(Expr::Identifier("values".into())),
                index: Box::new(Expr::Number("0".into())),
            }),
            index: Box::new(Expr::Number("0".into())),
        },
    ];

    for target in targets {
        let mut program = parse(
            "fn make() -> number[] { return [1]; }
            fn test(values: number[]) -> void { values[0] = 1; }",
        );
        let error_span = Span::new(137, 163);

        program.functions[1].body[0] = Statement {
            kind: StatementKind::Assignment {
                target,
                value: Expr::Number("1".into()),
            },
            span: error_span,
        };

        assert_eq!(
            SemanticAnalyzer::new().analyze(&program).map(|_| ()),
            Err(SemanticError::InvalidAssignmentTarget { span: error_span })
        );
    }
}

#[test]
fn manually_constructed_index_expressions_can_read_nonidentifier_arrays() {
    for base in [
        Expr::Array(vec![Expr::Number("1".into())]),
        Expr::Call {
            func: "make".into(),
            args: Vec::new(),
        },
    ] {
        let mut program = parse(
            "fn make() -> number[] { return [1]; }
            fn test() -> number { return 1; }",
        );

        program.functions[1].body[0].kind = StatementKind::Return(Some(Expr::Index {
            expr: Box::new(base),
            index: Box::new(Expr::Number("0".into())),
        }));

        assert_eq!(
            SemanticAnalyzer::new().analyze(&program).map(|_| ()),
            Ok(())
        );
    }
}

#[test]
fn manually_constructed_index_checks_void_base_and_preserves_statement_span() {
    let mut program = parse("fn noop() -> void {} fn test() -> void { noop(); }");
    let error_span = Span::new(71, 89);

    program.functions[1].body[0] = Statement {
        kind: StatementKind::Expression(Expr::Index {
            expr: Box::new(Expr::Call {
                func: "noop".into(),
                args: Vec::new(),
            }),
            index: Box::new(Expr::Number("0".into())),
        }),
        span: error_span,
    };

    assert_eq!(
        SemanticAnalyzer::new().analyze(&program).map(|_| ()),
        Err(SemanticError::TypeMismatch {
            expected: Type::NumberArray,
            found: Type::Void,
            span: error_span,
        })
    );
}

#[test]
fn semantic_error_implements_the_standard_error_contract() {
    fn assert_error_traits<T: std::error::Error + std::fmt::Debug + PartialEq>() {}

    assert_error_traits::<SemanticError>();
}
