use crate::{ast::Type, typed_ast::TypedFunction};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, statement};

pub(super) fn emit(function: &TypedFunction) -> Result<Function, CodegenError> {
    let mut body = Function::new([]);
    let mut always_returns = false;

    for statement in &function.body {
        let returns = statement::emit(statement, &mut body, function)?;

        always_returns |= returns;
    }

    if function.return_type != Type::Void && !always_returns {
        return Err(CodegenError::MissingReturnValue {
            function: function.name.clone(),
            return_type: function.return_type,
        });
    }

    body.instruction(&Instruction::End);

    Ok(body)
}
