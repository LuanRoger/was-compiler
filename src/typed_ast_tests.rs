use super::*;
use crate::{
    ast::{BinaryOp, Block as AstBlock, Program, StatementKind as AstStatementKind, Type, UnaryOp},
    lexer::Lexer,
    parser::Parser,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, FunctionId, FunctionRef, TypedBlock, TypedExpr, TypedExprKind as ExprKind,
        TypedFunction, TypedParam, TypedProgram, TypedStatement, TypedStatementKind as StmtKind,
        VariableId,
    },
};

fn parse(source: &str) -> Program {
    let tokens = Lexer::new(source).tokenize();

    Parser::new(&tokens)
        .parse_program()
        .unwrap_or_else(|error| panic!("failed to parse {source:?}: {error:?}"))
}

fn analyze(source: &str) -> TypedProgram {
    SemanticAnalyzer::new()
        .analyze(&parse(source))
        .unwrap_or_else(|error| panic!("failed to analyze {source:?}: {error:?}"))
}

fn expr(kind: ExprKind, ty: Type) -> TypedExpr {
    TypedExpr { kind, ty }
}

fn number(value: &str) -> TypedExpr {
    expr(ExprKind::Number(value.into()), Type::Number)
}

fn identifier(id: usize, name: &str, ty: Type) -> TypedExpr {
    expr(
        ExprKind::Identifier {
            id: VariableId(id),
            name: name.into(),
        },
        ty,
    )
}

fn unary(op: UnaryOp, value: TypedExpr, ty: Type) -> TypedExpr {
    expr(
        ExprKind::Unary {
            op,
            expr: Box::new(value),
        },
        ty,
    )
}

fn binary(left: TypedExpr, op: BinaryOp, right: TypedExpr, ty: Type) -> TypedExpr {
    expr(
        ExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
        ty,
    )
}

fn call(func: FunctionRef, name: &str, args: Vec<TypedExpr>, ty: Type) -> TypedExpr {
    expr(
        ExprKind::Call {
            func,
            name: name.into(),
            args,
        },
        ty,
    )
}

fn len(value: TypedExpr) -> TypedExpr {
    call(
        FunctionRef::Builtin(BuiltinFunction::Len),
        "len",
        vec![value],
        Type::Number,
    )
}

fn returned(statement: &TypedStatement) -> &TypedExpr {
    let StmtKind::Return(Some(value)) = &statement.kind else {
        panic!("expected a return value, found {statement:?}");
    };

    value
}

fn assert_let(statement: &TypedStatement, id: usize, name: &str, ty: Type, value: TypedExpr) {
    assert_eq!(
        statement.kind,
        StmtKind::Let {
            id: VariableId(id),
            name: name.into(),
            ty,
            value,
        }
    );
}

fn assert_assignment(statement: &TypedStatement, target: TypedExpr, value: TypedExpr) {
    assert_eq!(statement.kind, StmtKind::Assignment { target, value });
}

fn collect_let_ids(block: &TypedBlock, ids: &mut Vec<VariableId>) {
    for statement in block {
        match &statement.kind {
            StmtKind::Let { id, .. } => ids.push(*id),
            StmtKind::If { then, else_, .. } => {
                collect_let_ids(then, ids);

                if let Some(else_) = else_ {
                    collect_let_ids(else_, ids);
                }
            }
            StmtKind::While { body, .. } => collect_let_ids(body, ids),
            _ => {}
        }
    }
}

fn assert_block_spans(ast: &AstBlock, typed: &TypedBlock) {
    assert_eq!(ast.len(), typed.len());

    for (original, lowered) in ast.iter().zip(typed) {
        assert_eq!(lowered.span, original.span);

        match (&original.kind, &lowered.kind) {
            (
                AstStatementKind::If { then, else_, .. },
                StmtKind::If {
                    then: typed_then,
                    else_: typed_else,
                    ..
                },
            ) => {
                assert_block_spans(then, typed_then);
                assert_eq!(else_.is_some(), typed_else.is_some());

                if let (Some(else_), Some(typed_else)) = (else_, typed_else) {
                    assert_block_spans(else_, typed_else);
                }
            }
            (
                AstStatementKind::While { body, .. },
                StmtKind::While {
                    body: typed_body, ..
                },
            ) => assert_block_spans(body, typed_body),
            (AstStatementKind::Let { .. }, StmtKind::Let { .. })
            | (AstStatementKind::Assignment { .. }, StmtKind::Assignment { .. })
            | (AstStatementKind::Return(_), StmtKind::Return(_))
            | (AstStatementKind::Expression(_), StmtKind::Expression(_)) => {}
            _ => panic!("statement kind changed: {original:?} became {lowered:?}"),
        }
    }
}

#[test]
fn inferred_declarations_resolve_every_initializer_to_a_concrete_type() {
    let source = r#"
        fn test() -> void {
            let literal = 10;
            let decimal = 10.5;
            let arithmetic = -(literal + decimal) / 2;
            let comparison = arithmetic >= 0;
            let boolean = !comparison || literal == 0;
            let text = "é";
            let same = text == "é";
            let values = [literal, decimal];
            let empty = [];
            let element = values[0];
            let length = len(empty);
            let result = identity(element);
            let alias = values;
        }

        fn identity(value: number) -> number { return value; }
    "#;
    let ast = parse(source);
    let typed = SemanticAnalyzer::new().analyze(&ast).unwrap();
    let expected = [
        Type::Number,
        Type::Number,
        Type::Number,
        Type::Bool,
        Type::Bool,
        Type::String,
        Type::Bool,
        Type::NumberArray,
        Type::NumberArray,
        Type::Number,
        Type::Number,
        Type::Number,
        Type::NumberArray,
    ];
    let body = &typed.functions[0].body;

    assert_eq!(body.len(), expected.len());
    assert_block_spans(&ast.functions[0].body, body);

    for (index, (statement, expected_type)) in body.iter().zip(expected).enumerate() {
        let StmtKind::Let { id, ty, value, .. } = &statement.kind else {
            panic!("expected declaration");
        };

        assert_eq!(*id, VariableId(index));
        assert_eq!(*ty, expected_type);
        assert_eq!(value.ty, expected_type);
        assert!(matches!(
            ast.functions[0].body[index].kind,
            AstStatementKind::Let { ty: None, .. }
        ));
    }

    let StmtKind::Let { value, .. } = &body[11].kind else {
        panic!("expected call initializer");
    };

    assert_eq!(
        *value,
        call(
            FunctionRef::User(FunctionId(1)),
            "identity",
            vec![identifier(9, "element", Type::Number)],
            Type::Number,
        )
    );
}

#[test]
fn inferred_shadow_initializers_resolve_outer_ids_and_restore_bindings() {
    let source = "fn test(value: number[]) -> number[] {
        if true {
            let value = value;
            let size = len(value);
        }

        let copy = value;
        return copy;
    }";
    let ast = parse(source);
    let typed = SemanticAnalyzer::new().analyze(&ast).unwrap();
    let body = &typed.functions[0].body;
    let StmtKind::If { then, .. } = &body[0].kind else {
        panic!("expected conditional");
    };

    assert_let(
        &then[0],
        1,
        "value",
        Type::NumberArray,
        identifier(0, "value", Type::NumberArray),
    );
    assert_let(
        &then[1],
        2,
        "size",
        Type::Number,
        len(identifier(1, "value", Type::NumberArray)),
    );
    assert_let(
        &body[1],
        3,
        "copy",
        Type::NumberArray,
        identifier(0, "value", Type::NumberArray),
    );
    assert_eq!(
        returned(&body[2]),
        &identifier(3, "copy", Type::NumberArray)
    );
    assert_block_spans(&ast.functions[0].body, body);
}

#[test]
fn nested_arithmetic_arrays_calls_and_indices_have_types_at_every_node() {
    let typed = analyze(
        "fn make(n: number) -> number { return n; }
        fn test(values: number[], index: number) -> number {
            let elements: number[] = [1, -2, make(index + 3)];

            return -(values[index + 1] + len(elements));
        }",
    );
    let body = &typed.functions[1].body;

    assert_eq!(body.len(), 2);
    assert_eq!(
        returned(&typed.functions[0].body[0]),
        &identifier(0, "n", Type::Number)
    );
    assert_let(
        &body[0],
        2,
        "elements",
        Type::NumberArray,
        expr(
            ExprKind::Array(vec![
                number("1"),
                unary(UnaryOp::Negate, number("2"), Type::Number),
                call(
                    FunctionRef::User(FunctionId(0)),
                    "make",
                    vec![binary(
                        identifier(1, "index", Type::Number),
                        BinaryOp::Add,
                        number("3"),
                        Type::Number,
                    )],
                    Type::Number,
                ),
            ]),
            Type::NumberArray,
        ),
    );
    assert_eq!(
        returned(&body[1]),
        &unary(
            UnaryOp::Negate,
            binary(
                expr(
                    ExprKind::Index {
                        expr: Box::new(identifier(0, "values", Type::NumberArray)),
                        index: Box::new(binary(
                            identifier(1, "index", Type::Number),
                            BinaryOp::Add,
                            number("1"),
                            Type::Number,
                        )),
                    },
                    Type::Number,
                ),
                BinaryOp::Add,
                len(identifier(2, "elements", Type::NumberArray)),
                Type::Number,
            ),
            Type::Number,
        )
    );
}

#[test]
fn nested_string_comparisons_and_boolean_operators_preserve_operand_types() {
    let typed = analyze(
        "fn test(text: string, flag: bool) -> bool {
            let greeting: string = \"ready\";

            return !(text == greeting) || flag && (2 < 3) && true;
        }",
    );
    let body = &typed.functions[0].body;

    assert_let(
        &body[0],
        2,
        "greeting",
        Type::String,
        expr(ExprKind::String("ready".into()), Type::String),
    );
    assert_eq!(
        returned(&body[1]),
        &binary(
            unary(
                UnaryOp::Not,
                binary(
                    identifier(0, "text", Type::String),
                    BinaryOp::Equal,
                    identifier(2, "greeting", Type::String),
                    Type::Bool,
                ),
                Type::Bool,
            ),
            BinaryOp::Or,
            binary(
                binary(
                    identifier(1, "flag", Type::Bool),
                    BinaryOp::And,
                    binary(number("2"), BinaryOp::LessThan, number("3"), Type::Bool),
                    Type::Bool,
                ),
                BinaryOp::And,
                expr(ExprKind::Bool(true), Type::Bool),
                Type::Bool,
            ),
            Type::Bool,
        )
    );
}

#[test]
fn array_equality_preserves_array_operands_and_bool_results_at_every_node() {
    let source = "fn identity(values: number[]) -> number[] { return values; }
        fn test(left: number[], right: number[]) -> bool {
            let same: bool = [1, 2, 3] == [1, 2, 3];
            let shorter: bool = [1, 2, 3] != [1, 2];
            return same && !(identity(left) != right) || ([] == []);
        }";
    let ast = parse(source);
    let typed = analyze(source);
    let body = &typed.functions[1].body;
    let array = |values: &[&str]| {
        expr(
            ExprKind::Array(values.iter().map(|value| number(value)).collect()),
            Type::NumberArray,
        )
    };

    assert_let(
        &body[0],
        2,
        "same",
        Type::Bool,
        binary(
            array(&["1", "2", "3"]),
            BinaryOp::Equal,
            array(&["1", "2", "3"]),
            Type::Bool,
        ),
    );
    assert_let(
        &body[1],
        3,
        "shorter",
        Type::Bool,
        binary(
            array(&["1", "2", "3"]),
            BinaryOp::NotEqual,
            array(&["1", "2"]),
            Type::Bool,
        ),
    );
    assert_eq!(
        returned(&body[2]),
        &binary(
            binary(
                identifier(2, "same", Type::Bool),
                BinaryOp::And,
                unary(
                    UnaryOp::Not,
                    binary(
                        call(
                            FunctionRef::User(FunctionId(0)),
                            "identity",
                            vec![identifier(0, "left", Type::NumberArray)],
                            Type::NumberArray,
                        ),
                        BinaryOp::NotEqual,
                        identifier(1, "right", Type::NumberArray),
                        Type::Bool,
                    ),
                    Type::Bool,
                ),
                Type::Bool,
            ),
            BinaryOp::Or,
            binary(array(&[]), BinaryOp::Equal, array(&[]), Type::Bool),
            Type::Bool,
        )
    );
    for (original, lowered) in ast.functions.iter().zip(&typed.functions) {
        assert_block_spans(&original.body, &lowered.body);
    }
}

#[test]
fn builtin_len_is_resolved_separately_and_empty_arrays_are_typed() {
    let typed = analyze(
        "fn arrays(values: number[]) -> number[] { len([]); return values; }
        fn size() -> number { return len([len([]), 1]); }",
    );

    assert_eq!(typed.functions.len(), 2);
    assert_eq!(typed.functions[0].id, FunctionId(0));
    assert_eq!(typed.functions[1].id, FunctionId(1));
    assert_eq!(typed.functions[0].return_type, Type::NumberArray);
    assert_eq!(
        typed.functions[0].body[0].kind,
        StmtKind::Expression(len(expr(ExprKind::Array(Vec::new()), Type::NumberArray)))
    );
    assert_eq!(
        returned(&typed.functions[0].body[1]),
        &identifier(0, "values", Type::NumberArray)
    );
    assert_eq!(
        returned(&typed.functions[1].body[0]),
        &len(expr(
            ExprKind::Array(vec![
                len(expr(ExprKind::Array(Vec::new()), Type::NumberArray)),
                number("1"),
            ]),
            Type::NumberArray,
        ))
    );
}

#[test]
fn function_ids_follow_source_order_for_forward_self_and_mutual_calls() {
    let typed = analyze(
        "fn zeta(n: number) -> number { return alpha(n); }
        fn alpha(n: number) -> number { return middle(n); }
        fn middle(n: number) -> number { return zeta(n) + middle(n); }",
    );

    assert_eq!(typed.functions.len(), 3);

    for (index, name) in ["zeta", "alpha", "middle"].iter().enumerate() {
        let function = &typed.functions[index];

        assert_eq!(function.id, FunctionId(index));
        assert_eq!(function.name, *name);
        assert_eq!(function.return_type, Type::Number);
        assert_eq!(function.params.len(), 1);
        assert_eq!(function.params[0].id, VariableId(0));
        assert_eq!(function.params[0].name, "n");
        assert_eq!(function.params[0].ty, Type::Number);
    }

    assert_eq!(
        returned(&typed.functions[0].body[0]),
        &call(
            FunctionRef::User(FunctionId(1)),
            "alpha",
            vec![identifier(0, "n", Type::Number)],
            Type::Number,
        )
    );
    assert_eq!(
        returned(&typed.functions[1].body[0]),
        &call(
            FunctionRef::User(FunctionId(2)),
            "middle",
            vec![identifier(0, "n", Type::Number)],
            Type::Number,
        )
    );
    assert_eq!(
        returned(&typed.functions[2].body[0]),
        &binary(
            call(
                FunctionRef::User(FunctionId(0)),
                "zeta",
                vec![identifier(0, "n", Type::Number)],
                Type::Number,
            ),
            BinaryOp::Add,
            call(
                FunctionRef::User(FunctionId(2)),
                "middle",
                vec![identifier(0, "n", Type::Number)],
                Type::Number,
            ),
            Type::Number,
        )
    );
}

#[test]
fn shadow_initializers_use_outer_ids_and_leaving_scopes_restores_bindings() {
    let typed = analyze(
        "fn test(value: number) -> number {
            if true {
                let value: number = value + 1;

                if true {
                    let value: number = value + 2;

                    value = value + 3;
                }

                value = value + 4;
            }

            value = value + 5;

            return value;
        }",
    );
    let body = &typed.functions[0].body;
    let StmtKind::If { cond, then, else_ } = &body[0].kind else {
        panic!("expected outer if");
    };

    assert_eq!(**cond, expr(ExprKind::Bool(true), Type::Bool));
    assert!(else_.is_none());
    assert_let(
        &then[0],
        1,
        "value",
        Type::Number,
        binary(
            identifier(0, "value", Type::Number),
            BinaryOp::Add,
            number("1"),
            Type::Number,
        ),
    );

    let StmtKind::If { then: nested, .. } = &then[1].kind else {
        panic!("expected nested if");
    };

    assert_let(
        &nested[0],
        2,
        "value",
        Type::Number,
        binary(
            identifier(1, "value", Type::Number),
            BinaryOp::Add,
            number("2"),
            Type::Number,
        ),
    );

    for (statement, id, increment) in [(&nested[1], 2, "3"), (&then[2], 1, "4"), (&body[1], 0, "5")]
    {
        assert_assignment(
            statement,
            identifier(id, "value", Type::Number),
            binary(
                identifier(id, "value", Type::Number),
                BinaryOp::Add,
                number(increment),
                Type::Number,
            ),
        );
    }

    assert_eq!(returned(&body[2]), &identifier(0, "value", Type::Number));
}

#[test]
fn variable_ids_put_parameters_first_never_reuse_scopes_and_reset_per_function() {
    let typed = analyze(
        "fn first(a: number, b: bool) -> void {
            let shared: number = a;

            if b {
                let shared: number = 1;

                if true { let nested: bool = b; }
            } else {
                let shared: string = \"other\";
            }

            if false { let shared: number[] = []; }
            while false { let shared: number = 2; }

            let tail: number = a;

            return;
        }

        fn second(shared: bool) -> void { let tail: bool = shared; }
        fn third() -> void { let tail: number = 0; }",
    );
    let first = &typed.functions[0];

    assert_eq!(first.params.len(), 2);
    assert_eq!(first.params[0].id, VariableId(0));
    assert_eq!(first.params[0].name, "a");
    assert_eq!(first.params[0].ty, Type::Number);
    assert_eq!(first.params[1].id, VariableId(1));
    assert_eq!(first.params[1].name, "b");
    assert_eq!(first.params[1].ty, Type::Bool);

    let mut ids = Vec::new();

    collect_let_ids(&first.body, &mut ids);

    assert_eq!(ids, (2..9).map(VariableId).collect::<Vec<_>>());
    assert_let(
        &first.body[0],
        2,
        "shared",
        Type::Number,
        identifier(0, "a", Type::Number),
    );
    assert_let(
        &first.body[4],
        8,
        "tail",
        Type::Number,
        identifier(0, "a", Type::Number),
    );
    assert_eq!(first.body[5].kind, StmtKind::Return(None));
    assert_eq!(typed.functions[1].params[0].id, VariableId(0));
    assert_let(
        &typed.functions[1].body[0],
        1,
        "tail",
        Type::Bool,
        identifier(0, "shared", Type::Bool),
    );
    assert!(typed.functions[2].params.is_empty());
    assert_let(
        &typed.functions[2].body[0],
        0,
        "tail",
        Type::Number,
        number("0"),
    );
}

#[test]
fn assignments_control_flow_void_calls_and_bare_returns_are_lowered() {
    let typed = analyze(
        "fn noop() -> void { return; }
        fn test(values: number[], index: number) -> void {
            values[index] = len(values);
            index = index + 1;

            if index < 2 { noop(); } else { values = []; }
            while false { index = -index; }
            if true {}

            return;
        }",
    );
    let body = &typed.functions[1].body;

    assert_eq!(typed.functions[0].return_type, Type::Void);
    assert_eq!(typed.functions[0].body[0].kind, StmtKind::Return(None));
    assert_eq!(body.len(), 6);
    assert_assignment(
        &body[0],
        expr(
            ExprKind::Index {
                expr: Box::new(identifier(0, "values", Type::NumberArray)),
                index: Box::new(identifier(1, "index", Type::Number)),
            },
            Type::Number,
        ),
        len(identifier(0, "values", Type::NumberArray)),
    );
    assert_assignment(
        &body[1],
        identifier(1, "index", Type::Number),
        binary(
            identifier(1, "index", Type::Number),
            BinaryOp::Add,
            number("1"),
            Type::Number,
        ),
    );

    let StmtKind::If { cond, then, else_ } = &body[2].kind else {
        panic!("expected if with else");
    };

    assert_eq!(
        **cond,
        binary(
            identifier(1, "index", Type::Number),
            BinaryOp::LessThan,
            number("2"),
            Type::Bool,
        )
    );
    assert_eq!(then.len(), 1);
    assert_eq!(
        then[0].kind,
        StmtKind::Expression(call(
            FunctionRef::User(FunctionId(0)),
            "noop",
            Vec::new(),
            Type::Void,
        ))
    );

    let else_ = else_.as_ref().expect("expected else block");

    assert_eq!(else_.len(), 1);
    assert_assignment(
        &else_[0],
        identifier(0, "values", Type::NumberArray),
        expr(ExprKind::Array(Vec::new()), Type::NumberArray),
    );

    let StmtKind::While {
        cond,
        body: loop_body,
    } = &body[3].kind
    else {
        panic!("expected while");
    };

    assert_eq!(**cond, expr(ExprKind::Bool(false), Type::Bool));
    assert_eq!(loop_body.len(), 1);
    assert_assignment(
        &loop_body[0],
        identifier(1, "index", Type::Number),
        unary(
            UnaryOp::Negate,
            identifier(1, "index", Type::Number),
            Type::Number,
        ),
    );

    let StmtKind::If { cond, then, else_ } = &body[4].kind else {
        panic!("expected if without else");
    };

    assert_eq!(**cond, expr(ExprKind::Bool(true), Type::Bool));
    assert!(then.is_empty());
    assert!(else_.is_none());
    assert_eq!(body[5].kind, StmtKind::Return(None));
}

#[test]
fn function_parameter_and_nested_statement_spans_are_preserved() {
    let program = parse(
        "fn helper(value: number) -> number { return value; }
        fn test(values: number[], ready: bool) -> void {
            let count: number = len(values);

            if ready {
                while false {
                    values[0] = helper(count);

                    if true { helper(count); } else { return; }
                }
            } else {
                let text: string = \"else\";
            }

            helper(count);

            return;
        }",
    );
    let typed = SemanticAnalyzer::new().analyze(&program).unwrap();

    assert_eq!(typed.functions.len(), program.functions.len());

    for (original, lowered) in program.functions.iter().zip(&typed.functions) {
        assert_eq!(lowered.name, original.name);
        assert_eq!(lowered.return_type, original.return_type);
        assert_eq!(lowered.span, original.span);
        assert_eq!(lowered.params.len(), original.params.len());

        for (original, lowered) in original.params.iter().zip(&lowered.params) {
            assert_eq!(lowered.name, original.name);
            assert_eq!(lowered.ty, original.ty);
            assert_eq!(lowered.span, original.span);
        }

        assert_block_spans(&original.body, &lowered.body);
    }
}

#[test]
fn reusing_analyzer_after_success_failure_or_empty_program_resets_ids() {
    let valid = parse(
        "fn first(value: number) -> number {
            let copy: number = value;

            return second(copy);
        }
        fn second(value: number) -> number { return value; }",
    );
    let expected = SemanticAnalyzer::new().analyze(&valid).unwrap();
    let mut analyzer = SemanticAnalyzer::new();

    assert_eq!(expected.functions[0].id, FunctionId(0));
    assert_eq!(expected.functions[1].id, FunctionId(1));
    assert_eq!(expected.functions[0].params[0].id, VariableId(0));
    assert_let(
        &expected.functions[0].body[0],
        1,
        "copy",
        Type::Number,
        identifier(0, "value", Type::Number),
    );
    assert_eq!(analyzer.analyze(&valid).unwrap(), expected);

    for source in [
        "fn carried() -> void {}
        fn broken(value: number) -> void {
            let copy: number = value;

            if true { let nested: number = copy; missing; }
        }",
        "fn duplicate() -> void {} fn duplicate() -> void {}",
        "fn broken(value: number, value: bool) -> void {}",
        "fn broken(value: number) -> number { let copy: number = value; }",
    ] {
        assert!(analyzer.analyze(&parse(source)).is_err(), "{source}");
        assert_eq!(
            analyzer.analyze(&valid).unwrap(),
            expected,
            "after {source}"
        );
        assert_eq!(
            analyzer
                .analyze(&Program {
                    functions: Vec::new()
                })
                .unwrap(),
            TypedProgram {
                host_functions: Vec::new(),
                functions: Vec::new()
            }
        );
        assert_eq!(
            analyzer.analyze(&valid).unwrap(),
            expected,
            "after empty program"
        );
    }
}

#[test]
fn typed_program_owns_names_literals_and_bodies_after_inputs_are_dropped() {
    let source_text =
        "fn echo(value: string) -> string { let copy: string = value; \"kept\"; return copy; }";
    let typed = {
        let source = source_text.to_owned();
        let program = parse(&source);
        let mut analyzer = SemanticAnalyzer::new();
        let typed = analyzer.analyze(&program).unwrap();

        drop(program);
        drop(analyzer);
        drop(source);

        typed
    };
    let fragment_span = |text: &str| {
        let start = source_text.find(text).expect("expected source fragment");

        Span::new(start, start + text.len())
    };

    assert_eq!(
        typed,
        TypedProgram {
            host_functions: Vec::new(),
            functions: vec![TypedFunction {
                id: FunctionId(0),
                name: "echo".into(),
                params: vec![TypedParam {
                    id: VariableId(0),
                    name: "value".into(),
                    ty: Type::String,
                    span: fragment_span("value: string"),
                }],
                return_type: Type::String,
                body: vec![
                    TypedStatement {
                        kind: StmtKind::Let {
                            id: VariableId(1),
                            name: "copy".into(),
                            ty: Type::String,
                            value: identifier(0, "value", Type::String),
                        },
                        span: fragment_span("let copy: string = value;"),
                    },
                    TypedStatement {
                        kind: StmtKind::Expression(expr(
                            ExprKind::String("kept".into()),
                            Type::String,
                        )),
                        span: fragment_span("\"kept\";"),
                    },
                    TypedStatement {
                        kind: StmtKind::Return(Some(identifier(1, "copy", Type::String))),
                        span: fragment_span("return copy;"),
                    },
                ],
                span: Span::new(0, source_text.len()),
            }],
        }
    );
}

#[test]
fn analyzing_successful_and_failing_programs_does_not_mutate_parser_ast() {
    let valid_source = "fn helper(n: number) -> number { return n; }
        fn test(value: number) -> void {
            let values: number[] = [helper(value), 1];

            if true { values[0] = value; } else { value = value + 1; }
            while false { let value: number = value; }

            helper(value);

            return;
        }";
    let invalid_source = valid_source.replace("helper(value);", "missing;");
    let mut analyzer = SemanticAnalyzer::new();

    for (source, succeeds) in [(valid_source, true), (invalid_source.as_str(), false)] {
        let program = parse(source);
        let original = parse(source);

        for _ in 0..2 {
            let result = analyzer.analyze(&program);

            assert_eq!(result.is_ok(), succeeds, "{source}: {result:?}");
            assert_eq!(program, original, "{source}");
        }
    }
}
