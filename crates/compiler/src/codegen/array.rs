use crate::ast::ArrayElementType;
use wasm_encoder::{Instruction, MemArg};

use super::{abi, function::BodyEmitter, memory::Helper};

pub(super) fn validator(element: ArrayElementType) -> Helper {
    match element {
        ArrayElementType::Number => Helper::ValidateArray,
        ArrayElementType::Bool => Helper::ValidateBoolArray,
        ArrayElementType::String => Helper::ValidateStringArray,
    }
}

pub(super) fn address(element: ArrayElementType) -> Helper {
    match element {
        ArrayElementType::Number => Helper::ArrayAddress,
        ArrayElementType::Bool => Helper::BoolArrayAddress,
        ArrayElementType::String => Helper::StringArrayAddress,
    }
}

pub(super) fn length(element: ArrayElementType) -> Helper {
    match element {
        ArrayElementType::Number => Helper::ArrayLen,
        ArrayElementType::Bool => Helper::BoolArrayLen,
        ArrayElementType::String => Helper::StringArrayLen,
    }
}

pub(super) fn equality(element: ArrayElementType) -> Helper {
    match element {
        ArrayElementType::Number => Helper::ArrayEqual,
        ArrayElementType::Bool => Helper::BoolArrayEqual,
        ArrayElementType::String => Helper::StringArrayEqual,
    }
}

pub(super) fn consumer(element: ArrayElementType) -> Option<Helper> {
    match element {
        ArrayElementType::Number => None,
        ArrayElementType::Bool => Some(Helper::ValidateBool),
        ArrayElementType::String => Some(Helper::ValidateString),
    }
}

pub(super) fn consume(element: ArrayElementType, body: &mut BodyEmitter<'_>) {
    if let Some(helper) = consumer(element) {
        body.call(body.helper(helper));
    }
}

pub(super) fn element(expr: &crate::typed_ast::TypedExpr) -> ArrayElementType {
    expr.array_element_type()
        .unwrap_or(ArrayElementType::Number)
}

pub(super) fn iteration_element(expr: &crate::typed_ast::TypedExpr) -> Option<ArrayElementType> {
    expr.array_element_type().filter(|element| {
        // Legacy Never iterables use the range-shaped plan, even when their
        // syntax constructs a numeric array. Retain its scratch and helper bytes.
        expr.ty != crate::typed_ast::ExprResult::Never || *element != ArrayElementType::Number
    })
}

pub(super) fn width(element: ArrayElementType) -> u32 {
    match element {
        ArrayElementType::Number => abi::NUMBER_ELEMENT_SIZE,
        ArrayElementType::Bool => abi::BOOL_ELEMENT_SIZE,
        ArrayElementType::String => abi::STRING_ARRAY_ELEMENT_SIZE,
    }
}

pub(super) fn argument(element: ArrayElementType, offset: u64) -> MemArg {
    MemArg {
        offset,
        align: match element {
            ArrayElementType::Number => abi::NUMBER_ELEMENT_ALIGN_LOG2,
            _ => width(element).trailing_zeros(),
        },
        memory_index: 0,
    }
}

pub(super) fn load(element: ArrayElementType, offset: u64) -> Instruction<'static> {
    let arg = argument(element, offset);
    match element {
        ArrayElementType::Number => Instruction::F64Load(arg),
        ArrayElementType::Bool => Instruction::I32Load8U(arg),
        ArrayElementType::String => Instruction::I32Load(arg),
    }
}

pub(super) fn store(element: ArrayElementType, offset: u64) -> Instruction<'static> {
    let arg = argument(element, offset);
    match element {
        ArrayElementType::Number => Instruction::F64Store(arg),
        ArrayElementType::Bool => Instruction::I32Store8(arg),
        ArrayElementType::String => Instruction::I32Store(arg),
    }
}
