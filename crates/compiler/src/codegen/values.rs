use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{ExprResult, TypedExpr},
};
use wasm_encoder::{Instruction, MemArg, ValType};

use super::{
    CodegenError, abi, expect_operand, expect_result, function::BodyEmitter, memory::Helper,
};

pub(super) fn emit_array(
    elements: &[TypedExpr],
    element_type: crate::ast::ArrayElementType,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    let limit = || CodegenError::LimitExceeded {
        what: "array allocation size",
    };
    let length = u32::try_from(elements.len()).map_err(|_| limit())?;
    let size = length
        .checked_mul(super::array::width(element_type))
        .and_then(|payload| payload.checked_add(abi::DESCRIPTOR_SIZE))
        .filter(|size| u64::from(*size) <= super::memory::MAX_BYTES)
        .ok_or_else(limit)?;
    let temporary = body.temporary(ValType::I32);

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

    let mut flow = Flow::FALLTHROUGH;

    for (index, element) in (0..length).zip(elements) {
        expect_operand(body.name(), element_type.value_type(), element.ty, span)?;

        body.local_get(temporary);

        flow = flow.then(body.emit_expr(element, span)?);

        super::array::consume(element_type, body);
        body.instruction(&super::array::store(
            element_type,
            u64::from(abi::DESCRIPTOR_SIZE)
                + u64::from(index) * u64::from(super::array::width(element_type)),
        ));
    }

    body.local_get(temporary);

    Ok(flow)
}

pub(super) fn expect_index_types(
    base: &TypedExpr,
    index: &TypedExpr,
    result_type: ExprResult,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let expected = if base.ty == ExprResult::Never || index.ty == ExprResult::Never {
        ExprResult::Never
    } else {
        super::array::element(base).value_type().into()
    };
    expect_result(body.name(), expected, result_type, span)?;
    expect_operand(
        body.name(),
        Type::Array(super::array::element(base)),
        base.ty,
        span,
    )?;
    expect_operand(body.name(), Type::Number, index.ty, span)
}

pub(super) fn emit_address(
    base: &TypedExpr,
    index: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    let base_flow = body.emit_expr(base, span)?;
    let index_flow = body.emit_expr(index, span)?;

    body.call(body.helper(super::array::address(super::array::element(base))));

    Ok(base_flow.then(index_flow))
}

fn memory_arg(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}
