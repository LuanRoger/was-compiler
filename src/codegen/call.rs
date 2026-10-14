use std::collections::{HashMap, HashSet};

use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, CallBindingOrigin, ExprResult, FunctionRef, TypedExpr, TypedExprKind,
        TypedNormalizedCall,
    },
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

pub(super) fn emit_normalized(
    sequence: &TypedNormalizedCall,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(Flow, ExprResult), CodegenError> {
    let invalid = |reason| CodegenError::InvalidNormalizedCall { reason, span };
    let TypedExprKind::Call {
        func: reference @ FunctionRef::User(_),
        name,
        args,
    } = &sequence.call.kind
    else {
        return Err(invalid("final expression must be a direct user call"));
    };
    let target = body.target(*reference, span)?;
    if name != &target.name {
        return Err(invalid("final call name does not match its target"));
    }
    if sequence.call.ty != ExprResult::from(target.return_type) {
        return Err(invalid("final call result does not match its signature"));
    }
    if sequence.bindings.len() != target.params.len()
        || sequence.formal_to_binding.len() != target.params.len()
        || args.len() != target.params.len()
    {
        return Err(invalid(
            "binding, mapping and argument counts must match the signature",
        ));
    }

    let mut bindings = HashMap::with_capacity(sequence.bindings.len());
    let mut explicit_count = 0;
    let mut previous_default = None;
    for (position, binding) in sequence.bindings.iter().enumerate() {
        let reason = match binding.origin {
            CallBindingOrigin::PipelineInput => {
                if position != 0 {
                    Some("pipeline input must be the first and only injected binding")
                } else if sequence.formal_to_binding.first() != Some(&binding.id) {
                    Some("pipeline input must map to formal zero")
                } else {
                    None
                }
            }
            CallBindingOrigin::Explicit { argument }
                if previous_default.is_none() && argument == explicit_count =>
            {
                explicit_count += 1;
                None
            }
            CallBindingOrigin::Explicit { .. } => {
                Some("bindings must have contiguous written-order explicit origins")
            }
            CallBindingOrigin::Default { formal } => {
                if sequence.formal_to_binding.get(formal) != Some(&binding.id) {
                    Some("default origin must match its formal mapping")
                } else if previous_default.is_some_and(|previous| formal <= previous) {
                    Some("defaults must follow explicit arguments in increasing formal order")
                } else {
                    previous_default = Some(formal);
                    None
                }
            }
        };
        if let Some(reason) = reason {
            return Err(CodegenError::InvalidNormalizedCall {
                reason,
                span: binding.span,
            });
        }
        if bindings.insert(binding.id, binding).is_some() {
            return Err(invalid("binding identities must be unique"));
        }
    }
    let mut mapped = HashSet::with_capacity(sequence.bindings.len());
    for ((id, arg), formal) in sequence
        .formal_to_binding
        .iter()
        .zip(args)
        .zip(&target.params)
    {
        let Some(binding) = bindings.get(id) else {
            return Err(invalid("formal mapping must own every referenced binding"));
        };
        if !mapped.insert(*id) {
            return Err(invalid("formal mapping must be a bijection"));
        }
        if binding.ty != *formal || arg.ty != ExprResult::from(*formal) {
            return Err(invalid(
                "mapped binding and argument types must match the formal",
            ));
        }
        if !matches!(&arg.kind, TypedExprKind::Identifier { id: actual, .. } if actual == id) {
            return Err(invalid(
                "final arguments must read owned bindings in formal order",
            ));
        }
    }

    body.scoped(|body| {
        let mut flow = Flow::FALLTHROUGH;
        for (position, binding) in sequence.bindings.iter().enumerate() {
            if let CallBindingOrigin::Default { formal } = binding.origin {
                check_default(
                    &binding.value,
                    &sequence.formal_to_binding[..formal],
                    binding.span,
                )?;
                // Explicit values are already captured, but only defaults may read
                // them. The whitelist further limits reads to earlier formals.
                for previous in &sequence.bindings[..position] {
                    body.initialize(previous.id);
                }
            }
            expect_operand(body.name(), binding.ty, binding.value.ty, binding.span)?;
            let slot = body.declaration(binding.id, binding.ty, binding.span)?;
            flow = flow.then(body.emit_expr(&binding.value, binding.span)?);
            if binding.value.ty != ExprResult::Never {
                body.local_set(slot);
            }
        }
        // Explicit initializers remain outside generated-binding scope. Defaults
        // grant only checked earlier-formal reads; the final call reads all slots.
        for binding in &sequence.bindings {
            body.initialize(binding.id);
        }
        flow = flow.then(body.emit_expr(&sequence.call, span)?);
        let result = if flow.can_fall_through() {
            target.return_type.into()
        } else {
            ExprResult::Never
        };
        Ok((flow, result))
    })
}

fn check_default(
    value: &TypedExpr,
    earlier: &[crate::typed_ast::VariableId],
    span: Span,
) -> Result<(), CodegenError> {
    super::pure::check(value, earlier).map_err(|violation| {
        let reason = match violation {
            super::pure::Violation::Read => "defaults may only read owned earlier-formal bindings",
            super::pure::Violation::Membership => "range membership is not allowed in defaults",
            super::pure::Violation::Expression => {
                "default initializer is outside the pure-expression whitelist"
            }
        };
        CodegenError::InvalidNormalizedCall { reason, span }
    })
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
