use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{BuiltinFunction, FunctionRef, TypedExpr},
};

use super::{CodegenError, expect_type, function::BodyEmitter, memory::Helper};

pub(super) fn emit(
    reference: FunctionRef,
    args: &[TypedExpr],
    return_type: Type,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let function = body.name();

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
        body.emit_expr(&args[0], span)?;

        body.call(body.helper(Helper::ArrayLen));

        return Ok(());
    }

    let target = body.target(reference, span)?;

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
        body.emit_expr(arg, span)?;

        if is_host {
            normalize_host_value(*expected, body);
        }
    }

    body.call(target.function_index);

    if is_host {
        normalize_host_value(target.return_type, body);
    }

    Ok(())
}

fn normalize_host_value(ty: Type, body: &mut BodyEmitter<'_>) {
    match ty {
        Type::Bool => {
            body.normalize_bool();
        }

        Type::String => {
            body.call(body.helper(Helper::ValidateString));
        }

        Type::NumberArray => {
            body.call(body.helper(Helper::ValidateArray));
        }

        Type::Number | Type::Void => {}
    }
}
