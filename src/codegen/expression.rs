use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type};

pub(super) fn emit(
    expr: &TypedExpr,
    body: &mut Function,
    function: &str,
    span: Span,
) -> Result<(), CodegenError> {
    let kind = match &expr.kind {
        TypedExprKind::Number(literal) => {
            expect_type(function, Type::Number, expr.ty, span)?;

            let value = number(literal, function, span)?;

            body.instruction(&Instruction::F64Const(value.into()));

            return Ok(());
        }

        TypedExprKind::String(_) => "string",
        TypedExprKind::Bool(_) => "bool",
        TypedExprKind::Identifier { .. } => "identifier",
        TypedExprKind::Unary { .. } => "unary",
        TypedExprKind::Binary { .. } => "binary",
        TypedExprKind::Call { .. } => "call",
        TypedExprKind::Array(_) => "array",
        TypedExprKind::Index { .. } => "index",
    };

    Err(CodegenError::UnsupportedExpression {
        function: function.into(),
        kind,
        span,
    })
}

fn number(literal: &str, function: &str, span: Span) -> Result<f64, CodegenError> {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let valid = match literal.split_once('.') {
        Some((integer, fraction)) => digits(integer) && digits(fraction),
        None => digits(literal),
    };

    if !valid {
        return Err(CodegenError::InvalidNumberLiteral {
            function: function.into(),
            literal: literal.into(),
            span,
        });
    }

    let value = literal
        .parse::<f64>()
        .map_err(|_| CodegenError::InvalidNumberLiteral {
            function: function.into(),
            literal: literal.into(),
            span,
        })?;

    if !value.is_finite() {
        return Err(CodegenError::NumberLiteralOutOfRange {
            function: function.into(),
            literal: literal.into(),
            span,
        });
    }

    Ok(value)
}
