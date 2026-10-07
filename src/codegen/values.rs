use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{Function, Instruction, MemArg};

use super::{CodegenError, expect_type, expression, function::Context, memory::Helper};

pub(super) fn emit_array(
    elements: &[TypedExpr],
    body: &mut Function,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let limit = || CodegenError::LimitExceeded {
        what: "array allocation size",
    };
    let length = u32::try_from(elements.len()).map_err(|_| limit())?;
    let size = length
        .checked_mul(8)
        .and_then(|payload| payload.checked_add(8))
        .filter(|size| u64::from(*size) <= super::memory::MAX_BYTES)
        .ok_or_else(limit)?;
    let temporary = context.temporary();

    body.instruction(&Instruction::I32Const(size as i32));
    body.instruction(&Instruction::Call(context.helper(Helper::Alloc)));
    body.instruction(&Instruction::LocalSet(temporary));

    body.instruction(&Instruction::LocalGet(temporary));
    body.instruction(&Instruction::LocalGet(temporary));
    body.instruction(&Instruction::I32Const(8));
    body.instruction(&Instruction::I32Add);
    body.instruction(&Instruction::I32Store(memory_arg(0, 2)));

    body.instruction(&Instruction::LocalGet(temporary));
    body.instruction(&Instruction::I32Const(length as i32));
    body.instruction(&Instruction::I32Store(memory_arg(4, 2)));

    for (index, element) in (0..length).zip(elements) {
        expect_type(context.name, Type::Number, element.ty, span)?;

        body.instruction(&Instruction::LocalGet(temporary));

        expression::emit(element, body, context, span)?;

        body.instruction(&Instruction::F64Store(memory_arg(
            8 + u64::from(index) * 8,
            3,
        )));
    }

    body.instruction(&Instruction::LocalGet(temporary));

    Ok(())
}

pub(super) fn expect_index_types(
    base: &TypedExpr,
    index: &TypedExpr,
    result_type: Type,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    expect_type(context.name, Type::Number, result_type, span)?;
    expect_type(context.name, Type::NumberArray, base.ty, span)?;
    expect_type(context.name, Type::Number, index.ty, span)
}

pub(super) fn emit_address(
    base: &TypedExpr,
    index: &TypedExpr,
    body: &mut Function,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    expression::emit(base, body, context, span)?;
    expression::emit(index, body, context, span)?;

    body.instruction(&Instruction::Call(context.helper(Helper::ArrayAddress)));

    Ok(())
}

pub(super) fn emit_assignment(
    target: &TypedExpr,
    value: &TypedExpr,
    body: &mut Function,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    match &target.kind {
        TypedExprKind::Identifier { id, .. } => {
            let index = context.resolve(*id, target.ty, span)?;

            expect_type(context.name, target.ty, value.ty, span)?;
            expression::emit(value, body, context, span)?;

            body.instruction(&Instruction::LocalSet(index));
        }

        TypedExprKind::Index { expr: base, index } => {
            let TypedExprKind::Identifier { id, .. } = &base.kind else {
                return Err(CodegenError::InvalidAssignmentTarget {
                    function: context.name.into(),
                    span,
                });
            };

            expect_index_types(base, index, target.ty, context, span)?;
            context.resolve(*id, base.ty, span)?;
            expect_type(context.name, Type::Number, value.ty, span)?;

            emit_address(base, index, body, context, span)?;
            expression::emit(value, body, context, span)?;

            body.instruction(&Instruction::F64Store(memory_arg(0, 3)));
        }

        _ => {
            return Err(CodegenError::InvalidAssignmentTarget {
                function: context.name.into(),
                span,
            });
        }
    }

    Ok(())
}

fn memory_arg(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}
