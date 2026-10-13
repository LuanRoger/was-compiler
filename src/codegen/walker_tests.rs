use super::*;
use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    host::HostFunctionId,
    tokens::Span,
    typed_ast::{FunctionRef, LoopId, VariableId},
};

fn expr(kind: TypedExprKind) -> TypedExpr {
    TypedExpr {
        kind,
        ty: Type::Number.into(),
    }
}

fn identifier(name: &str) -> TypedExpr {
    expr(TypedExprKind::Identifier {
        id: VariableId(0),
        name: name.into(),
    })
}

fn statement(id: usize, kind: TypedStatementKind) -> TypedStatement {
    TypedStatement {
        kind,
        span: Span::new(id, id + 1),
    }
}

#[test]
fn match_patterns_are_separate_visits_in_scrutinee_pattern_body_order() {
    use crate::{
        ast::{MatchPattern, MatchPatternKind},
        typed_ast::TypedMatchArm,
    };

    struct PatternVisitor(Vec<String>);
    impl Visitor for PatternVisitor {
        type Break = ();

        fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<()> {
            if let TypedExprKind::String(value) = &expr.kind {
                self.0.push(format!("expression:{value}"));
            }
            ControlFlow::Continue(())
        }

        fn visit_pattern(&mut self, pattern: &MatchPattern) -> ControlFlow<()> {
            if let MatchPatternKind::String(value) = &pattern.kind {
                self.0.push(format!("pattern:{value}"));
            }
            ControlFlow::Continue(())
        }
    }
    let arms = || {
        vec![TypedMatchArm {
            pattern: MatchPattern {
                kind: MatchPatternKind::String("pattern".into()),
                span: Span::new(0, 1),
            },
            body: expr(TypedExprKind::String("body".into())),
            span: Span::new(0, 2),
        }]
    };
    let block = vec![
        statement(
            0,
            TypedStatementKind::Match {
                scrutinee: Box::new(expr(TypedExprKind::String("statement".into()))),
                arms: arms(),
            },
        ),
        statement(
            1,
            TypedStatementKind::Expression(expr(TypedExprKind::Match {
                scrutinee: Box::new(expr(TypedExprKind::String("value".into()))),
                arms: arms(),
            })),
        ),
    ];
    let mut visitor = PatternVisitor(vec![]);
    assert_eq!(walk_block(&block, &mut visitor), ControlFlow::Continue(()));
    assert_eq!(
        visitor.0,
        [
            "expression:statement",
            "pattern:pattern",
            "expression:body",
            "expression:value",
            "pattern:pattern",
            "expression:body"
        ]
    );
}

fn fixture() -> TypedBlock {
    vec![
        statement(
            1,
            TypedStatementKind::Let {
                id: VariableId(0),
                name: "local".into(),
                ty: Type::Number,
                value: expr(TypedExprKind::Unary {
                    op: UnaryOp::Negate,
                    expr: Box::new(identifier("initializer")),
                }),
            },
        ),
        statement(
            2,
            TypedStatementKind::Assignment {
                target: expr(TypedExprKind::Index {
                    expr: Box::new(identifier("target")),
                    index: Box::new(identifier("index")),
                }),
                value: expr(TypedExprKind::Call {
                    func: FunctionRef::Host(HostFunctionId(0)),
                    name: "call".into(),
                    args: vec![
                        expr(TypedExprKind::Array(vec![
                            expr(TypedExprKind::String("first".into())),
                            expr(TypedExprKind::Number("second".into())),
                        ])),
                        expr(TypedExprKind::Bool(false)),
                    ],
                }),
            },
        ),
        statement(
            3,
            TypedStatementKind::If {
                cond: Box::new(expr(TypedExprKind::Binary {
                    left: Box::new(identifier("left")),
                    op: BinaryOp::And,
                    right: Box::new(identifier("right")),
                })),
                then: vec![statement(
                    4,
                    TypedStatementKind::Return(Some(expr(TypedExprKind::Number("then".into())))),
                )],
                else_: Some(vec![statement(
                    5,
                    TypedStatementKind::While {
                        id: LoopId(91),
                        cond: Box::new(expr(TypedExprKind::Bool(true))),
                        body: vec![statement(
                            6,
                            TypedStatementKind::Expression(expr(TypedExprKind::Array(vec![]))),
                        )],
                    },
                )]),
            },
        ),
        statement(7, TypedStatementKind::Return(None)),
        statement(8, TypedStatementKind::Expression(identifier("unreachable"))),
        statement(
            9,
            TypedStatementKind::If {
                cond: Box::new(expr(TypedExprKind::Bool(false))),
                then: vec![],
                else_: None,
            },
        ),
    ]
}

#[derive(Default)]
struct Recorder {
    events: Vec<String>,
    stop_at: Option<usize>,
    skip_expressions: bool,
}

impl Recorder {
    fn record(&mut self, event: String) -> ControlFlow<usize> {
        self.events.push(event);

        if self.stop_at == Some(self.events.len()) {
            ControlFlow::Break(self.events.len())
        } else {
            ControlFlow::Continue(())
        }
    }
}

impl Visitor for Recorder {
    type Break = usize;

    fn enter_block(&mut self, block: &TypedBlock) -> ControlFlow<usize> {
        self.record(format!("block+{}", block.len()))
    }

    fn exit_block(&mut self, block: &TypedBlock) -> ControlFlow<usize> {
        self.record(format!("block-{}", block.len()))
    }

    fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<usize> {
        self.record(format!("statement:{}", statement.span.start()))
    }

    fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<usize> {
        let event = match &expr.kind {
            TypedExprKind::Number(value) => format!("number:{value}"),
            TypedExprKind::String(value) => format!("string:{value}"),
            TypedExprKind::Bool(value) => format!("bool:{value}"),
            TypedExprKind::Identifier { name, .. } => format!("identifier:{name}"),
            TypedExprKind::Unary { .. } => "unary".into(),
            TypedExprKind::Binary { .. } => "binary".into(),
            TypedExprKind::Range { .. } => "range".into(),
            TypedExprKind::Call { .. } => "call".into(),
            TypedExprKind::NormalizedCall(_) => "normalized-call".into(),
            TypedExprKind::Array(_) => "array".into(),
            TypedExprKind::Index { .. } => "index".into(),
            TypedExprKind::Block(_) => "value-block".into(),
            TypedExprKind::If { .. } => "value-if".into(),
            TypedExprKind::Match { .. } => "value-match".into(),
            TypedExprKind::Loop { .. } => "value-loop".into(),
        };

        self.record(event)
    }

    fn visit_expressions(&self) -> bool {
        !self.skip_expressions
    }
}

#[test]
fn range_fields_are_visited_left_to_right_including_optional_step() {
    for has_step in [false, true] {
        let block = vec![statement(
            0,
            TypedStatementKind::Expression(expr(TypedExprKind::Range {
                start: Box::new(identifier("start")),
                end: Box::new(identifier("end")),
                step: has_step.then(|| Box::new(identifier("step"))),
                inclusive: true,
            })),
        )];
        let mut recorder = Recorder::default();
        assert!(walk_block(&block, &mut recorder).is_continue());
        let mut expected = vec![
            "block+1",
            "statement:0",
            "range",
            "identifier:start",
            "identifier:end",
        ];
        if has_step {
            expected.push("identifier:step");
        }
        expected.push("block-1");
        assert_eq!(recorder.events, expected);
    }
}

fn expected() -> Vec<String> {
    [
        "block+6",
        "statement:1",
        "unary",
        "identifier:initializer",
        "statement:2",
        "index",
        "identifier:target",
        "identifier:index",
        "call",
        "array",
        "string:first",
        "number:second",
        "bool:false",
        "statement:3",
        "binary",
        "identifier:left",
        "identifier:right",
        "block+1",
        "statement:4",
        "number:then",
        "block-1",
        "block+1",
        "statement:5",
        "bool:true",
        "block+1",
        "statement:6",
        "array",
        "block-1",
        "block-1",
        "statement:7",
        "statement:8",
        "identifier:unreachable",
        "statement:9",
        "bool:false",
        "block+0",
        "block-0",
        "block-6",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn preorder_covers_every_statement_and_expression_child_without_runtime_shortcuts() {
    let block = fixture();
    let original = block.clone();
    let mut recorder = Recorder::default();

    assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
    assert_eq!(recorder.events, expected());
    assert_eq!(block, original);
}

#[test]
fn every_hook_can_abort_without_visiting_later_nodes_or_synthesizing_exits() {
    let block = fixture();
    let expected = expected();

    for stop_at in 1..=expected.len() {
        let mut recorder = Recorder {
            stop_at: Some(stop_at),
            ..Recorder::default()
        };

        assert_eq!(
            walk_block(&block, &mut recorder),
            ControlFlow::Break(stop_at)
        );
        assert_eq!(recorder.events, expected[..stop_at]);
    }
}

#[test]
fn declaration_only_walk_skips_expression_roots_and_all_their_children() {
    let mut recorder = Recorder {
        skip_expressions: true,
        ..Recorder::default()
    };

    assert_eq!(
        walk_block(&fixture(), &mut recorder),
        ControlFlow::Continue(())
    );
    assert_eq!(
        recorder.events,
        [
            "block+6",
            "statement:1",
            "statement:2",
            "statement:3",
            "block+1",
            "statement:4",
            "block-1",
            "block+1",
            "statement:5",
            "block+1",
            "statement:6",
            "block-1",
            "block-1",
            "statement:7",
            "statement:8",
            "statement:9",
            "block+0",
            "block-0",
            "block-6",
        ]
    );
}

#[test]
fn compound_walk_visits_target_then_rhs_and_respects_skip_and_break_hooks() {
    let block = vec![statement(
        1,
        TypedStatementKind::CompoundAssignment {
            target: expr(TypedExprKind::Index {
                expr: Box::new(identifier("target")),
                index: Box::new(identifier("index")),
            }),
            op: BinaryOp::Remainder,
            value: identifier("rhs"),
        },
    )];
    let expected = [
        "block+1",
        "statement:1",
        "index",
        "identifier:target",
        "identifier:index",
        "identifier:rhs",
        "block-1",
    ];
    let mut recorder = Recorder::default();
    assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
    assert_eq!(recorder.events, expected);

    for stop_at in 1..=expected.len() {
        let mut recorder = Recorder {
            stop_at: Some(stop_at),
            ..Recorder::default()
        };
        assert_eq!(
            walk_block(&block, &mut recorder),
            ControlFlow::Break(stop_at)
        );
        assert_eq!(recorder.events, expected[..stop_at]);
    }

    let mut recorder = Recorder {
        skip_expressions: true,
        ..Recorder::default()
    };
    assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
    assert_eq!(recorder.events, ["block+1", "statement:1", "block-1"]);
}

#[test]
fn loop_transfers_are_structural_leaves_without_pruning_later_children() {
    let block = vec![
        statement(1, TypedStatementKind::Break { target: LoopId(91) }),
        statement(2, TypedStatementKind::Continue { target: LoopId(91) }),
        statement(3, TypedStatementKind::Expression(identifier("after"))),
    ];

    for skip_expressions in [false, true] {
        let mut recorder = Recorder {
            skip_expressions,
            ..Recorder::default()
        };
        assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
        let mut expected = vec!["block+3", "statement:1", "statement:2", "statement:3"];
        if !skip_expressions {
            expected.push("identifier:after");
        }
        expected.push("block-3");
        assert_eq!(recorder.events, expected);
    }
}

#[test]
fn empty_blocks_still_receive_entry_and_exit_hooks() {
    let mut recorder = Recorder::default();

    assert_eq!(
        walk_block(&vec![], &mut recorder),
        ControlFlow::Continue(())
    );
    assert_eq!(recorder.events, ["block+0", "block-0"]);
}

#[test]
fn value_blocks_keep_scope_hooks_open_through_tails_and_visit_both_arms() {
    use crate::typed_ast::TypedValueBlock;

    let value_block = |id, name: &str| TypedValueBlock {
        statements: vec![statement(
            id,
            TypedStatementKind::Expression(identifier(name)),
        )],
        tail: Some(Box::new(identifier("tail"))),
        span: Span::new(0, 0),
    };
    let block = vec![statement(
        1,
        TypedStatementKind::Expression(expr(TypedExprKind::If {
            cond: Box::new(expr(TypedExprKind::Block(value_block(2, "condition")))),
            then: value_block(3, "then"),
            else_: Some(value_block(4, "else")),
        })),
    )];
    let expected = [
        "block+1",
        "statement:1",
        "value-if",
        "value-block",
        "block+1",
        "statement:2",
        "identifier:condition",
        "identifier:tail",
        "block-1",
        "block+1",
        "statement:3",
        "identifier:then",
        "identifier:tail",
        "block-1",
        "block+1",
        "statement:4",
        "identifier:else",
        "identifier:tail",
        "block-1",
        "block-1",
    ];
    let mut recorder = Recorder::default();
    assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
    assert_eq!(recorder.events, expected);

    for stop_at in 1..=expected.len() {
        let mut recorder = Recorder {
            stop_at: Some(stop_at),
            ..Recorder::default()
        };
        assert_eq!(
            walk_block(&block, &mut recorder),
            ControlFlow::Break(stop_at)
        );
        assert_eq!(recorder.events, expected[..stop_at]);
    }
}

#[test]
fn loop_bodies_and_break_values_keep_structural_preorder_and_abort_hooks() {
    let block = vec![statement(
        1,
        TypedStatementKind::Loop {
            id: LoopId(91),
            body: vec![statement(
                2,
                TypedStatementKind::BreakValue {
                    target: LoopId(91),
                    value: expr(TypedExprKind::Loop {
                        id: LoopId(92),
                        body: vec![
                            statement(3, TypedStatementKind::Continue { target: LoopId(92) }),
                            statement(
                                4,
                                TypedStatementKind::BreakValue {
                                    target: LoopId(92),
                                    value: identifier("unreachable"),
                                },
                            ),
                        ],
                    }),
                },
            )],
        },
    )];
    let expected = [
        "block+1",
        "statement:1",
        "block+1",
        "statement:2",
        "value-loop",
        "block+2",
        "statement:3",
        "statement:4",
        "identifier:unreachable",
        "block-2",
        "block-1",
        "block-1",
    ];
    let mut recorder = Recorder::default();
    assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
    assert_eq!(recorder.events, expected);

    for stop_at in 1..=expected.len() {
        let mut recorder = Recorder {
            stop_at: Some(stop_at),
            ..Recorder::default()
        };
        assert_eq!(
            walk_block(&block, &mut recorder),
            ControlFlow::Break(stop_at)
        );
        assert_eq!(recorder.events, expected[..stop_at]);
    }

    let mut recorder = Recorder {
        skip_expressions: true,
        ..Recorder::default()
    };
    assert_eq!(walk_block(&block, &mut recorder), ControlFlow::Continue(()));
    assert_eq!(
        recorder.events,
        [
            "block+1",
            "statement:1",
            "block+1",
            "statement:2",
            "block-1",
            "block-1"
        ]
    );
}

#[test]
fn visitors_can_be_reused_after_a_break() {
    let mut recorder = Recorder {
        stop_at: Some(1),
        ..Recorder::default()
    };

    assert_eq!(walk_block(&fixture(), &mut recorder), ControlFlow::Break(1));
    recorder.events.clear();
    recorder.stop_at = None;
    assert_eq!(
        walk_block(&fixture(), &mut recorder),
        ControlFlow::Continue(())
    );
    assert_eq!(recorder.events, expected());
}
