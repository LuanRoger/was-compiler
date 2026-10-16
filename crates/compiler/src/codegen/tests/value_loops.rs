use super::*;
use crate::{
    exports::ExportOptions,
    typed_ast::{ExprResult, TypedValueBlock},
};

fn span() -> Span {
    Span::new(12, 34)
}

fn statement(kind: TypedStatementKind) -> TypedStatement {
    TypedStatement { kind, span: span() }
}

fn expression(value: TypedExpr) -> TypedStatement {
    statement(TypedStatementKind::Expression(value))
}

fn loop_value(id: usize, body: Vec<TypedStatement>, ty: ExprResult) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(id),
            body,
        },
        ty,
    }
}

fn loop_statement(id: usize, body: Vec<TypedStatement>) -> TypedStatement {
    statement(TypedStatementKind::Loop {
        id: LoopId(id),
        body,
    })
}

fn break_value(target: usize, value: TypedExpr) -> TypedStatement {
    statement(TypedStatementKind::BreakValue {
        target: LoopId(target),
        value,
    })
}

fn bare_break(target: usize) -> TypedStatement {
    statement(TypedStatementKind::Break {
        target: LoopId(target),
    })
}

fn continue_to(target: usize) -> TypedStatement {
    statement(TypedStatementKind::Continue {
        target: LoopId(target),
    })
}

fn block(statements: Vec<TypedStatement>, tail: Option<TypedExpr>, ty: ExprResult) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Block(TypedValueBlock {
            statements,
            tail: tail.map(Box::new),
            span: span(),
        }),
        ty,
    }
}

fn array(elements: Vec<TypedExpr>) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Array(elements, crate::ast::ArrayElementType::Number),
        ty: Type::Array(crate::ast::ArrayElementType::Number).into(),
    }
}

fn string(value: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::String(value.into()),
        ty: Type::String.into(),
    }
}

#[test]
fn value_exits_use_mvp_outer_results_and_empty_backedges_without_result_locals() {
    for (ty, value, raw) in [
        (Type::Number, number(), ValType::F64),
        (Type::Bool, boolean(), ValType::I32),
        (Type::String, string("s"), ValType::I32),
        (
            Type::Array(crate::ast::ArrayElementType::Number),
            array(vec![]),
            ValType::I32,
        ),
    ] {
        let value = loop_value(usize::MAX, vec![break_value(usize::MAX, value)], ty.into());
        let program = return_program(Some(value), ty, span());
        let original = program.clone();
        let bytes = compile(&program).unwrap();
        let module = decode(&bytes);
        assert_eq!(compile(&program).unwrap(), bytes);
        assert_eq!(program, original);
        assert_eq!(
            module.bodies[0][0],
            DecodedInstruction::Block(BlockType::Type(raw))
        );
        assert_eq!(
            module.bodies[0][1],
            DecodedInstruction::Loop(BlockType::Empty)
        );
        assert!(module.bodies[0].windows(5).any(|ops| ops
            == [
                DecodedInstruction::Br(1),
                DecodedInstruction::Br(0),
                DecodedInstruction::End,
                DecodedInstruction::Unreachable,
                DecodedInstruction::End,
            ]));
        assert_eq!(
            module.bodies[0]
                .iter()
                .filter(|op| **op == DecodedInstruction::LocalSet(0))
                .count(),
            usize::from(ty == Type::Array(crate::ast::ArrayElementType::Number)),
        );
        assert_eq!(
            module.locals[0],
            if ty == Type::Array(crate::ast::ArrayElementType::Number) {
                vec![ValType::I32]
            } else {
                vec![]
            }
        );
        assert_eq!(
            module.functions.len(),
            if matches!(
                ty,
                Type::String | Type::Array(crate::ast::ArrayElementType::Number)
            ) {
                8
            } else {
                1
            }
        );
    }
}

#[test]
fn nested_value_and_statement_loops_resolve_identity_beneath_expression_regions() {
    let rhs = block(
        vec![expression(loop_value(
            999,
            vec![break_value(999, boolean())],
            Type::Bool.into(),
        ))],
        Some(number()),
        Type::Number.into(),
    );
    let value = loop_value(
        77,
        vec![conditional(
            boolean(),
            vec![break_value(77, rhs)],
            Some(vec![continue_to(77)]),
            span(),
        )],
        Type::Number.into(),
    );
    let module = decode(&compile(&return_program(Some(value), Type::Number, span())).unwrap());
    let branches: Vec<_> = module.bodies[0]
        .iter()
        .filter_map(|op| match op {
            DecodedInstruction::Br(depth) => Some(*depth),
            _ => None,
        })
        .collect();
    assert_eq!(branches, [1, 0, 2, 1, 0]);
    assert_eq!(module.locals[0], []);

    let value = loop_value(
        77,
        vec![
            loop_statement(999, vec![bare_break(999)]),
            break_value(77, number()),
        ],
        Type::Number.into(),
    );
    decode(&compile(&return_program(Some(value), Type::Number, span())).unwrap());
}

#[test]
fn reachable_conflicting_breaks_report_the_loop_span_in_source_order() {
    for (first, second, expected, found) in [
        (number(), boolean(), Type::Number, Type::Bool),
        (boolean(), string("s"), Type::Bool, Type::String),
    ] {
        let mut second = break_value(9, second);
        second.span = Span::new(50, 60);
        let value = loop_value(
            9,
            vec![conditional(
                boolean(),
                vec![break_value(9, first)],
                Some(vec![second]),
                Span::new(40, 70),
            )],
            expected.into(),
        );
        let error = compile(&local_program(vec![expression(value)])).unwrap_err();
        assert_eq!(
            error,
            CodegenError::TypeMismatch {
                function: "bad".into(),
                expected,
                found,
                span: span()
            }
        );
        assert_eq!(error.span(), Some(span()));
    }
}

#[test]
fn forged_loop_categories_are_recomputed_from_reachable_breaks() {
    for annotation in [
        ExprResult::Never,
        ExprResult::Void,
        ExprResult::Value(Type::Void),
        Type::Bool.into(),
    ] {
        let value = loop_value(9, vec![break_value(9, number())], annotation);
        let error = compile(&local_program(vec![expression(value)])).unwrap_err();
        assert!(matches!(
            error,
            CodegenError::TypeMismatch { .. } | CodegenError::ExprResultMismatch { .. }
        ));
        assert_eq!(error.span(), Some(span()));
    }
    for annotation in [
        ExprResult::Void,
        ExprResult::Value(Type::Void),
        Type::Number.into(),
    ] {
        let value = loop_value(9, vec![], annotation);
        assert_eq!(
            compile(&local_program(vec![expression(value)])),
            Err(CodegenError::ExprResultMismatch {
                function: "bad".into(),
                expected: ExprResult::Never,
                found: annotation,
                span: span(),
            })
        );
    }
    // Equal raw i32 payloads do not make different language types interchangeable.
    let value = loop_value(9, vec![break_value(9, boolean())], Type::String.into());
    assert_eq!(
        compile(&local_program(vec![expression(value)])),
        Err(CodegenError::TypeMismatch {
            function: "bad".into(),
            expected: Type::Bool,
            found: Type::String,
            span: span(),
        })
    );
}

#[test]
fn wrong_break_kinds_are_rejected_even_after_unconditional_transfers() {
    for prefix in [vec![], vec![continue_to(9)]] {
        let mut body = prefix.clone();
        body.push(bare_break(9));
        let value = loop_value(9, body, ExprResult::Never);
        let error = compile(&local_program(vec![expression(value)])).unwrap_err();
        assert_eq!(
            error,
            CodegenError::MissingBreakValue {
                function: "bad".into(),
                span: span()
            }
        );
        assert_eq!(error.span(), Some(span()));

        let mut body = prefix;
        body.push(break_value(9, number()));
        for loop_ in [
            loop_statement(9, body.clone()),
            statement(TypedStatementKind::While {
                id: LoopId(9),
                cond: Box::new(boolean()),
                body: body.clone(),
            }),
        ] {
            let error = compile(&local_program(vec![loop_])).unwrap_err();
            assert_eq!(
                error,
                CodegenError::BreakValueNotAllowed {
                    function: "bad".into(),
                    span: span()
                }
            );
            assert_eq!(error.span(), Some(span()));
        }
    }
}

#[test]
fn void_break_values_are_invalid_and_never_rhs_does_not_transfer_a_value() {
    for ty in [ExprResult::Void, ExprResult::Value(Type::Void)] {
        let value = block(vec![], None, ty);
        let loop_ = loop_value(9, vec![break_value(9, value)], ExprResult::Never);
        let error = compile(&local_program(vec![expression(loop_)])).unwrap_err();
        assert_eq!(
            error,
            CodegenError::InvalidBreakValue {
                function: "bad".into(),
                span: span()
            }
        );
        assert_eq!(error.span(), Some(span()));
    }
    let rhs = loop_value(99, vec![], ExprResult::Never);
    let value = loop_value(9, vec![break_value(9, rhs)], ExprResult::Never);
    let module = decode(&compile(&return_program(Some(value), Type::Number, span())).unwrap());
    assert!(!module.bodies[0].contains(&DecodedInstruction::Br(1)));
    assert!(!module.bodies[0].contains(&DecodedInstruction::Drop));
    assert!(module.locals[0].is_empty());

    // A Never RHS may itself exit the same loop before the syntactic outer break.
    let rhs = block(vec![break_value(9, number())], None, ExprResult::Never);
    let value = loop_value(9, vec![break_value(9, rhs)], Type::Number.into());
    decode(&compile(&return_program(Some(value), Type::Number, span())).unwrap());
}

#[test]
fn unreachable_conflicting_breaks_are_checked_without_invalid_wasm_payloads() {
    for (first, unreachable, ty) in [
        (number(), boolean(), Type::Number),
        (boolean(), number(), Type::Bool),
        (boolean(), string("unreachable"), Type::Bool),
    ] {
        let value = loop_value(
            9,
            vec![break_value(9, first), break_value(9, unreachable)],
            ty.into(),
        );
        decode(&compile(&return_program(Some(value), ty, span())).unwrap());
    }
    for prefix in [
        continue_to(9),
        loop_statement(99, vec![]),
        returned(Some(number()), span()),
    ] {
        let value = loop_value(
            9,
            vec![prefix, break_value(9, boolean()), break_value(9, number())],
            ExprResult::Never,
        );
        let module = decode(&compile(&return_program(Some(value), Type::Number, span())).unwrap());
        assert!(module.bodies[0].contains(&DecodedInstruction::I32Const(1)));
        assert!(module.bodies[0].contains(&DecodedInstruction::F64Const(1.0)));
        assert!(!module.bodies[0].contains(&DecodedInstruction::Br(1)));
    }
    // A noncompleting condition filters both arms' constraints, not their checks.
    let value = loop_value(
        9,
        vec![conditional(
            loop_value(99, vec![], ExprResult::Never),
            vec![break_value(9, number())],
            Some(vec![break_value(9, boolean())]),
            span(),
        )],
        ExprResult::Never,
    );
    decode(&compile(&return_program(Some(value), Type::Number, span())).unwrap());
}

#[test]
fn invalid_break_children_remain_checked_in_hidden_skipped_and_unreachable_code() {
    let invalid = break_value(9, number_literal("NaN"));
    let value = loop_value(
        9,
        vec![break_value(9, number()), invalid.clone()],
        Type::Number.into(),
    );
    let skipped = bool_binary(
        boolean(),
        BinaryOp::Or,
        loop_value(
            9,
            vec![invalid.clone(), break_value(9, boolean())],
            Type::Bool.into(),
        ),
    );
    for value in [value, skipped] {
        let program = local_program(vec![expression(value)]);
        for exports in [ExportOptions::All, ExportOptions::None] {
            assert_eq!(
                super::super::compile_with_exports(&program, &exports),
                Err(CodegenError::InvalidNumberLiteral {
                    function: "bad".into(),
                    literal: "NaN".into(),
                    span: span(),
                })
            );
        }
    }
    let mut forged = number();
    forged.ty = ExprResult::Never;
    let value = loop_value(
        9,
        vec![continue_to(9), break_value(9, forged)],
        ExprResult::Never,
    );
    assert_eq!(
        compile(&local_program(vec![expression(value)])),
        Err(CodegenError::ExprResultMismatch {
            function: "bad".into(),
            expected: Type::Number.into(),
            found: ExprResult::Never,
            span: span(),
        })
    );
}

#[test]
fn value_break_targets_reject_unknown_inactive_outer_and_cross_function_ids() {
    let cases = [
        vec![break_value(9, number())],
        vec![
            expression(loop_value(
                9,
                vec![break_value(9, number())],
                Type::Number.into(),
            )),
            break_value(9, number()),
        ],
        vec![expression(loop_value(
            9,
            vec![break_value(99, number())],
            Type::Number.into(),
        ))],
        vec![expression(loop_value(
            9,
            vec![expression(loop_value(
                10,
                vec![break_value(9, number())],
                Type::Number.into(),
            ))],
            ExprResult::Never,
        ))],
    ];
    for (body, target) in cases.into_iter().zip([9, 9, 99, 9]) {
        assert_eq!(
            compile(&local_program(body)),
            Err(CodegenError::InvalidLoopTarget {
                function: "bad".into(),
                target: LoopId(target),
                span: span(),
            })
        );
    }
    let mut program = local_program(vec![expression(loop_value(
        9,
        vec![break_value(9, number())],
        Type::Number.into(),
    ))]);
    let mut second = user(1, "second", &[], Type::Void);
    second.body = vec![break_value(9, number())];
    program.functions.push(second);
    assert_eq!(
        compile(&program),
        Err(CodegenError::InvalidLoopTarget {
            function: "second".into(),
            target: LoopId(9),
            span: span(),
        })
    );
}

#[test]
fn loop_ids_remain_unique_across_kinds_unreachable_nodes_and_function_local() {
    let duplicate = loop_value(usize::MAX, vec![], ExprResult::Never);
    for body in [
        vec![
            expression(duplicate.clone()),
            loop_statement(usize::MAX, vec![]),
        ],
        vec![loop_statement(
            usize::MAX,
            vec![expression(duplicate.clone())],
        )],
        vec![statement(TypedStatementKind::While {
            id: LoopId(usize::MAX),
            cond: Box::new(duplicate.clone()),
            body: vec![],
        })],
    ] {
        assert_eq!(
            compile(&local_program(body)),
            Err(CodegenError::DuplicateLoopId {
                function: "bad".into(),
                id: LoopId(usize::MAX),
                span: span(),
            })
        );
    }
    let mut program = local_program(vec![expression(duplicate.clone())]);
    let mut second = user(1, "second", &[], Type::Void);
    second.body = vec![expression(duplicate)];
    program.functions.push(second);
    decode(&compile(&program).unwrap());
}

#[test]
fn nonvoid_completion_accepts_explicit_divergence_but_not_possible_fallthrough() {
    for body in [
        vec![loop_statement(9, vec![])],
        vec![loop_statement(9, vec![continue_to(9)])],
        vec![conditional(
            boolean(),
            vec![returned(Some(number()), span())],
            Some(vec![loop_statement(9, vec![])]),
            span(),
        )],
        vec![declaration(
            99,
            Type::Number,
            loop_value(9, vec![], ExprResult::Never),
            span(),
        )],
    ] {
        let mut program = local_program(body);
        program.functions[0].return_type = Type::Number;
        let module = decode(&compile(&program).unwrap());
        assert!(!module.bodies[0].contains(&DecodedInstruction::LocalSet(0)));
        assert!(!module.bodies[0].contains(&DecodedInstruction::Drop));
    }
    for loop_ in [
        loop_statement(9, vec![bare_break(9)]),
        statement(TypedStatementKind::While {
            id: LoopId(9),
            cond: Box::new(boolean()),
            body: vec![loop_statement(99, vec![])],
        }),
    ] {
        let mut program = local_program(vec![loop_]);
        program.functions[0].return_type = Type::Number;
        assert_eq!(
            compile(&program),
            Err(CodegenError::MissingReturnValue {
                function: "bad".into(),
                return_type: Type::Number
            })
        );
    }
}

#[test]
fn noncompleting_while_conditions_consume_own_breaks_and_continues() {
    for transfer in [bare_break(9), continue_to(9), loop_statement(99, vec![])] {
        let cond = block(vec![transfer.clone()], None, ExprResult::Never);
        let while_ = statement(TypedStatementKind::While {
            id: LoopId(9),
            cond: Box::new(cond),
            body: vec![returned(Some(number()), span())],
        });
        let mut program = local_program(vec![while_]);
        program.functions[0].return_type = Type::Number;
        if matches!(transfer.kind, TypedStatementKind::Break { .. }) {
            assert_eq!(
                compile(&program),
                Err(CodegenError::MissingReturnValue {
                    function: "bad".into(),
                    return_type: Type::Number
                })
            );
            program.functions[0]
                .body
                .push(returned(Some(number()), span()));
        }
        decode(&compile(&program).unwrap());
    }
}

#[test]
fn loop_planning_keeps_declaration_preorder_scratch_and_helper_order() {
    let indexed = TypedExpr {
        kind: TypedExprKind::Index {
            expr: Box::new(identifier(
                88,
                "a",
                Type::Array(crate::ast::ArrayElementType::Number),
            )),
            index: Box::new(number_literal("0")),
        },
        ty: Type::Number.into(),
    };
    let rhs = block(
        vec![
            declaration(
                88,
                Type::Array(crate::ast::ArrayElementType::Number),
                array(vec![number()]),
                span(),
            ),
            statement(TypedStatementKind::CompoundAssignment {
                target: indexed,
                op: BinaryOp::Remainder,
                value: loop_value(99, vec![break_value(99, number())], Type::Number.into()),
            }),
            expression(string("first")),
        ],
        Some(number()),
        Type::Number.into(),
    );
    let initializer = loop_value(
        9,
        vec![
            break_value(9, rhs),
            declaration(777, Type::String, string("last"), span()),
            expression(array(vec![])),
        ],
        Type::Number.into(),
    );
    let program = local_program(vec![declaration(900, Type::Number, initializer, span())]);
    let module = decode(&compile(&program).unwrap());
    let locals = LocalLayout::new(&program.functions[0]).unwrap();
    assert_eq!(locals.get(VariableId(900)).unwrap().index.raw(), 0);
    assert_eq!(locals.get(VariableId(88)).unwrap().index.raw(), 1);
    assert_eq!(locals.get(VariableId(777)).unwrap().index.raw(), 2);
    assert_eq!(
        module.locals[0],
        [
            ValType::F64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::F64,
            ValType::I32
        ]
    );
    assert_eq!(module.functions.len(), 9);
    assert!(module.bodies[0].contains(&DecodedInstruction::Call(1))); // alloc
    assert!(module.bodies[0].contains(&DecodedInstruction::Call(4))); // checked array address
    assert!(module.bodies[0].contains(&DecodedInstruction::Call(8))); // remainder after memory helpers
    assert!(module.bodies[0].contains(&DecodedInstruction::LocalSet(0)));
    assert!(module.bodies[0].contains(&DecodedInstruction::LocalSet(5)));
    let plan = super::super::memory::MemoryPlan::new(&program)
        .unwrap()
        .unwrap();
    assert_eq!(plan.string("first"), Some(8));
    assert_eq!(plan.string("last"), Some(24));
}

#[test]
fn divergence_in_logical_and_value_if_regions_restores_polymorphism() {
    let diverge = || loop_value(99, vec![], ExprResult::Never);
    for value in [
        bool_binary(boolean(), BinaryOp::And, diverge()),
        bool_binary(boolean(), BinaryOp::Or, diverge()),
        TypedExpr {
            kind: TypedExprKind::If {
                cond: Box::new(boolean()),
                then: TypedValueBlock {
                    statements: vec![loop_statement(99, vec![])],
                    tail: None,
                    span: span(),
                },
                else_: Some(TypedValueBlock {
                    statements: vec![],
                    tail: Some(Box::new(boolean())),
                    span: span(),
                }),
            },
            ty: Type::Bool.into(),
        },
    ] {
        let value = loop_value(9, vec![break_value(9, value)], Type::Bool.into());
        decode(&compile(&return_program(Some(value), Type::Bool, span())).unwrap());
    }
    let mut eager = binary(number(), BinaryOp::Add, diverge());
    eager.ty = ExprResult::Never;
    let module = decode(&compile(&return_program(Some(eager), Type::Number, span())).unwrap());
    assert!(
        module.bodies[0]
            .windows(2)
            .any(|ops| ops == [DecodedInstruction::End, DecodedInstruction::Unreachable])
    );
}

#[test]
#[ignore = "requires Node.js"]
fn abrupt_break_and_return_rhs_unwind_live_prefixes_without_running_trailing_effects() {
    let touch = |value| {
        call(
            FunctionRef::Host(HostFunctionId(0)),
            "touch",
            vec![number_literal(value)],
            Type::Number,
        )
    };
    let mut abrupt = binary(
        touch("40"),
        BinaryOp::Add,
        block(vec![break_value(9, touch("6"))], None, ExprResult::Never),
    );
    abrupt.ty = ExprResult::Never;
    let value = loop_value(
        9,
        vec![expression(abrupt), break_value(9, touch("999"))],
        Type::Number.into(),
    );
    let mut program = return_program(
        Some(binary(touch("1"), BinaryOp::Add, value)),
        Type::Number,
        span(),
    );
    program.functions[0].name = "exit".into();
    program.host_functions = vec![host(0, "touch", &[Type::Number], Type::Number)];

    let mut return_rhs = binary(
        touch("20"),
        BinaryOp::Add,
        block(
            vec![returned(Some(touch("9")), span())],
            None,
            ExprResult::Never,
        ),
    );
    return_rhs.ty = ExprResult::Never;
    let mut returns = user(1, "returns", &[], Type::Number);
    returns.body = vec![returned(
        Some(loop_value(
            9,
            vec![break_value(9, return_rhs)],
            ExprResult::Never,
        )),
        span(),
    )];

    let mut unreachable = user(2, "unreachable", &[], Type::Number);
    unreachable.body = vec![returned(
        Some(loop_value(
            9,
            vec![
                break_value(9, touch("7")),
                break_value(9, boolean()),
                break_value(9, touch("999")),
            ],
            Type::Number.into(),
        )),
        span(),
    )];
    program.functions.extend([returns, unreachable]);
    let bytes = compile(&program).unwrap();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
const assert = require('node:assert/strict');
let effects = [];
const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
  env: { host_touch(value) { effects.push(value); return value; } },
}).exports;
assert.equal(e.exit(), 7); assert.deepEqual(effects, [1, 40, 6]);
effects = []; assert.equal(e.returns(), 9); assert.deepEqual(effects, [20, 9]);
effects = []; assert.equal(e.unreachable(), 7); assert.deepEqual(effects, [7]);
"#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn value_loops_execute_once_with_live_arithmetic_call_array_and_place_prefixes() {
    let touch = |value| {
        call(
            FunctionRef::Host(HostFunctionId(0)),
            "touch",
            vec![number_literal(value)],
            Type::Number,
        )
    };
    let exit = |id, value| loop_value(id, vec![break_value(id, touch(value))], Type::Number.into());
    let mut program = return_program(
        Some(binary(touch("10"), BinaryOp::Add, exit(9, "2"))),
        Type::Number,
        span(),
    );
    program.functions[0].name = "arithmetic".into();
    program.host_functions = vec![
        host(0, "touch", &[Type::Number], Type::Number),
        host(1, "sum", &[Type::Number, Type::Number], Type::Number),
    ];
    let mut calls = user(1, "calls", &[], Type::Number);
    calls.body = vec![returned(
        Some(call(
            FunctionRef::Host(HostFunctionId(1)),
            "sum",
            vec![touch("20"), exit(9, "3")],
            Type::Number,
        )),
        span(),
    )];
    let mut arrays = user(
        2,
        "arrays",
        &[],
        Type::Array(crate::ast::ArrayElementType::Number),
    );
    arrays.body = vec![returned(
        Some(array(vec![touch("30"), exit(9, "4")])),
        span(),
    )];
    let mut compound = user(
        3,
        "compound",
        &[Type::Array(crate::ast::ArrayElementType::Number)],
        Type::Number,
    );
    let place = TypedExpr {
        kind: TypedExprKind::Index {
            expr: Box::new(identifier(
                0,
                "p0",
                Type::Array(crate::ast::ArrayElementType::Number),
            )),
            index: Box::new(touch("0")),
        },
        ty: Type::Number.into(),
    };
    compound.body = vec![
        statement(TypedStatementKind::CompoundAssignment {
            target: place.clone(),
            op: BinaryOp::Add,
            value: exit(9, "5"),
        }),
        returned(Some(place), span()),
    ];
    program.functions.extend([calls, arrays, compound]);
    let bytes = compile(&program).unwrap();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
const assert = require('node:assert/strict');
const bytes = require('node:fs').readFileSync(0);
let effects = [];
const { exports: e } = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: {
  host_touch(value) { effects.push(value); return value; },
  host_sum(a, b) { effects.push('sum'); return a + b; },
} });
assert.equal(e.arithmetic(), 12); assert.deepEqual(effects, [10, 2]);
effects = []; assert.equal(e.calls(), 23); assert.deepEqual(effects, [20, 3, 'sum']);
effects = []; const a = e.arrays(); assert.deepEqual(effects, [30, 4]);
let view = new DataView(e.memory.buffer);
let p = view.getUint32(a, true); assert.equal(view.getFloat64(p, true), 30); assert.equal(view.getFloat64(p + 8, true), 4);
effects = []; assert.equal(e.compound(a), 35); assert.deepEqual(effects, [0, 5, 0]);
view = new DataView(e.memory.buffer); assert.equal(view.getFloat64(p, true), 35);
"#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn statement_backedges_continue_and_value_breaks_preserve_effects_and_handles() {
    let count = identifier(100, "n", Type::Number);
    let increment = statement(TypedStatementKind::CompoundAssignment {
        target: count.clone(),
        op: BinaryOp::Add,
        value: number(),
    });
    let body = vec![
        increment,
        conditional(
            bool_binary(count.clone(), BinaryOp::LessThan, number_literal("3")),
            vec![continue_to(9)],
            None,
            span(),
        ),
        break_value(9, count.clone()),
    ];
    let mut program = return_program(
        Some(loop_value(9, body, Type::Number.into())),
        Type::Number,
        span(),
    );
    program.functions[0].name = "count".into();
    program.functions[0].body.insert(
        0,
        declaration(100, Type::Number, number_literal("0"), span()),
    );
    let mut flag = user(1, "flag", &[], Type::Bool);
    flag.body = vec![returned(
        Some(loop_value(
            9,
            vec![break_value(9, boolean())],
            Type::Bool.into(),
        )),
        span(),
    )];
    let mut text = user(2, "text", &[], Type::String);
    text.body = vec![returned(
        Some(loop_value(
            9,
            vec![break_value(9, string("é"))],
            Type::String.into(),
        )),
        span(),
    )];
    let mut values = user(
        3,
        "values",
        &[],
        Type::Array(crate::ast::ArrayElementType::Number),
    );
    values.body = vec![returned(
        Some(loop_value(
            9,
            vec![break_value(9, array(vec![number_literal("7")]))],
            Type::Array(crate::ast::ArrayElementType::Number).into(),
        )),
        span(),
    )];
    let mut bare = user(4, "bare", &[], Type::Number);
    bare.body = vec![
        loop_statement(9, vec![bare_break(9)]),
        returned(Some(number_literal("8")), span()),
    ];
    let mut repeat = user(5, "repeat", &[], Type::Number);
    repeat.body = vec![
        declaration(100, Type::Number, number_literal("0"), span()),
        loop_statement(
            9,
            vec![
                statement(TypedStatementKind::CompoundAssignment {
                    target: count.clone(),
                    op: BinaryOp::Add,
                    value: number(),
                }),
                conditional(
                    bool_binary(
                        count.clone(),
                        BinaryOp::GreaterThanOrEqual,
                        number_literal("3"),
                    ),
                    vec![bare_break(9)],
                    None,
                    span(),
                ),
            ],
        ),
        returned(Some(count), span()),
    ];
    program.functions.extend([flag, text, values, bare, repeat]);
    let bytes = compile(&program).unwrap();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
const assert = require('node:assert/strict');
const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0))).exports;
assert.equal(e.count(), 3); assert.equal(e.flag(), 1); assert.equal(e.bare(), 8); assert.equal(e.repeat(), 3);
const s = e.text(); let view = new DataView(e.memory.buffer);
assert.equal(view.getUint32(s + 4, true), 2);
assert.equal(new TextDecoder().decode(new Uint8Array(e.memory.buffer, view.getUint32(s, true), 2)), 'é');
const a = e.values(); view = new DataView(e.memory.buffer);
assert.equal(view.getUint32(a + 4, true), 1); assert.equal(view.getFloat64(view.getUint32(a, true), true), 7);
"#,
    );
}
