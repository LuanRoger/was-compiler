use crate::ast::Type;
use wasm_encoder::ValType;

use super::{CodegenError, checked_u32};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Signature {
    pub params: Vec<ValType>,
    pub result: Option<ValType>,
}

pub(super) fn signature(
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
