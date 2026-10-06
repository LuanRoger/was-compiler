use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, function::Context};

pub(super) fn emit(
    expr: &TypedExpr,
    body: &mut Function,
    context: &Context<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let function = context.name;

    let kind = match &expr.kind {
        TypedExprKind::Number(literal) => {
            expect_type(function, Type::Number, expr.ty, span)?;

            let value = number(literal, function, span)?;

            body.instruction(&Instruction::F64Const(value.into()));

            return Ok(());
        }

        TypedExprKind::Bool(value) => {
            expect_type(function, Type::Bool, expr.ty, span)?;

            body.instruction(&Instruction::I32Const(i32::from(*value)));

            return Ok(());
        }

        TypedExprKind::Identifier { id, .. } => {
            let index = context.resolve(*id, expr.ty, span)?;

            body.instruction(&Instruction::LocalGet(index));

            return Ok(());
        }

        TypedExprKind::Unary { op, expr: operand } => {
            let instruction = match op {
                UnaryOp::Negate => Instruction::F64Neg,
                UnaryOp::Not => {
                    return Err(CodegenError::UnsupportedOperator {
                        function: function.into(),
                        operator: "!",
                        span,
                    });
                }
            };

            expect_type(function, Type::Number, expr.ty, span)?;
            expect_type(function, Type::Number, operand.ty, span)?;

            emit(operand, body, context, span)?;

            body.instruction(&instruction);

            return Ok(());
        }

        TypedExprKind::Binary { left, op, right } => {
            let instruction = arithmetic_instruction(op).map_err(|operator| {
                CodegenError::UnsupportedOperator {
                    function: function.into(),
                    operator,
                    span,
                }
            })?;

            expect_type(function, Type::Number, expr.ty, span)?;
            expect_type(function, Type::Number, left.ty, span)?;
            expect_type(function, Type::Number, right.ty, span)?;

            emit(left, body, context, span)?;
            emit(right, body, context, span)?;

            body.instruction(&instruction);

            return Ok(());
        }

        TypedExprKind::String(_) => "string",

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

fn arithmetic_instruction(op: &BinaryOp) -> Result<Instruction<'static>, &'static str> {
    match op {
        BinaryOp::Add => Ok(Instruction::F64Add),
        BinaryOp::Subtract => Ok(Instruction::F64Sub),
        BinaryOp::Multiply => Ok(Instruction::F64Mul),
        BinaryOp::Divide => Ok(Instruction::F64Div),

        BinaryOp::Remainder => Err("%"),
        BinaryOp::Equal => Err("=="),
        BinaryOp::NotEqual => Err("!="),
        BinaryOp::LessThan => Err("<"),
        BinaryOp::LessThanOrEqual => Err("<="),
        BinaryOp::GreaterThan => Err(">"),
        BinaryOp::GreaterThanOrEqual => Err(">="),
        BinaryOp::And => Err("&&"),
        BinaryOp::Or => Err("||"),
    }
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
