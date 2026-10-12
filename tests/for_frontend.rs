use was_compiler::{
    CompileError, Compiler, HostRegistrationError, Type,
    ast::{BinaryOp, Expr, Program, StatementKind},
    lexer::Lexer,
    parser::{ParseError, Parser},
    semantic::{SemanticAnalyzer, SemanticError},
    tokens::{Span, TokenKind},
    typed_ast::{ExprResult, TypedExprKind, TypedStatementKind},
};

fn parse(source: &str) -> Result<Program, ParseError> {
    let tokens = Lexer::new(source).tokenize();

    Parser::new(&tokens).parse_program()
}

fn semantic_error(source: &str) -> SemanticError {
    match Compiler::new().analyze(source).unwrap_err() {
        CompileError::Semantic(error) => error,
        error => panic!("expected semantic error for {source}: {error:?}"),
    }
}

#[test]
fn range_for_exposes_a_numeric_iteration_binding() {
    let source = "fn main() -> void { for i in 0..=3 by 2 { let n: number = i; } }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::For {
        variable,
        name,
        iterable,
        body,
        ..
    } = &program.functions[0].body[0].kind
    else {
        panic!("expected typed for");
    };

    assert_eq!(name, "i");
    assert_eq!(iterable.ty, ExprResult::Value(Type::Range));

    let TypedStatementKind::Let { ty, value, .. } = &body[0].kind else {
        panic!("expected typed let");
    };

    assert_eq!(*ty, Type::Number);
    assert_eq!(value.ty, ExprResult::Value(Type::Number));
    assert!(matches!(&value.kind, TypedExprKind::Identifier { id, name }
        if id == variable && name == "i"));
}

#[test]
fn iteration_binding_rejects_writes_even_when_unreachable() {
    for prefix in ["", "continue;", "break;", "return;"] {
        for operator in ["=", "+=", "-=", "*=", "/=", "%="] {
            let source =
                format!("fn main() -> void {{ for i in 0..0 {{ {prefix} i {operator} 1; }} }}");
            let error = semantic_error(&source);
            let start = source.find(&format!("i {operator} 1;")).unwrap();
            let span = Span::new(start, start + operator.len() + 5);

            assert_eq!(
                error,
                SemanticError::ReadOnlyVariable {
                    name: "i".into(),
                    span
                },
                "{source}"
            );
            assert_eq!(error.span(), Some(span));
        }
    }
}

#[test]
fn for_keyword_preserves_boundaries_trivia_strings_and_utf8_spans() {
    let source = "/*λ*/for/*μ*/item in 0..1 {} for1 foreach For \"for\"";
    let tokens = Lexer::new(source).tokenize();

    assert_eq!(tokens[0].kind, TokenKind::For);
    assert_eq!(tokens[0].span, Span::new(6, 9));
    assert_eq!(tokens[1].kind, TokenKind::Identifier("item".into()));
    assert_eq!(tokens[1].span, Span::new(15, 19));
    assert_eq!(tokens[2].kind, TokenKind::In);
    assert_eq!(tokens[8].kind, TokenKind::Identifier("for1".into()));
    assert_eq!(tokens[9].kind, TokenKind::Identifier("foreach".into()));
    assert_eq!(tokens[10].kind, TokenKind::Identifier("For".into()));
    assert_eq!(tokens[11].kind, TokenKind::StringLiteral("for".into()));
}

#[test]
fn for_reserves_source_and_host_language_names_only() {
    for source in [
        "fn for() -> void {}",
        "fn main(for: number) -> void {}",
        "fn main() -> void { let for = 1; }",
        "fn main() -> void { for for in 0..1 {} }",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }

    let mut compiler = Compiler::new();

    assert_eq!(
        compiler.register_host_function("for", "env", "run", vec![], Type::Void),
        Err(HostRegistrationError::ReservedName { name: "for".into() })
    );
    compiler
        .register_host_function("for1", "for", "for", vec![], Type::Void)
        .unwrap();
    compiler.analyze("fn main() -> void { for1(); }").unwrap();
}

#[test]
fn parser_keeps_the_exact_for_contract_and_original_span() {
    let source = "fn main() -> void { \"λ\"; for/*step*/i in 1+2..=8 by -2 { continue; }-1; }";
    let program = parse(source).unwrap();
    let statement = &program.functions[0].body[1];
    let StatementKind::For {
        name,
        iterable,
        body,
    } = &statement.kind
    else {
        panic!("expected raw for");
    };

    assert_eq!(name, "i");
    assert_eq!(
        iterable,
        &Expr::Range {
            start: Box::new(Expr::Binary {
                left: Box::new(Expr::Number("1".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Number("2".into())),
            }),
            end: Box::new(Expr::Number("8".into())),
            step: Some(Box::new(Expr::Unary {
                op: was_compiler::ast::UnaryOp::Negate,
                expr: Box::new(Expr::Number("2".into())),
            })),
            inclusive: true,
        }
    );
    assert_eq!(body[0].kind, StatementKind::Continue);
    let start = source.find("for/*").unwrap();
    let end = source.find("}-1").unwrap() + 1;

    assert_eq!(statement.span, Span::new(start, end));
    assert_eq!(program.functions[0].body.len(), 3);
}

#[test]
fn for_is_always_a_statement_even_when_final_in_a_value_block() {
    let source = "fn main() -> void { { for i in 0..1 {} }; }";
    let program = parse(source).unwrap();
    let StatementKind::Expression(Expr::Block(block)) = &program.functions[0].body[0].kind else {
        panic!("expected value block");
    };

    assert!(block.tail.is_none());
    assert!(matches!(
        block.statements[0].kind,
        StatementKind::For { .. }
    ));

    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Expression(expr) = &program.functions[0].body[0].kind else {
        panic!("expected typed block");
    };

    assert_eq!(expr.ty, ExprResult::Void);
    Compiler::new()
        .analyze("fn main() -> number { return { for i in 0..1 {} 9 }; }")
        .unwrap();
}

#[test]
fn malformed_for_and_expression_forms_are_rejected_with_locations() {
    for fragment in [
        "for in 0..1 {}",
        "for i 0..1 {}",
        "for i: number in 0..1 {}",
        "for i in {}",
        "for i in 0..1",
        "for i in 0..1 { i }",
        "for i in 0..1 {};",
        "for i in 0..1..2 {}",
        "let n = for i in 0..1 {};",
        "return for i in 0..1 {};",
        "(for i in 0..1 {});",
        "len(for i in 0..1 {});",
        "1 + for i in 0..1 {};",
    ] {
        let source = format!("fn main() -> void {{ {fragment} }}");
        let error = parse(&source).unwrap_err();
        let span = error.span().unwrap();

        assert!(
            span.start() <= span.end() && span.end() <= source.len(),
            "{error:?}"
        );
    }

    let source = "fn main() -> void { for i 0..1 {} }";
    let start = source.find("0..1").unwrap();

    assert_eq!(
        parse(source).unwrap_err(),
        ParseError::UnexpectedToken {
            expected: "in",
            found: "number literal",
            span: Span::new(start, start + 1),
        }
    );
}

#[test]
fn truncated_for_programs_and_token_slices_do_not_panic() {
    let source = "fn main() -> void { for i in { 0..3 } { if true { continue; } break; } }";

    for end in 0..source.len() {
        assert!(parse(&source[..end]).is_err(), "prefix ending at {end}");
    }

    let tokens = Lexer::new(source).tokenize();

    for end in 0..tokens.len() - 1 {
        assert!(
            Parser::new(&tokens[..end]).parse_program().is_err(),
            "token prefix {end}"
        );
    }
}

#[test]
fn for_accepts_array_values_from_existing_expression_forms() {
    for iterable in [
        "[]",
        "[1, 2]",
        "values",
        "make()",
        "{ let a = [1]; a }",
        "if true { [1] } else { [] }",
        "loop { break [1]; }",
    ] {
        let source = format!(
            "fn make() -> number[] {{ return [1, 2]; }}
             fn main(values: number[]) -> void {{ for i in {iterable} {{ let n: number = i; }} }}"
        );
        Compiler::new().analyze(&source).unwrap();
    }
}

#[test]
fn array_iteration_preserves_readonly_scope_and_checks_skipped_children() {
    for iterable in ["[]", "[1, 2]"] {
        for prefix in ["", "continue;", "break;", "return;"] {
            for operator in ["=", "+=", "-=", "*=", "/=", "%="] {
                let source = format!(
                    "fn main() -> void {{ if false {{ for i in {iterable} {{ {prefix} i {operator} missing; }} }} }}"
                );
                assert!(
                    matches!(semantic_error(&source), SemanticError::ReadOnlyVariable { name, .. } if name == "i")
                );
            }
        }
    }
    Compiler::new()
        .analyze("fn main(i: number[]) -> void { for i in i { let i = i; i += 1; } i = []; }")
        .unwrap();
    for source in [
        "fn main() -> void { for i in [] {} i; }",
        "fn main() -> void { for i in [] { return; break 1; } }",
        "fn main() -> void { for i in [] { missing; } }",
        "fn main() -> number { for i in [1] { return i; } }",
    ] {
        assert!(Compiler::new().analyze(source).is_err(), "{source}");
    }
}

#[test]
fn for_accepts_range_values_from_existing_expression_forms() {
    for iterable in [
        "0..3",
        "3..=0 by -1",
        "bounds",
        "make()",
        "{ let r = 0..3; r }",
        "if true { 0..3 } else { 3..0 by -1 }",
        "loop { break 0..3; }",
    ] {
        let source = format!(
            "fn make() -> range {{ return 0..3; }}
             fn main(bounds: range) -> void {{ for i in {iterable} {{ i + 1; }} }}"
        );

        Compiler::new().analyze(&source).unwrap();
    }
}

#[test]
fn iterable_rejects_types_other_than_range_or_numeric_array() {
    for (iterable, found) in [
        ("1", Type::Number),
        ("true", Type::Bool),
        ("\"text\"", Type::String),
        ("noop()", Type::Void),
        ("0 in 0..1", Type::Bool),
    ] {
        let source = format!(
            "fn noop() -> void {{}}
             fn main() -> void {{ for i in {iterable} {{ missing; }} }}"
        );

        assert!(
            matches!(semantic_error(&source), SemanticError::TypeMismatch {
            expected: Type::Range, found: actual, ..
        } if actual == found),
            "{source}"
        );
    }
}

#[test]
fn iterable_resolves_outer_names_before_introducing_the_iteration_binding() {
    let source = "fn main(i: range) -> void { for i in i { i + 1; } }";
    let program = Compiler::new().analyze(source).unwrap();
    let param = &program.functions[0].params[0];
    let TypedStatementKind::For {
        variable, iterable, ..
    } = &program.functions[0].body[0].kind
    else {
        panic!("expected for");
    };

    assert_ne!(*variable, param.id);
    assert!(matches!(iterable.kind, TypedExprKind::Identifier { id, .. } if id == param.id));
    assert!(matches!(
        semantic_error("fn main() -> void { for i in i {} }"),
        SemanticError::UndefinedVariable { name, .. } if name == "i"
    ));
    Compiler::new()
        .analyze("fn main(i: number) -> void { for i in { i += 1; i..3 } {} i = 4; }")
        .unwrap();
}

#[test]
fn body_let_shadowing_is_distinct_mutable_and_does_not_change_outer_binding() {
    let source = "fn main() -> void {
        let i = 99;
        for i in 0..3 { let i = i; i = 4; i += 1; }
        i = 5;
    }";
    let program = Compiler::new().analyze(source).unwrap();
    let body = &program.functions[0].body;
    let TypedStatementKind::Let { id: outer, .. } = &body[0].kind else {
        panic!("expected outer binding");
    };
    let TypedStatementKind::For {
        variable,
        body: loop_body,
        ..
    } = &body[1].kind
    else {
        panic!("expected for");
    };
    let TypedStatementKind::Let {
        id: inner, value, ..
    } = &loop_body[0].kind
    else {
        panic!("expected shadow binding");
    };

    assert_ne!(outer, variable);
    assert_ne!(inner, variable);
    assert!(matches!(value.kind, TypedExprKind::Identifier { id, .. } if id == *variable));

    for statement in &loop_body[1..] {
        let target = match &statement.kind {
            TypedStatementKind::Assignment { target, .. }
            | TypedStatementKind::CompoundAssignment { target, .. } => target,
            _ => panic!("expected write to shadow binding"),
        };

        assert!(matches!(target.kind, TypedExprKind::Identifier { id, .. } if id == *inner));
    }

    let TypedStatementKind::Assignment { target, .. } = &body[2].kind else {
        panic!("expected outer write");
    };

    assert!(matches!(target.kind, TypedExprKind::Identifier { id, .. } if id == *outer));
}

#[test]
fn iteration_scope_ends_at_the_body_and_inner_shadow_scope_restores_readonly() {
    assert!(matches!(
        semantic_error("fn main() -> void { for i in 0..1 {} i; }"),
        SemanticError::UndefinedVariable { name, .. } if name == "i"
    ));
    assert!(matches!(
        semantic_error("fn main() -> void { for i in 0..1 { { let i = 1; i += 1; }; i = 2; } }"),
        SemanticError::ReadOnlyVariable { name, .. } if name == "i"
    ));
    assert!(matches!(
        semantic_error("fn main() -> void { for i in 0..1 { let i = 1; let i = 2; } }"),
        SemanticError::DuplicateVariable { name, .. } if name == "i"
    ));
}

#[test]
fn readonly_target_errors_precede_rhs_errors_and_indexing_remains_typed() {
    for operator in ["=", "+="] {
        let source = format!("fn main() -> void {{ for i in 0..1 {{ i {operator} missing; }} }}");

        assert!(matches!(
            semantic_error(&source),
            SemanticError::ReadOnlyVariable { .. }
        ));
    }

    assert!(matches!(
        semantic_error("fn main() -> void { for i in 0..1 { i[0] = 1; } }"),
        SemanticError::TypeMismatch {
            expected: Type::NumberArray,
            found: Type::Number,
            ..
        }
    ));
}

#[test]
fn body_transfers_resolve_the_nearest_active_loop() {
    let source = "fn main() -> void { loop {
        for i in 0..3 {
            continue;
            while true { continue; break; }
            for j in 0..2 { continue; break; }
            { if true { continue; } else { break; } };
            break;
        }
        continue;
        break;
    } }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Loop { id: outer, body } = &program.functions[0].body[0].kind else {
        panic!("expected outer loop");
    };
    let TypedStatementKind::For {
        id, body: for_body, ..
    } = &body[0].kind
    else {
        panic!("expected for");
    };

    assert_ne!(outer, id);
    assert_eq!(
        for_body[0].kind,
        TypedStatementKind::Continue { target: *id }
    );
    assert_eq!(for_body[4].kind, TypedStatementKind::Break { target: *id });
    assert_eq!(
        body[1].kind,
        TypedStatementKind::Continue { target: *outer }
    );
    assert_eq!(body[2].kind, TypedStatementKind::Break { target: *outer });

    let TypedStatementKind::While {
        id: nested_while,
        body,
        ..
    } = &for_body[1].kind
    else {
        panic!("expected nested while");
    };

    assert_ne!(nested_while, id);
    assert_eq!(
        body[0].kind,
        TypedStatementKind::Continue {
            target: *nested_while
        }
    );
    assert_eq!(
        body[1].kind,
        TypedStatementKind::Break {
            target: *nested_while
        }
    );

    let TypedStatementKind::For {
        id: nested_for,
        body,
        ..
    } = &for_body[2].kind
    else {
        panic!("expected nested for");
    };

    assert_ne!(nested_for, id);
    assert_ne!(nested_for, nested_while);
    assert_eq!(
        body[0].kind,
        TypedStatementKind::Continue {
            target: *nested_for
        }
    );
    assert_eq!(
        body[1].kind,
        TypedStatementKind::Break {
            target: *nested_for
        }
    );

    let TypedStatementKind::Expression(expr) = &for_body[3].kind else {
        panic!("expected nested value block");
    };
    let TypedExprKind::Block(block) = &expr.kind else {
        panic!("expected block");
    };
    let TypedExprKind::If {
        then,
        else_: Some(else_),
        ..
    } = &block.tail.as_ref().unwrap().kind
    else {
        panic!("expected tail if");
    };

    assert_eq!(
        then.statements[0].kind,
        TypedStatementKind::Continue { target: *id }
    );
    assert_eq!(
        else_.statements[0].kind,
        TypedStatementKind::Break { target: *id }
    );
}

#[test]
fn iterable_transfers_target_enclosing_loops_not_the_new_for() {
    for transfer in ["break;", "continue;"] {
        let source = format!(
            "fn main() -> void {{ loop {{ for i in {{ {transfer} 0..1 }} {{ break; }} }} }}"
        );
        let program = Compiler::new().analyze(&source).unwrap();
        let TypedStatementKind::Loop { id: outer, body } = &program.functions[0].body[0].kind
        else {
            panic!("expected outer loop");
        };
        let TypedStatementKind::For {
            id, iterable, body, ..
        } = &body[0].kind
        else {
            panic!("expected for");
        };
        let TypedExprKind::Block(block) = &iterable.kind else {
            panic!("expected iterable block");
        };
        let expected = if transfer == "break;" {
            TypedStatementKind::Break { target: *outer }
        } else {
            TypedStatementKind::Continue { target: *outer }
        };

        assert_eq!(iterable.ty, ExprResult::Never);
        assert_eq!(block.statements[0].kind, expected);
        assert_ne!(outer, id);
        assert_eq!(body[0].kind, TypedStatementKind::Break { target: *id });
    }
}

#[test]
fn iterable_transfers_without_an_enclosing_loop_are_rejected() {
    assert!(matches!(
        semantic_error("fn main() -> void { for i in { break; 0..1 } {} }"),
        SemanticError::BreakOutsideLoop { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() -> void { for i in { continue; 0..1 } {} }"),
        SemanticError::ContinueOutsideLoop { .. }
    ));
}

#[test]
fn value_breaks_in_for_are_rejected_even_after_transfers_or_divergence() {
    for prefix in ["", "break;", "continue;", "return;", "loop {}"] {
        for iterable in ["0..0", "loop {}"] {
            let source =
                format!("fn main() -> void {{ for i in {iterable} {{ {prefix} break 1; }} }}");

            assert!(
                matches!(
                    semantic_error(&source),
                    SemanticError::BreakValueNotAllowed { .. }
                ),
                "{source}"
            );
        }
    }

    Compiler::new()
        .analyze("fn main() -> void { for i in 0..1 { let n = loop { break 1; }; } }")
        .unwrap();
}

#[test]
fn iterable_return_and_divergence_do_not_invent_normal_completion() {
    for iterable in ["{ return 7; }", "{ return 7; 0..1 }", "loop {}"] {
        let source = format!("fn main() -> number {{ for i in {iterable} {{ i; }} }}");

        Compiler::new().analyze(&source).unwrap();
    }

    Compiler::new()
        .analyze(
            "fn main(flag: bool) -> number {
            for i in if flag { return 7; } else { loop {} } { i; }
        }",
        )
        .unwrap();
}

#[test]
fn range_for_flow_is_conservative_about_empty_iteration() {
    for iterable in ["0..0", "0..1", "0..=0"] {
        let source = format!("fn main() -> number {{ for i in {iterable} {{ return i; }} }}");

        assert!(matches!(
            semantic_error(&source),
            SemanticError::MissingReturn { .. }
        ));
        Compiler::new()
            .analyze(&format!(
                "fn main() -> number {{ for i in {iterable} {{ loop {{}} }} return 8; }}"
            ))
            .unwrap();
    }

    assert!(matches!(
        semantic_error(
            "fn main(flag: bool) -> number {
            for i in if flag { return 7; } else { 0..1 } { return i; }
        }"
        ),
        SemanticError::MissingReturn { .. }
    ));
}

#[test]
fn iterable_outer_break_flow_is_not_consumed_by_for() {
    assert!(matches!(
        semantic_error(
            "fn main() -> number {
            loop { for i in { break; 0..1 } { return i; } }
        }"
        ),
        SemanticError::MissingReturn { .. }
    ));

    let program = Compiler::new()
        .analyze(
            "fn main() -> void {
        let r: range = loop { for i in { break 0..1; } {} break true; };
    }",
        )
        .unwrap();
    let TypedStatementKind::Let { value, .. } = &program.functions[0].body[0].kind else {
        panic!("expected range value loop");
    };

    assert_eq!(value.ty, ExprResult::Value(Type::Range));
}

#[test]
fn iterable_outer_continue_flow_keeps_later_value_breaks_unreachable() {
    let program = Compiler::new()
        .analyze(
            "fn main() -> void {
        let n: number = loop { for i in { continue; 0..1 } {} break true; };
    }",
        )
        .unwrap();
    let TypedStatementKind::Let { value, .. } = &program.functions[0].body[0].kind else {
        panic!("expected divergent value loop");
    };

    assert_eq!(value.ty, ExprResult::Never);
}

#[test]
fn bodies_and_later_iterable_children_are_checked_despite_divergence_or_skipping() {
    for source in [
        "fn main() -> void { for i in loop {} { missing; } }",
        "fn main() -> void { for i in { return; } { missing; } }",
        "fn main() -> void { for i in 0..0 { missing; } }",
        "fn main() -> void { return; for i in 0..1 { missing; } }",
        "fn main() -> void { false && { for i in 0..1 { missing; } true }; }",
        "fn main() -> void { for i in { return; missing; 0..1 } {} }",
        "fn main() -> void {} fn hidden() -> void { for i in 0..0 { missing; } }",
    ] {
        assert!(
            matches!(semantic_error(source), SemanticError::UndefinedVariable { name, .. }
            if name == "missing"),
            "{source}"
        );
    }

    assert!(matches!(
        semantic_error("fn main() -> void { for i in loop {} { i = 1; } }"),
        SemanticError::ReadOnlyVariable { .. }
    ));
}

#[test]
fn nested_for_iterable_uses_the_enclosing_for_binding_and_transfer_target() {
    let source = "fn main() -> void {
        for i in 0..2 {
            for i in { i + 1; continue; 0..1 } { break; }
        }
    }";
    let program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::For {
        id: outer,
        variable: outer_variable,
        body,
        ..
    } = &program.functions[0].body[0].kind
    else {
        panic!("expected enclosing for");
    };
    let TypedStatementKind::For {
        id,
        variable,
        iterable,
        body,
        ..
    } = &body[0].kind
    else {
        panic!("expected nested for");
    };
    let TypedExprKind::Block(block) = &iterable.kind else {
        panic!("expected iterable block");
    };
    let TypedStatementKind::Expression(expr) = &block.statements[0].kind else {
        panic!("expected numeric expression");
    };
    let TypedExprKind::Binary { left, .. } = &expr.kind else {
        panic!("expected numeric addition");
    };

    assert_ne!(id, outer);
    assert_ne!(variable, outer_variable);
    assert!(matches!(left.kind, TypedExprKind::Identifier { id, .. } if id == *outer_variable));
    assert_eq!(
        block.statements[1].kind,
        TypedStatementKind::Continue { target: *outer }
    );
    assert_eq!(body[0].kind, TypedStatementKind::Break { target: *id });
    assert!(matches!(
        semantic_error(
            "fn main() -> void {
            for i in 0..2 { for i in { i += 1; 0..1 } {} }
        }"
        ),
        SemanticError::ReadOnlyVariable { .. }
    ));
}

#[test]
fn for_iterable_in_a_while_condition_targets_the_active_while() {
    let program = Compiler::new()
        .analyze(
            "fn main() -> void {
        while { for i in { break; 0..1 } {} true } {}
    }",
        )
        .unwrap();
    let TypedStatementKind::While { id, cond, .. } = &program.functions[0].body[0].kind else {
        panic!("expected while");
    };
    let TypedExprKind::Block(block) = &cond.kind else {
        panic!("expected condition block");
    };
    let TypedStatementKind::For {
        id: for_id,
        iterable,
        ..
    } = &block.statements[0].kind
    else {
        panic!("expected condition for");
    };
    let TypedExprKind::Block(iterable) = &iterable.kind else {
        panic!("expected iterable block");
    };

    assert_ne!(id, for_id);
    assert_eq!(
        iterable.statements[0].kind,
        TypedStatementKind::Break { target: *id }
    );
}

#[test]
fn readonly_errors_in_runtime_skipped_code_keep_compile_error_spans() {
    let source = "fn main() -> void { for i in 0..1 { false && { i += 1; true }; } }";
    let error = Compiler::new().analyze(source).unwrap_err();
    let start = source.find("i += 1;").unwrap();

    assert_eq!(error.span(), Some(Span::new(start, start + 7)));
    assert!(matches!(
        error,
        CompileError::Semantic(SemanticError::ReadOnlyVariable { .. })
    ));
}

#[test]
fn loop_and_iteration_ids_are_independent_per_function() {
    let program = Compiler::new()
        .analyze(
            "fn first() -> void { for i in 0..1 {} }
        fn second() -> void { for j in 0..1 {} }",
        )
        .unwrap();
    let TypedStatementKind::For {
        id: first_loop,
        variable: first_variable,
        ..
    } = &program.functions[0].body[0].kind
    else {
        panic!("expected first for");
    };
    let TypedStatementKind::For {
        id: second_loop,
        variable: second_variable,
        ..
    } = &program.functions[1].body[0].kind
    else {
        panic!("expected second for");
    };

    assert_eq!(first_loop, second_loop);
    assert_eq!(first_variable, second_variable);
    assert!(matches!(
        semantic_error(
            "fn first() -> void { for i in 0..1 {} }
            fn second() -> void { continue; }"
        ),
        SemanticError::ContinueOutsideLoop { .. }
    ));
}

#[test]
fn analyzer_can_be_reused_after_for_scope_or_transfer_errors() {
    let mut analyzer = SemanticAnalyzer::new();

    for source in [
        "fn main() -> void { for i in 0..1 { i += 1; } }",
        "fn main() -> void { for i in 0..1 { break 1; } }",
        "fn main() -> void { for i in { continue; 0..1 } {} }",
    ] {
        assert!(analyzer.analyze(&parse(source).unwrap()).is_err());

        let valid =
            parse("fn main() -> void { let i = 1; i += 2; for j in 0..1 { break; } }").unwrap();
        let reused = analyzer.analyze(&valid).unwrap();
        let fresh = SemanticAnalyzer::new().analyze(&valid).unwrap();

        assert_eq!(reused, fresh);
    }
}
