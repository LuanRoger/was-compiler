use super::*;

#[test]
fn memory_expressions_validate_with_live_stack_prefixes_and_nested_calls_at_all_levels() {
    let source = r#"
        fn identity(a: number[]) -> number[] { return a; }
        fn combine(n: number, a: number[], s: string, b: bool) -> number {
            if b { return n + len(a); } else { return n; }
        }
        fn work(a: number[], i: number) -> number {
            return 10 + combine(2, identity([a[i], len(identity(a))]), "value", "a" == "a");
        }
    "#;
    for level in LEVELS {
        decode(
            &compiler(level, &[])
                .compile_source(source)
                .expect("validate stack prefixes"),
        );
    }
}

#[test]
fn forged_memory_roots_operands_and_nested_errors_preserve_statement_spans() {
    let span = Span::new(12, 34);
    let mut wrong_string = string("value");
    wrong_string.ty = Type::Bool.into();
    let mut wrong_array = array(vec![]);
    wrong_array.ty = Type::Number.into();
    let mut wrong_index = index(array(vec![]), number("0"));
    wrong_index.ty = Type::Bool.into();
    let mut wrong_len = len(vec![array(vec![])]);
    wrong_len.ty = Type::Bool.into();
    let mut wrong_equal = equal(string("a"), string("b"), BinaryOp::Equal);
    wrong_equal.ty = Type::Number.into();
    let cases = [
        (wrong_string, mismatch(Type::String, Type::Bool, span)),
        (
            wrong_array,
            mismatch(
                Type::Array(crate::ast::ArrayElementType::Number),
                Type::Number,
                span,
            ),
        ),
        (wrong_index, mismatch(Type::Number, Type::Bool, span)),
        (wrong_len, mismatch(Type::Number, Type::Bool, span)),
        (wrong_equal, mismatch(Type::Bool, Type::Number, span)),
        (
            array(vec![boolean()]),
            mismatch(Type::Number, Type::Bool, span),
        ),
        (
            index(string("bad"), number("0")),
            mismatch(
                Type::Array(crate::ast::ArrayElementType::Number),
                Type::String,
                span,
            ),
        ),
        (
            index(array(vec![]), boolean()),
            mismatch(Type::Number, Type::Bool, span),
        ),
        (
            len(vec![string("bad")]),
            mismatch(
                Type::Array(crate::ast::ArrayElementType::Number),
                Type::String,
                span,
            ),
        ),
        (
            equal(string("a"), number("1"), BinaryOp::Equal),
            mismatch(Type::String, Type::Number, span),
        ),
        (
            equal(string("a"), boolean(), BinaryOp::NotEqual),
            mismatch(Type::String, Type::Bool, span),
        ),
        (
            array(vec![number("NaN"), boolean()]),
            CodegenError::InvalidNumberLiteral {
                function: "bad".into(),
                literal: "NaN".into(),
                span,
            },
        ),
        (
            array(vec![boolean(), number("NaN")]),
            mismatch(Type::Number, Type::Bool, span),
        ),
        (
            index(array(vec![number("-1")]), number("NaN")),
            CodegenError::InvalidNumberLiteral {
                function: "bad".into(),
                literal: "-1".into(),
                span,
            },
        ),
    ];
    for (value, expected) in cases {
        for after_return in [false, true] {
            let mut program = return_program(value.clone(), span);
            if after_return {
                program.functions[0].body.insert(
                    0,
                    TypedStatement {
                        kind: TypedStatementKind::Return(Some(value_for_type(
                            value.ty.value_type().unwrap(),
                        ))),
                        span: Span::new(0, 7),
                    },
                );
            }
            assert_eq!(
                compile(&program).expect_err("reject malformed memory expression"),
                expected,
                "{value:?}, after return {after_return}"
            );
        }
    }
}

#[test]
fn len_checks_arity_and_rejects_strings_in_source_and_typed_ast() {
    let span = Span::new(20, 40);
    for args in [vec![], vec![array(vec![]), number("NaN")]] {
        let count = args.len();
        assert_eq!(
            compile(&return_program(len(args), span)),
            Err(CodegenError::ArgumentCountMismatch {
                function: "bad".into(),
                reference: FunctionRef::Builtin(BuiltinFunction::Len),
                expected: 1,
                found: count,
                span,
            })
        );
    }
    for source in [
        "fn bad() -> number { return len(\"text\"); }",
        "fn bad() -> number { return len(1); }",
        "fn bad() -> number[] { return [true]; }",
        "fn bad(a: number[]) -> number { return a[true]; }",
    ] {
        assert!(
            matches!(
                Compiler::new().compile_source(source),
                Err(CompileError::Semantic(_))
            ),
            "{source}"
        );
    }
}

#[test]
fn indexed_assignment_checks_identifier_base_target_index_and_rhs_types() {
    let source = "fn bad(a: number[]) -> void { a[0] = 1; }";
    let compiler = Compiler::new();
    let original = compiler.analyze(source).unwrap();
    let span = original.functions[0].body[0].span;
    for (which, expected) in [
        (
            0,
            CodegenError::InvalidAssignmentTarget {
                function: "bad".into(),
                span,
            },
        ),
        (1, mismatch(Type::Number, Type::Bool, span)),
        (2, mismatch(Type::Number, Type::Bool, span)),
        (
            3,
            mismatch(
                Type::Array(crate::ast::ArrayElementType::Number),
                Type::Bool,
                span,
            ),
        ),
    ] {
        let mut program = original.clone();
        let TypedStatementKind::Assignment { target, value } =
            &mut program.functions[0].body[0].kind
        else {
            unreachable!()
        };
        let TypedExprKind::Index { expr, index } = &mut target.kind else {
            unreachable!()
        };
        match which {
            0 => {
                **expr = array(vec![]);
                *value = number("NaN");
            }
            1 => {
                target.ty = Type::Bool.into();
                *value = number("NaN");
            }
            2 => {
                **index = boolean();
                *value = number("NaN");
            }
            3 => {
                expr.ty = Type::Bool.into();
                *value = number("NaN");
            }
            _ => unreachable!(),
        }
        assert_eq!(compile(&program), Err(expected));
    }
    let mut program = original;
    let TypedStatementKind::Assignment { value, .. } = &mut program.functions[0].body[0].kind
    else {
        unreachable!()
    };
    *value = boolean();
    assert_eq!(
        compile(&program),
        Err(mismatch(Type::Number, Type::Bool, span))
    );
}

#[test]
fn memory_local_initializers_remain_checked_in_unreachable_nested_blocks() {
    let original = Compiler::new()
        .analyze(
            r#"
        fn bad() -> void {
            return;
            if true {
                while false { let s: string = "valid"; let a: number[] = []; }
            }
        }
    "#,
        )
        .unwrap();
    for (position, expected_type) in [
        (0, Type::String),
        (1, Type::Array(crate::ast::ArrayElementType::Number)),
    ] {
        let mut program = original.clone();
        let TypedStatementKind::If { then, .. } = &mut program.functions[0].body[1].kind else {
            unreachable!()
        };
        let TypedStatementKind::While { body, .. } = &mut then[0].kind else {
            unreachable!()
        };
        let span = body[position].span;
        let TypedStatementKind::Let { value, .. } = &mut body[position].kind else {
            unreachable!()
        };
        *value = number("NaN");
        assert_eq!(
            compile(&program),
            Err(mismatch(expected_type, Type::Number, span))
        );
    }
}

#[test]
fn memory_handle_types_are_not_interchangeable_in_assignments_and_host_calls() {
    let hosts = [host(0, "text_host", &[Type::String], Type::String)];
    let compiler = compiler(OptimizationLevel::None, &hosts);
    for source in [
        "fn bad(s: string, a: number[]) -> void { s = s; }",
        "fn bad(s: string, a: number[]) -> void { text_host(s); }",
    ] {
        let mut program = compiler.analyze(source).unwrap();
        let span = program.functions[0].body[0].span;
        let a = &program.functions[0].params[1];
        let replacement = TypedExpr {
            kind: TypedExprKind::Identifier {
                id: a.id,
                name: a.name.clone(),
            },
            ty: Type::Array(crate::ast::ArrayElementType::Number).into(),
        };
        match &mut program.functions[0].body[0].kind {
            TypedStatementKind::Assignment { value, .. } => *value = replacement,
            TypedStatementKind::Expression(TypedExpr {
                kind: TypedExprKind::Call { args, .. },
                ..
            }) => {
                args[0] = replacement;
            }
            _ => unreachable!(),
        }
        assert_eq!(
            compile(&program),
            Err(mismatch(
                Type::String,
                Type::Array(crate::ast::ArrayElementType::Number),
                span
            ))
        );
    }
}
