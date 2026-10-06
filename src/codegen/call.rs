use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{FunctionRef, TypedExpr},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, expression, function::Context};

pub(super) fn emit(
    reference: FunctionRef,
    args: &[TypedExpr],
    return_type: Type,
    body: &mut Function,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let function = context.name;

    if let FunctionRef::Builtin(builtin) = reference {
        return Err(CodegenError::UnsupportedBuiltin {
            function: function.into(),
            builtin,
            span,
        });
    }

    let target = context.module.resolve(reference).ok_or_else(|| {
        CodegenError::UnknownFunctionReference {
            function: function.into(),
            reference,
            span,
        }
    })?;

    if target.params.len() != args.len() {
        return Err(CodegenError::ArgumentCountMismatch {
            function: function.into(),
            reference,
            expected: target.params.len(),
            found: args.len(),
            span,
        });
    }

    expect_type(function, target.return_type, return_type, span)?;

    let is_host = matches!(reference, FunctionRef::Host(_));

    for (arg, expected) in args.iter().zip(&target.params) {
        expect_type(function, *expected, arg.ty, span)?;
        expression::emit(arg, body, context, span)?;

        if is_host && *expected == Type::Bool {
            normalize_bool(body);
        }
    }

    body.instruction(&Instruction::Call(target.function_index));

    if is_host && target.return_type == Type::Bool {
        normalize_bool(body);
    }

    Ok(())
}

fn normalize_bool(body: &mut Function) {
    body.instruction(&Instruction::I32Eqz);
    body.instruction(&Instruction::I32Eqz);
}
