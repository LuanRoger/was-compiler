use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{Instruction, MemArg};

use super::{CodegenError, abi, expect_type, function::BodyEmitter, memory::Helper};

pub(super) fn emit_array(
    elements: &[TypedExpr],
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let limit = || CodegenError::LimitExceeded {
        what: "array allocation size",
    };
    let length = u32::try_from(elements.len()).map_err(|_| limit())?;
    let size = length
        .checked_mul(abi::NUMBER_ELEMENT_SIZE)
        .and_then(|payload| payload.checked_add(abi::DESCRIPTOR_SIZE))
        .filter(|size| u64::from(*size) <= super::memory::MAX_BYTES)
        .ok_or_else(limit)?;
    let temporary = body.temporary();

    body.instruction(&Instruction::I32Const(size as i32));
    body.call(body.helper(Helper::Alloc));
    body.local_set(temporary);

    body.local_get(temporary);
    body.local_get(temporary);
    body.instruction(&Instruction::I32Const(abi::DESCRIPTOR_SIZE as i32));
    body.instruction(&Instruction::I32Add);
    body.instruction(&Instruction::I32Store(memory_arg(
        abi::DESCRIPTOR_DATA_OFFSET,
        abi::DESCRIPTOR_FIELD_ALIGN_LOG2,
    )));

    body.local_get(temporary);
    body.instruction(&Instruction::I32Const(length as i32));
    body.instruction(&Instruction::I32Store(memory_arg(
        abi::DESCRIPTOR_LENGTH_OFFSET,
        abi::DESCRIPTOR_FIELD_ALIGN_LOG2,
    )));

    for (index, element) in (0..length).zip(elements) {
        expect_type(body.name(), Type::Number, element.ty, span)?;

        body.local_get(temporary);

        body.emit_expr(element, span)?;

        body.instruction(&Instruction::F64Store(memory_arg(
            u64::from(abi::DESCRIPTOR_SIZE)
                + u64::from(index) * u64::from(abi::NUMBER_ELEMENT_SIZE),
            abi::NUMBER_ELEMENT_ALIGN_LOG2,
        )));
    }

    body.local_get(temporary);

    Ok(())
}

pub(super) fn expect_index_types(
    base: &TypedExpr,
    index: &TypedExpr,
    result_type: Type,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    expect_type(body.name(), Type::Number, result_type, span)?;
    expect_type(body.name(), Type::NumberArray, base.ty, span)?;
    expect_type(body.name(), Type::Number, index.ty, span)
}

pub(super) fn emit_address(
    base: &TypedExpr,
    index: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    body.emit_expr(base, span)?;
    body.emit_expr(index, span)?;

    body.call(body.helper(Helper::ArrayAddress));

    Ok(())
}

pub(super) fn emit_assignment(
    target: &TypedExpr,
    value: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    match &target.kind {
        TypedExprKind::Identifier { id, .. } => {
            let index = body.resolve(*id, target.ty, span)?;

            expect_type(body.name(), target.ty, value.ty, span)?;
            body.emit_expr(value, span)?;

            body.local_set(index);
        }

        TypedExprKind::Index { expr: base, index } => {
            let TypedExprKind::Identifier { id, .. } = &base.kind else {
                return Err(CodegenError::InvalidAssignmentTarget {
                    function: body.name().into(),
                    span,
                });
            };

            expect_index_types(base, index, target.ty, body, span)?;
            body.resolve(*id, base.ty, span)?;
            expect_type(body.name(), Type::Number, value.ty, span)?;

            emit_address(base, index, body, span)?;
            body.emit_expr(value, span)?;

            body.instruction(&Instruction::F64Store(memory_arg(
                0,
                abi::NUMBER_ELEMENT_ALIGN_LOG2,
            )));
        }

        _ => {
            return Err(CodegenError::InvalidAssignmentTarget {
                function: body.name().into(),
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
