use crate::{
    ast::BinaryOp,
    typed_ast::{BuiltinFunction, FunctionRef, TypedExpr, TypedExprKind, VariableId},
};

pub(super) enum Violation {
    Read,
    Membership,
    Expression,
}

// This checks admission only. Ordinary checked emission still validates types,
// literal syntax, builtin arity and every admitted child.
pub(super) fn check(value: &TypedExpr, readable: &[VariableId]) -> Result<(), Violation> {
    match &value.kind {
        TypedExprKind::Number(_) | TypedExprKind::String(_) | TypedExprKind::Bool(_) => {}
        TypedExprKind::Identifier { id, .. } => {
            if !readable.contains(id) {
                return Err(Violation::Read);
            }
        }
        TypedExprKind::Unary { expr, .. } => check(expr, readable)?,
        TypedExprKind::Binary { left, op, right } => {
            if *op == BinaryOp::In {
                return Err(Violation::Membership);
            }
            check(left, readable)?;
            check(right, readable)?;
        }
        TypedExprKind::Call {
            func: FunctionRef::Builtin(BuiltinFunction::Len),
            args,
            ..
        } => {
            for arg in args {
                check(arg, readable)?;
            }
        }
        TypedExprKind::Call { .. }
        | TypedExprKind::LexicalCall { .. }
        | TypedExprKind::NormalizedCall(_)
        | TypedExprKind::Range { .. }
        | TypedExprKind::Loop { .. }
        | TypedExprKind::Block(_)
        | TypedExprKind::If { .. }
        | TypedExprKind::Match { .. }
        | TypedExprKind::Array(_)
        | TypedExprKind::Index { .. } => return Err(Violation::Expression),
    }
    Ok(())
}
