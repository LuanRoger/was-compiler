use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{BlockType, Instruction, MemArg, ValType};

use super::{CodegenError, abi, call, expect_type, function::BodyEmitter, memory::Helper, values};

pub(super) fn emit(
    expr: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(), CodegenError> {
    let function = body.name();

    match &expr.kind {
        TypedExprKind::Number(literal) => {
            expect_type(function, Type::Number, expr.ty, span)?;

            let value = number(literal, function, span)?;

            body.instruction(&Instruction::F64Const(value.into()));

            Ok(())
        }

        TypedExprKind::Bool(value) => {
            expect_type(function, Type::Bool, expr.ty, span)?;

            body.instruction(&Instruction::I32Const(i32::from(*value)));

            Ok(())
        }

        TypedExprKind::Identifier { id, .. } => {
            let index = body.resolve(*id, expr.ty, span)?;

            body.local_get(index);

            Ok(())
        }

        TypedExprKind::Unary { op, expr: operand } => {
            let (ty, instruction) = match op {
                UnaryOp::Negate => (Type::Number, Instruction::F64Neg),
                UnaryOp::Not => (Type::Bool, Instruction::I32Eqz),
            };

            expect_type(function, ty, expr.ty, span)?;
            expect_type(function, ty, operand.ty, span)?;

            body.emit_expr(operand, span)?;

            body.instruction(&instruction);

            Ok(())
        }

        TypedExprKind::Binary { left, op, right } => {
            let (operand_type, result_type) = match op {
                BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Remainder => (Type::Number, Type::Number),

                BinaryOp::LessThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThan
                | BinaryOp::GreaterThanOrEqual => (Type::Number, Type::Bool),

                BinaryOp::And | BinaryOp::Or => (Type::Bool, Type::Bool),

                BinaryOp::Equal | BinaryOp::NotEqual => {
                    expect_type(function, Type::Bool, expr.ty, span)?;

                    if !matches!(
                        left.ty,
                        Type::Number | Type::Bool | Type::String | Type::NumberArray
                    ) {
                        expect_type(function, Type::Number, left.ty, span)?;
                    }

                    (left.ty, Type::Bool)
                }
            };

            expect_type(function, result_type, expr.ty, span)?;
            expect_type(function, operand_type, left.ty, span)?;
            expect_type(function, operand_type, right.ty, span)?;

            if matches!(op, BinaryOp::And | BinaryOp::Or) {
                body.emit_expr(left, span)?;

                body.begin_if(BlockType::Result(ValType::I32));

                if *op == BinaryOp::And {
                    body.emit_expr(right, span)?;
                    body.normalize_bool();
                } else {
                    body.instruction(&Instruction::I32Const(1));
                }

                body.otherwise();

                if *op == BinaryOp::Or {
                    body.emit_expr(right, span)?;
                    body.normalize_bool();
                } else {
                    body.instruction(&Instruction::I32Const(0));
                }

                body.end();

                return Ok(());
            }

            if matches!(operand_type, Type::String | Type::NumberArray) {
                let helper = if operand_type == Type::String {
                    Helper::StringEqual
                } else {
                    Helper::ArrayEqual
                };

                body.emit_expr(left, span)?;
                body.emit_expr(right, span)?;

                body.call(body.helper(helper));

                if *op == BinaryOp::NotEqual {
                    body.instruction(&Instruction::I32Eqz);
                }

                return Ok(());
            }

            let instruction = binary_instruction(op, operand_type, body).map_err(|operator| {
                CodegenError::UnsupportedOperator {
                    function: function.into(),
                    operator,
                    span,
                }
            })?;

            let normalize_operands =
                matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) && operand_type == Type::Bool;

            body.emit_expr(left, span)?;

            if normalize_operands {
                body.normalize_bool();
            }

            body.emit_expr(right, span)?;

            if normalize_operands {
                body.normalize_bool();
            }

            body.instruction(&instruction);

            Ok(())
        }

        TypedExprKind::Call { func, args, .. } => call::emit(*func, args, expr.ty, body, span),

        TypedExprKind::String(value) => {
            expect_type(function, Type::String, expr.ty, span)?;

            body.instruction(&Instruction::I32Const(body.string(value) as i32));

            Ok(())
        }

        TypedExprKind::Array(elements) => {
            expect_type(function, Type::NumberArray, expr.ty, span)?;

            values::emit_array(elements, body, span)
        }

        TypedExprKind::Index { expr: base, index } => {
            values::expect_index_types(base, index, expr.ty, body, span)?;
            values::emit_address(base, index, body, span)?;

            body.instruction(&Instruction::F64Load(MemArg {
                offset: 0,
                align: abi::NUMBER_ELEMENT_ALIGN_LOG2,
                memory_index: 0,
            }));

            Ok(())
        }
    }
}

fn binary_instruction(
    op: &BinaryOp,
    operand_type: Type,
    body: &BodyEmitter<'_>,
) -> Result<Instruction<'static>, &'static str> {
    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Remainder => arithmetic_instruction(op, body),

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

        BinaryOp::And => Err("&&"),
        BinaryOp::Or => Err("||"),
    }
}

pub(super) fn arithmetic_instruction(
    op: &BinaryOp,
    body: &BodyEmitter<'_>,
) -> Result<Instruction<'static>, &'static str> {
    match op {
        BinaryOp::Add => Ok(Instruction::F64Add),
        BinaryOp::Subtract => Ok(Instruction::F64Sub),
        BinaryOp::Multiply => Ok(Instruction::F64Mul),
        BinaryOp::Divide => Ok(Instruction::F64Div),
        BinaryOp::Remainder => Ok(Instruction::Call(body.remainder().raw())),
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
