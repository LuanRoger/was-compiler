use super::*;
use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    host::HostFunctionId,
    tokens::Span,
    typed_ast::{FunctionRef, VariableId},
};

fn expr(kind: TypedExprKind) -> TypedExpr {
    TypedExpr {
        kind,
        ty: Type::Number,
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
            TypedExprKind::Call { .. } => "call".into(),
            TypedExprKind::Array(_) => "array".into(),
            TypedExprKind::Index { .. } => "index".into(),
        };

        self.record(event)
    }

    fn visit_expressions(&self) -> bool {
        !self.skip_expressions
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
fn empty_blocks_still_receive_entry_and_exit_hooks() {
    let mut recorder = Recorder::default();

    assert_eq!(
        walk_block(&vec![], &mut recorder),
        ControlFlow::Continue(())
    );
    assert_eq!(recorder.events, ["block+0", "block-0"]);
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
