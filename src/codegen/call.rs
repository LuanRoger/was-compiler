use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{BuiltinFunction, FunctionRef, TypedExpr},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, expression, function::Context, memory::Helper};

pub(super) fn emit(
    reference: FunctionRef,
    args: &[TypedExpr],
    return_type: Type,
    body: &mut Function,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let function = context.name;

    if let FunctionRef::Builtin(BuiltinFunction::Len) = reference {
        if args.len() != 1 {
            return Err(CodegenError::ArgumentCountMismatch {
                function: function.into(),
                reference,
                expected: 1,
                found: args.len(),
                span,
            });
        }

        expect_type(function, Type::Number, return_type, span)?;
        expect_type(function, Type::NumberArray, args[0].ty, span)?;
        expression::emit(&args[0], body, context, span)?;

        body.instruction(&Instruction::Call(context.helper(Helper::ArrayLen)));

        return Ok(());
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

        if is_host {
            normalize_host_value(*expected, body, context);
        }
    }

    body.instruction(&Instruction::Call(target.function_index));

    if is_host {
        normalize_host_value(target.return_type, body, context);
    }

    Ok(())
}

fn normalize_host_value(ty: Type, body: &mut Function, context: &Context<'_>) {
    match ty {
        Type::Bool => {
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::I32Eqz);
        }

        Type::String => {
            body.instruction(&Instruction::Call(context.helper(Helper::ValidateString)));
        }

        Type::NumberArray => {
            body.instruction(&Instruction::Call(context.helper(Helper::ValidateArray)));
        }

        Type::Number | Type::Void => {}
    }
}
