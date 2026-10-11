use crate::{
    ast::Type,
    host::{supports_external_result, supports_external_value},
};
use wasm_encoder::ValType;

use super::{CodegenError, checked_u32};

pub(super) const DESCRIPTOR_SIZE: u32 = 8;
pub(super) const DESCRIPTOR_DATA_OFFSET: u64 = 0;
pub(super) const DESCRIPTOR_LENGTH_OFFSET: u64 = 4;
pub(super) const DESCRIPTOR_ALIGNMENT: u32 = 8;
pub(super) const DESCRIPTOR_FIELD_ALIGN_LOG2: u32 = 2;
pub(super) const NUMBER_ELEMENT_SIZE: u32 = 8;
pub(super) const NUMBER_ELEMENT_ALIGNMENT: u32 = 8;
pub(super) const NUMBER_ELEMENT_ALIGN_LOG2: u32 = NUMBER_ELEMENT_ALIGNMENT.trailing_zeros();
pub(super) const STRING_ELEMENT_SIZE: u32 = 1;
pub(super) const MIN_ADDRESS: u32 = 8;
pub(super) const ALLOCATION_ALIGNMENT: u32 = 8;
pub(super) const MIN_ALLOCATION_BYTES: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Signature {
    pub params: Vec<ValType>,
    pub result: Option<ValType>,
}

pub(super) fn internal_signature(
    function: &str,
    params: impl ExactSizeIterator<Item = Type>,
    return_type: Type,
) -> Result<Signature, CodegenError> {
    checked_u32(params.len(), "function parameter count")?;

    let params = params
        .enumerate()
        .map(|(index, ty)| {
            if ty == Type::Void {
                return Err(CodegenError::VoidParameter {
                    function: function.to_owned(),
                    index,
                });
            }

            value_type(function, "parameter", ty)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let result = if return_type == Type::Void {
        None
    } else {
        Some(value_type(function, "return", return_type)?)
    };

    Ok(Signature { params, result })
}

pub(super) fn check_external_signature(
    function: &str,
    params: impl Iterator<Item = Type>,
    return_type: Type,
) -> Result<(), CodegenError> {
    for (index, ty) in params.enumerate() {
        if ty == Type::Void {
            return Err(CodegenError::VoidParameter {
                function: function.to_owned(),
                index,
            });
        }

        if !supports_external_value(ty) {
            return Err(CodegenError::UnsupportedAbiType {
                function: function.to_owned(),
                position: "parameter",
                ty,
            });
        }
    }

    if !supports_external_result(return_type) {
        return Err(CodegenError::UnsupportedAbiType {
            function: function.to_owned(),
            position: "return",
            ty: return_type,
        });
    }

    Ok(())
}

pub(super) fn value_type(
    function: &str,
    position: &'static str,
    ty: Type,
) -> Result<ValType, CodegenError> {
    match ty {
        Type::Number => Ok(ValType::F64),
        Type::Bool | Type::String | Type::NumberArray => Ok(ValType::I32),
        Type::Void => Err(CodegenError::UnsupportedAbiType {
            function: function.to_owned(),
            position,
            ty,
        }),
    }
}
