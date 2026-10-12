use std::ops::ControlFlow;

use crate::typed_ast::{
    TypedBlock, TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind, TypedValueBlock,
};

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
    walk_statements(block, visitor)?;
    visitor.exit_block(block)
}

fn walk_statements<V: Visitor>(block: &TypedBlock, visitor: &mut V) -> ControlFlow<V::Break> {
    for statement in block {
        visitor.enter_statement(statement)?;

        match &statement.kind {
            TypedStatementKind::Let { value, .. }
            | TypedStatementKind::BreakValue { value, .. }
            | TypedStatementKind::Expression(value) => {
                walk_expression(value, visitor)?;
            }

            TypedStatementKind::Assignment { target, value }
            | TypedStatementKind::CompoundAssignment { target, value, .. } => {
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

            TypedStatementKind::While { cond, body, .. } => {
                walk_expression(cond, visitor)?;
                walk_block(body, visitor)?;
            }

            TypedStatementKind::For { iterable, body, .. } => {
                walk_expression(iterable, visitor)?;
                walk_block(body, visitor)?;
            }

            TypedStatementKind::Loop { body, .. } => walk_block(body, visitor)?,

            TypedStatementKind::Break { .. } | TypedStatementKind::Continue { .. } => {}

            TypedStatementKind::Return(value) => {
                if let Some(value) = value {
                    walk_expression(value, visitor)?;
                }
            }
        }
    }

    ControlFlow::Continue(())
}

fn walk_value_block<V: Visitor>(block: &TypedValueBlock, visitor: &mut V) -> ControlFlow<V::Break> {
    visitor.enter_block(&block.statements)?;
    walk_statements(&block.statements, visitor)?;

    if let Some(tail) = &block.tail {
        walk_expression(tail, visitor)?;
    }

    visitor.exit_block(&block.statements)
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

        TypedExprKind::Range {
            start, end, step, ..
        } => {
            walk_expression(start, visitor)?;
            walk_expression(end, visitor)?;
            if let Some(step) = step {
                walk_expression(step, visitor)?;
            }
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

        TypedExprKind::Loop { body, .. } => walk_block(body, visitor)?,

        TypedExprKind::Block(block) => walk_value_block(block, visitor)?,

        TypedExprKind::If { cond, then, else_ } => {
            walk_expression(cond, visitor)?;
            walk_value_block(then, visitor)?;

            if let Some(block) = else_ {
                walk_value_block(block, visitor)?;
            }
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
