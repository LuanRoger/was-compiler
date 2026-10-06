use crate::{
    ast::Type,
    typed_ast::{TypedFunction, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, expression};

pub(super) fn emit(
    statement: &TypedStatement,
    body: &mut Function,
    function: &TypedFunction,
) -> Result<bool, CodegenError> {
    let span = statement.span;

    let kind = match &statement.kind {
        TypedStatementKind::Return(value) => {
            match value {
                Some(_) if function.return_type == Type::Void => {
                    return Err(CodegenError::UnexpectedReturnValue {
                        function: function.name.clone(),
                        span,
                    });
                }

                Some(expr) => {
                    expect_type(&function.name, function.return_type, expr.ty, span)?;
                    expression::emit(expr, body, &function.name, span)?;
                }

                None => {
                    expect_type(&function.name, function.return_type, Type::Void, span)?;
                }
            }

            body.instruction(&Instruction::Return);

            return Ok(true);
        }

        TypedStatementKind::Let { .. } => "let",
        TypedStatementKind::Assignment { .. } => "assignment",
        TypedStatementKind::If { .. } => "if",
        TypedStatementKind::While { .. } => "while",
        TypedStatementKind::Expression(_) => "expression",
    };

    Err(CodegenError::UnsupportedStatement {
        function: function.name.clone(),
        kind,
        span,
    })
}
