use crate::typed_ast::{TypedBlock, TypedExpr, TypedExprKind, TypedStatementKind};

use super::CodegenError;

pub(super) fn count(block: &TypedBlock) -> Result<u32, CodegenError> {
    let mut count = 0;

    visit_block(block, &mut count)?;

    Ok(count)
}

fn visit_block(block: &TypedBlock, count: &mut u32) -> Result<(), CodegenError> {
    for statement in block {
        match &statement.kind {
            TypedStatementKind::Let { value, .. } | TypedStatementKind::Expression(value) => {
                visit_expr(value, count)?;
            }

            TypedStatementKind::Assignment { target, value } => {
                visit_expr(target, count)?;
                visit_expr(value, count)?;
            }

            TypedStatementKind::If { cond, then, else_ } => {
                visit_expr(cond, count)?;
                visit_block(then, count)?;

                if let Some(block) = else_ {
                    visit_block(block, count)?;
                }
            }

            TypedStatementKind::While { cond, body } => {
                visit_expr(cond, count)?;
                visit_block(body, count)?;
            }

            TypedStatementKind::Return(value) => {
                if let Some(value) = value {
                    visit_expr(value, count)?;
                }
            }
        }
    }

    Ok(())
}

fn visit_expr(expr: &TypedExpr, count: &mut u32) -> Result<(), CodegenError> {
    match &expr.kind {
        TypedExprKind::Array(elements) => {
            *count = count.checked_add(1).ok_or(CodegenError::LimitExceeded {
                what: "temporary local count",
            })?;

            for element in elements {
                visit_expr(element, count)?;
            }
        }

        TypedExprKind::Unary { expr, .. } => visit_expr(expr, count)?,

        TypedExprKind::Binary { left, right, .. } => {
            visit_expr(left, count)?;
            visit_expr(right, count)?;
        }

        TypedExprKind::Call { args, .. } => {
            for arg in args {
                visit_expr(arg, count)?;
            }
        }

        TypedExprKind::Index { expr, index } => {
            visit_expr(expr, count)?;
            visit_expr(index, count)?;
        }

        TypedExprKind::Number(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Identifier { .. } => {}
    }

    Ok(())
}
