use crate::{
    ast::Type,
    typed_ast::{TypedFunction, TypedStatementKind},
};
use wasm_encoder::{Function, Instruction};

use super::CodegenError;

pub(super) fn emit(function: &TypedFunction) -> Result<Function, CodegenError> {
    if let Some(statement) = function.body.first() {
        let kind = match &statement.kind {
            TypedStatementKind::Let { .. } => "let",
            TypedStatementKind::Assignment { .. } => "assignment",
            TypedStatementKind::If { .. } => "if",
            TypedStatementKind::While { .. } => "while",
            TypedStatementKind::Return(_) => "return",
            TypedStatementKind::Expression(_) => "expression",
        };

        return Err(CodegenError::UnsupportedStatement {
            function: function.name.clone(),
            kind,
            span: statement.span,
        });
    }

    if function.return_type != Type::Void {
        return Err(CodegenError::MissingReturnValue {
            function: function.name.clone(),
            return_type: function.return_type,
        });
    }

    let mut body = Function::new([]);

    body.instruction(&Instruction::End);

    Ok(body)
}
