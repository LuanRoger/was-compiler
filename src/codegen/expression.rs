use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::{CodegenError, call, expect_type, function::Context};

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
            let (ty, instruction) = match op {
                UnaryOp::Negate => (Type::Number, Instruction::F64Neg),
                UnaryOp::Not => (Type::Bool, Instruction::I32Eqz),
            };

            expect_type(function, ty, expr.ty, span)?;
            expect_type(function, ty, operand.ty, span)?;

            emit(operand, body, context, span)?;

            body.instruction(&instruction);

            return Ok(());
        }

        TypedExprKind::Binary { left, op, right } => {
            let (operand_type, result_type) = match op {
                BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                    (Type::Number, Type::Number)
                }

                BinaryOp::Remainder => {
                    return Err(CodegenError::UnsupportedOperator {
                        function: function.into(),
                        operator: "%",
                        span,
                    });
                }

                BinaryOp::LessThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThan
                | BinaryOp::GreaterThanOrEqual => (Type::Number, Type::Bool),

                BinaryOp::And | BinaryOp::Or => (Type::Bool, Type::Bool),

                BinaryOp::Equal | BinaryOp::NotEqual => {
                    expect_type(function, Type::Bool, expr.ty, span)?;

                    if !matches!(left.ty, Type::Number | Type::Bool | Type::String) {
                        expect_type(function, Type::Number, left.ty, span)?;
                    }

                    (left.ty, Type::Bool)
                }
            };

            expect_type(function, result_type, expr.ty, span)?;
            expect_type(function, operand_type, left.ty, span)?;
            expect_type(function, operand_type, right.ty, span)?;

            if matches!(op, BinaryOp::And | BinaryOp::Or) {
                emit(left, body, context, span)?;

                body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));

                if *op == BinaryOp::And {
                    emit(right, body, context, span)?;
                    normalize_bool(body);
                } else {
                    body.instruction(&Instruction::I32Const(1));
                }

                body.instruction(&Instruction::Else);

                if *op == BinaryOp::Or {
                    emit(right, body, context, span)?;
                    normalize_bool(body);
                } else {
                    body.instruction(&Instruction::I32Const(0));
                }

                body.instruction(&Instruction::End);

                return Ok(());
            }

            let instruction = binary_instruction(op, operand_type).map_err(|operator| {
                CodegenError::UnsupportedOperator {
                    function: function.into(),
                    operator,
                    span,
                }
            })?;

            let normalize_operands =
                matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) && operand_type == Type::Bool;

            emit(left, body, context, span)?;

            if normalize_operands {
                normalize_bool(body);
            }

            emit(right, body, context, span)?;

            if normalize_operands {
                normalize_bool(body);
            }

            body.instruction(&instruction);

            return Ok(());
        }

        TypedExprKind::Call { func, args, .. } => {
            return call::emit(*func, args, expr.ty, body, context, span);
        }

        TypedExprKind::String(_) => "string",
        TypedExprKind::Array(_) => "array",
        TypedExprKind::Index { .. } => "index",
    };

    Err(CodegenError::UnsupportedExpression {
        function: function.into(),
        kind,
        span,
    })
}

fn binary_instruction(
    op: &BinaryOp,
    operand_type: Type,
) -> Result<Instruction<'static>, &'static str> {
    match op {
        BinaryOp::Add => Ok(Instruction::F64Add),
        BinaryOp::Subtract => Ok(Instruction::F64Sub),
        BinaryOp::Multiply => Ok(Instruction::F64Mul),
        BinaryOp::Divide => Ok(Instruction::F64Div),

        BinaryOp::Equal => match operand_type {
            Type::Number => Ok(Instruction::F64Eq),
            Type::Bool => Ok(Instruction::I32Eq),
            _ => Err("=="),
        },

        BinaryOp::NotEqual => match operand_type {
            Type::Number => Ok(Instruction::F64Ne),
            Type::Bool => Ok(Instruction::I32Ne),
            _ => Err("!="),
        },

        BinaryOp::LessThan => Ok(Instruction::F64Lt),
        BinaryOp::LessThanOrEqual => Ok(Instruction::F64Le),
        BinaryOp::GreaterThan => Ok(Instruction::F64Gt),
        BinaryOp::GreaterThanOrEqual => Ok(Instruction::F64Ge),

        BinaryOp::Remainder => Err("%"),
        BinaryOp::And => Err("&&"),
        BinaryOp::Or => Err("||"),
    }
}

fn normalize_bool(body: &mut Function) {
    body.instruction(&Instruction::I32Eqz);
    body.instruction(&Instruction::I32Eqz);
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
