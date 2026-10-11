use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{BuiltinFunction, ExprResult, FunctionRef, TypedExpr},
};

use super::{CodegenError, expect_operand, expect_result, function::BodyEmitter, memory::Helper};

pub(super) fn emit(
    reference: FunctionRef,
    args: &[TypedExpr],
    annotation: ExprResult,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(Flow, Type), CodegenError> {
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

        expect_result(
            function,
            predicted_result(Type::Number, args),
            annotation,
            span,
        )?;
        expect_operand(function, Type::NumberArray, args[0].ty, span)?;
        let flow = body.emit_expr(&args[0], span)?;

        body.call(body.helper(Helper::ArrayLen));

        return Ok((flow, Type::Number));
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

    expect_result(
        function,
        predicted_result(target.return_type, args),
        annotation,
        span,
    )?;

    let is_host = matches!(reference, FunctionRef::Host(_));

    let mut flow = Flow::FALLTHROUGH;

    for (arg, expected) in args.iter().zip(&target.params) {
        expect_operand(function, *expected, arg.ty, span)?;
        flow = flow.then(body.emit_expr(arg, span)?);

        if is_host {
            normalize_host_value(*expected, body);
        }
    }

    body.call(target.function_index);

    if is_host {
        normalize_host_value(target.return_type, body);
    }

    Ok((flow, target.return_type))
}

fn predicted_result(ty: Type, args: &[TypedExpr]) -> ExprResult {
    if args.iter().any(|arg| arg.ty == ExprResult::Never) {
        ExprResult::Never
    } else {
        ty.into()
    }
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

        Type::Range => unreachable!("range host signatures rejected during layout"),

        Type::Number | Type::Void => {}
    }
}
