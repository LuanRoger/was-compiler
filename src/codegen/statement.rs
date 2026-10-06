use crate::{
    ast::Type,
    typed_ast::{TypedExprKind, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, expression, function::Context};

pub(super) fn emit(
    statement: &TypedStatement,
    body: &mut Function,
    context: &mut Context<'_>,
) -> Result<bool, CodegenError> {
    let function = context.name;
    let span = statement.span;

    let kind = match &statement.kind {
        TypedStatementKind::Let { id, ty, value, .. } => {
            let index = context.declaration(*id, *ty, span)?;

            expect_type(function, *ty, value.ty, span)?;
            expression::emit(value, body, context, span)?;

            body.instruction(&Instruction::LocalSet(index));

            context.initialize(*id);

            return Ok(false);
        }

        TypedStatementKind::Assignment { target, value } => {
            let TypedExprKind::Identifier { id, .. } = &target.kind else {
                return Err(CodegenError::InvalidAssignmentTarget {
                    function: function.into(),
                    span,
                });
            };

            let index = context.resolve(*id, target.ty, span)?;

            expect_type(function, target.ty, value.ty, span)?;
            expression::emit(value, body, context, span)?;

            body.instruction(&Instruction::LocalSet(index));

            return Ok(false);
        }

        TypedStatementKind::Expression(expr) => {
            expression::emit(expr, body, context, span)?;

            if expr.ty != Type::Void {
                body.instruction(&Instruction::Drop);
            }

            return Ok(false);
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

            return Ok(true);
        }

        TypedStatementKind::If { .. } => "if",
        TypedStatementKind::While { .. } => "while",
    };

    Err(CodegenError::UnsupportedStatement {
        function: function.into(),
        kind,
        span,
    })
}
