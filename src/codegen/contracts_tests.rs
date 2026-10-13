use std::ops::ControlFlow;

use crate::{
    ast::{BinaryOp, Type},
    compiler::Compiler,
    tokens::Span,
    typed_ast::{
        TypedContract, TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind,
        TypedValueBlock, VariableId,
    },
};
use wasm_encoder::ValType;

use super::{
    locals::LocalLayout,
    memory::MemoryPlan,
    numeric, range, temporaries,
    walker::{self, Visitor},
};

fn expr(kind: TypedExprKind, ty: Type) -> TypedExpr {
    TypedExpr {
        kind,
        ty: ty.into(),
    }
}

#[test]
fn independent_planners_visit_even_forbidden_contract_structure_before_body() {
    let mut program = Compiler::new()
        .analyze("fn target() -> void { let body = [2]; \"body\"; }")
        .unwrap();
    let span = Span::new(70, 80);
    let number = || expr(TypedExprKind::Number("1".into()), Type::Number);
    let remainder = expr(
        TypedExprKind::Binary {
            left: Box::new(number()),
            op: BinaryOp::Remainder,
            right: Box::new(number()),
        },
        Type::Number,
    );
    // Planning is structural, not a second semantic or whitelist validator.
    program.functions[0].contracts = vec![TypedContract {
        value: expr(
            TypedExprKind::Block(TypedValueBlock {
                statements: vec![
                    TypedStatement {
                        kind: TypedStatementKind::Let {
                            id: VariableId(99),
                            name: "contract_local".into(),
                            ty: Type::NumberArray,
                            value: expr(TypedExprKind::Array(vec![number()]), Type::NumberArray),
                        },
                        span,
                    },
                    TypedStatement {
                        kind: TypedStatementKind::Expression(expr(
                            TypedExprKind::String("entry".into()),
                            Type::String,
                        )),
                        span,
                    },
                ],
                tail: Some(Box::new(expr(
                    TypedExprKind::Range {
                        start: Box::new(remainder),
                        end: Box::new(number()),
                        step: None,
                        inclusive: false,
                    },
                    Type::Range,
                ))),
                span,
            }),
            Type::Range,
        ),
        span,
    }];
    let locals = LocalLayout::new(&program.functions[0]).unwrap();
    assert_eq!(locals.get(VariableId(99)).unwrap().index.raw(), 0);
    assert_eq!(locals.get(VariableId(0)).unwrap().index.raw(), 1);
    assert_eq!(locals.types, [ValType::I32, ValType::I32]);
    assert_eq!(
        temporaries::plan(&program.functions[0]).unwrap(),
        [ValType::I32, ValType::I32]
    );
    let memory = MemoryPlan::new(&program).unwrap().unwrap();
    assert_eq!(memory.string("entry"), Some(8));
    assert_eq!(memory.string("body"), Some(24));
    assert!(numeric::needs_remainder(&program));
    assert!(matches!(
        range::helpers(&program),
        [range::Helper::Construct, range::Helper::Validate]
    ));
    assert!(
        matches!(super::compile(&program), Err(super::CodegenError::InvalidContractExpression { span: actual, .. }) if actual == span)
    );

    program.functions[0].contracts[0].value = expr(
        TypedExprKind::Binary {
            left: Box::new(number()),
            op: BinaryOp::In,
            right: Box::new(expr(
                TypedExprKind::Identifier {
                    id: VariableId(99),
                    name: "forged".into(),
                },
                Type::Range,
            )),
        },
        Type::Bool,
    );
    assert!(matches!(
        range::helpers(&program),
        [
            range::Helper::Construct,
            range::Helper::Validate,
            range::Helper::Contains
        ]
    ));
}

#[test]
fn function_walk_is_ordered_and_breaks_before_later_contracts_and_body_hooks() {
    struct Recorder {
        values: Vec<String>,
        blocks: usize,
        stop: bool,
    }
    impl Visitor for Recorder {
        type Break = ();
        fn enter_expression(&mut self, value: &TypedExpr) -> ControlFlow<()> {
            if let TypedExprKind::String(value) = &value.kind {
                self.values.push(value.clone());
                if self.stop {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        }
        fn enter_block(&mut self, _: &crate::typed_ast::TypedBlock) -> ControlFlow<()> {
            self.blocks += 1;
            ControlFlow::Continue(())
        }
    }
    let mut program = Compiler::new()
        .analyze("fn target() -> void { \"body\"; }")
        .unwrap();
    program.functions[0].contracts = ["first", "second"]
        .into_iter()
        .map(|text| TypedContract {
            value: expr(TypedExprKind::String(text.into()), Type::String),
            span: Span::new(0, 0),
        })
        .collect();
    let mut recorder = Recorder {
        values: vec![],
        blocks: 0,
        stop: false,
    };
    assert_eq!(
        walker::walk_function(&program.functions[0], &mut recorder),
        ControlFlow::Continue(())
    );
    assert_eq!(recorder.values, ["first", "second", "body"]);
    assert_eq!(recorder.blocks, 1);
    let mut recorder = Recorder {
        values: vec![],
        blocks: 0,
        stop: true,
    };
    assert_eq!(
        walker::walk_function(&program.functions[0], &mut recorder),
        ControlFlow::Break(())
    );
    assert_eq!(recorder.values, ["first"]);
    assert_eq!(recorder.blocks, 0);
}
