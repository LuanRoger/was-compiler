use std::ops::ControlFlow;

use crate::typed_ast::{TypedBlock, TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind};

pub(super) trait Visitor {
    type Break;

    fn enter_block(&mut self, _block: &TypedBlock) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn exit_block(&mut self, _block: &TypedBlock) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn enter_statement(&mut self, _statement: &TypedStatement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn enter_expression(&mut self, _expr: &TypedExpr) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn visit_expressions(&self) -> bool {
        true
    }
}

pub(super) fn walk_block<V: Visitor>(block: &TypedBlock, visitor: &mut V) -> ControlFlow<V::Break> {
    visitor.enter_block(block)?;

    for statement in block {
        visitor.enter_statement(statement)?;

        match &statement.kind {
            TypedStatementKind::Let { value, .. } | TypedStatementKind::Expression(value) => {
                walk_expression(value, visitor)?;
            }

            TypedStatementKind::Assignment { target, value } => {
                walk_expression(target, visitor)?;
                walk_expression(value, visitor)?;
            }

            TypedStatementKind::If { cond, then, else_ } => {
                walk_expression(cond, visitor)?;
                walk_block(then, visitor)?;

                if let Some(block) = else_ {
                    walk_block(block, visitor)?;
                }
            }

            TypedStatementKind::While { cond, body } => {
                walk_expression(cond, visitor)?;
                walk_block(body, visitor)?;
            }

            TypedStatementKind::Return(value) => {
                if let Some(value) = value {
                    walk_expression(value, visitor)?;
                }
            }
        }
    }

    visitor.exit_block(block)
}

fn walk_expression<V: Visitor>(expr: &TypedExpr, visitor: &mut V) -> ControlFlow<V::Break> {
    if !visitor.visit_expressions() {
        return ControlFlow::Continue(());
    }

    visitor.enter_expression(expr)?;

    match &expr.kind {
        TypedExprKind::Unary { expr, .. } => walk_expression(expr, visitor)?,

        TypedExprKind::Binary { left, right, .. } => {
            walk_expression(left, visitor)?;
            walk_expression(right, visitor)?;
        }

        TypedExprKind::Call { args, .. } => {
            for arg in args {
                walk_expression(arg, visitor)?;
            }
        }

        TypedExprKind::Array(elements) => {
            for element in elements {
                walk_expression(element, visitor)?;
            }
        }

        TypedExprKind::Index { expr, index } => {
            walk_expression(expr, visitor)?;
            walk_expression(index, visitor)?;
        }

        TypedExprKind::Number(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Identifier { .. } => {}
    }

    ControlFlow::Continue(())
}

#[cfg(test)]
#[path = "walker_tests.rs"]
mod tests;
