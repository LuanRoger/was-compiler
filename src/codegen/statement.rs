use crate::{
    ast::Type,
    typed_ast::{TypedBlock, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, control_flow, expect_type, expression, function::Context, values};

pub(super) fn emit_block(
    block: &TypedBlock,
    body: &mut Function,
    context: &mut Context<'_>,
) -> Result<bool, CodegenError> {
    let mut always_returns = false;

    for statement in block {
        let returns = emit(statement, body, context)?;

        always_returns |= returns;
    }

    Ok(always_returns)
}

pub(super) fn emit(
    statement: &TypedStatement,
    body: &mut Function,
    context: &mut Context<'_>,
) -> Result<bool, CodegenError> {
    let function = context.name;
    let span = statement.span;

    match &statement.kind {
        TypedStatementKind::Let { id, ty, value, .. } => {
            let index = context.declaration(*id, *ty, span)?;

            expect_type(function, *ty, value.ty, span)?;
            expression::emit(value, body, context, span)?;

            body.instruction(&Instruction::LocalSet(index));

            context.initialize(*id);

            Ok(false)
        }

        TypedStatementKind::Assignment { target, value } => {
            values::emit_assignment(target, value, body, context, span)?;

            Ok(false)
        }

        TypedStatementKind::Expression(expr) => {
            expression::emit(expr, body, context, span)?;

            if expr.ty != Type::Void {
                body.instruction(&Instruction::Drop);
            }

            Ok(false)
        }

        TypedStatementKind::Return(value) => {
            match value {
                Some(_) if context.return_type == Type::Void => {
                    return Err(CodegenError::UnexpectedReturnValue {
                        function: function.into(),
                        span,
                    });
                }

                Some(expr) => {
                    expect_type(function, context.return_type, expr.ty, span)?;
                    expression::emit(expr, body, context, span)?;
                }

                None => {
                    expect_type(function, context.return_type, Type::Void, span)?;
                }
            }

            body.instruction(&Instruction::Return);

            Ok(true)
        }

        TypedStatementKind::If { cond, then, else_ } => {
            control_flow::emit_if(cond, then, else_.as_ref(), body, context, span)
        }

        TypedStatementKind::While { cond, body: block } => {
            control_flow::emit_while(cond, block, body, context, span)
        }
    }
}
