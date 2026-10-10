use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    flow::Flow,
    tokens::Span,
    typed_ast::{ExprResult, TypedExpr, TypedExprKind, TypedValueBlock},
};
use wasm_encoder::{BlockType, Instruction, MemArg, ValType};

use super::{
    CodegenError, abi, call, expect_operand, expect_result, function::BodyEmitter, memory::Helper,
    values,
};

fn result(flow: Flow, ty: Type) -> ExprResult {
    if flow.can_fall_through() {
        ty.into()
    } else {
        ExprResult::Never
    }
}

fn eager_result(ty: Type, operands: &[&TypedExpr]) -> ExprResult {
    if operands.iter().any(|expr| expr.ty == ExprResult::Never) {
        ExprResult::Never
    } else {
        ty.into()
    }
}

fn block_type(result: ExprResult) -> BlockType {
    match result {
        ExprResult::Value(Type::Number) => BlockType::Result(ValType::F64),
        ExprResult::Value(Type::Bool | Type::String | Type::NumberArray) => {
            BlockType::Result(ValType::I32)
        }
        ExprResult::Value(Type::Void) | ExprResult::Void | ExprResult::Never => BlockType::Empty,
    }
}

pub(super) fn emit(
    expr: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    let function = body.name();

    let (flow, actual) = match &expr.kind {
        TypedExprKind::Number(literal) => {
            expect_result(function, Type::Number.into(), expr.ty, span)?;

            let value = number(literal, function, span)?;

            body.instruction(&Instruction::F64Const(value.into()));

            (Flow::FALLTHROUGH, Type::Number.into())
        }

        TypedExprKind::Bool(value) => {
            expect_result(function, Type::Bool.into(), expr.ty, span)?;

            body.instruction(&Instruction::I32Const(i32::from(*value)));

            (Flow::FALLTHROUGH, Type::Bool.into())
        }

        TypedExprKind::Identifier { id, .. } => {
            let index = body.resolve_expr(*id, expr.ty, span)?;

            body.local_get(index);

            (Flow::FALLTHROUGH, expr.ty)
        }

        TypedExprKind::Unary { op, expr: operand } => {
            let (ty, instruction) = match op {
                UnaryOp::Negate => (Type::Number, Instruction::F64Neg),
                UnaryOp::Not => (Type::Bool, Instruction::I32Eqz),
            };

            expect_result(function, eager_result(ty, &[operand]), expr.ty, span)?;
            expect_operand(function, ty, operand.ty, span)?;

            let flow = body.emit_expr(operand, span)?;

            body.instruction(&instruction);

            (flow, result(flow, ty))
        }

        TypedExprKind::Binary { left, op, right } => {
            let logical = matches!(op, BinaryOp::And | BinaryOp::Or);

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
                    let ty = left
                        .ty
                        .value_type()
                        .or_else(|| right.ty.value_type())
                        .unwrap_or(Type::Number);
                    (ty, Type::Bool)
                }
            };

            let predicted = if logical {
                eager_result(result_type, &[left])
            } else {
                eager_result(result_type, &[left, right])
            };

            expect_result(function, predicted, expr.ty, span)?;
            // Void is not an equality operand, even when its peer is Never.
            if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) && left.ty == ExprResult::Void {
                expect_operand(function, Type::Number, left.ty, span)?;
            }
            expect_operand(function, operand_type, left.ty, span)?;
            expect_operand(function, operand_type, right.ty, span)?;

            let left_flow = body.emit_expr(left, span)?;
            let flow = if logical {
                emit_logical(op, right, left_flow, body, span)?
            } else {
                let normalize = matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
                    && operand_type == Type::Bool;
                if normalize {
                    body.normalize_bool();
                }
                let right_flow = body.emit_expr(right, span)?;
                if normalize {
                    body.normalize_bool();
                }
                if matches!(operand_type, Type::String | Type::NumberArray) {
                    body.call(body.helper(if operand_type == Type::String {
                        Helper::StringEqual
                    } else {
                        Helper::ArrayEqual
                    }));
                    if *op == BinaryOp::NotEqual {
                        body.instruction(&Instruction::I32Eqz);
                    }
                } else {
                    let instruction =
                        binary_instruction(op, operand_type, body).map_err(|operator| {
                            CodegenError::UnsupportedOperator {
                                function: function.into(),
                                operator,
                                span,
                            }
                        })?;
                    body.instruction(&instruction);
                }
                left_flow.then(right_flow)
            };
            (flow, result(flow, result_type))
        }

        TypedExprKind::Call { func, args, .. } => {
            let (flow, ty) = call::emit(*func, args, expr.ty, body, span)?;
            (flow, result(flow, ty))
        }

        TypedExprKind::String(value) => {
            expect_result(function, Type::String.into(), expr.ty, span)?;
            body.instruction(&Instruction::I32Const(body.string(value) as i32));
            (Flow::FALLTHROUGH, Type::String.into())
        }

        TypedExprKind::Array(elements) => {
            let predicted = if elements
                .iter()
                .any(|element| element.ty == ExprResult::Never)
            {
                ExprResult::Never
            } else {
                Type::NumberArray.into()
            };

            expect_result(function, predicted, expr.ty, span)?;
            let flow = values::emit_array(elements, body, span)?;
            (flow, result(flow, Type::NumberArray))
        }

        TypedExprKind::Index { expr: base, index } => {
            values::expect_index_types(base, index, expr.ty, body, span)?;
            let flow = values::emit_address(base, index, body, span)?;
            body.instruction(&Instruction::F64Load(MemArg {
                offset: 0,
                align: abi::NUMBER_ELEMENT_ALIGN_LOG2,
                memory_index: 0,
            }));
            (flow, result(flow, Type::Number))
        }

        TypedExprKind::Block(block) => emit_value_block(block, expr.ty, body)?,
        TypedExprKind::If { cond, then, else_ } => {
            expect_operand(function, Type::Bool, cond.ty, span)?;
            let cond_flow = body.emit_expr(cond, span)?;
            body.begin_if(block_type(expr.ty))?;
            let (then_flow, then_result) = emit_value_contents(then, body)?;
            if !cond_flow.can_fall_through() {
                body.instruction(&Instruction::Unreachable);
            }
            let (else_flow, else_result) = if let Some(block) = else_ {
                body.otherwise();
                let outcome = emit_value_contents(block, body)?;
                if !cond_flow.can_fall_through() {
                    body.instruction(&Instruction::Unreachable);
                }
                outcome
            } else {
                (Flow::FALLTHROUGH, ExprResult::Void)
            };
            // Joining is structural, even if the condition itself never completes.
            let joined = join_results(then_result, else_result, body, span, else_.is_some())?;
            let flow = cond_flow.then(then_flow.join(else_flow));
            body.end();
            let actual = if flow.can_fall_through() {
                joined
            } else {
                ExprResult::Never
            };
            (flow, actual)
        }
    };

    expect_result(function, actual, expr.ty, span)?;
    if !flow.can_fall_through() {
        // End restores the enclosing Wasm stack's reachability. Reassert the
        // checked absence of a normal path, rather than inventing a result value.
        body.instruction(&Instruction::Unreachable);
    }
    Ok(flow)
}

fn emit_logical(
    op: &BinaryOp,
    right: &TypedExpr,
    left_flow: Flow,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    body.begin_if(block_type(result(left_flow, Type::Bool)))?;

    let then_flow = if *op == BinaryOp::And {
        let flow = body.emit_expr(right, span)?;
        body.normalize_bool();
        flow
    } else {
        body.instruction(&Instruction::I32Const(1));
        Flow::FALLTHROUGH
    };

    if !left_flow.can_fall_through() {
        body.instruction(&Instruction::Unreachable);
    }

    body.otherwise();

    let else_flow = if *op == BinaryOp::Or {
        let flow = body.emit_expr(right, span)?;
        body.normalize_bool();
        flow
    } else {
        body.instruction(&Instruction::I32Const(0));
        Flow::FALLTHROUGH
    };

    if !left_flow.can_fall_through() {
        body.instruction(&Instruction::Unreachable);
    }

    body.end();

    Ok(left_flow.then(then_flow.join(else_flow)))
}

fn join_results(
    then: ExprResult,
    else_: ExprResult,
    body: &BodyEmitter<'_>,
    span: Span,
    has_else: bool,
) -> Result<ExprResult, CodegenError> {
    if !has_else {
        return Err(CodegenError::MissingElse {
            function: body.name().into(),
            span,
        });
    }
    match (then, else_) {
        (ExprResult::Never, other) | (other, ExprResult::Never) => Ok(other),
        _ => {
            expect_result(body.name(), then, else_, span)?;
            Ok(then)
        }
    }
}

fn emit_value_block(
    block: &TypedValueBlock,
    annotation: ExprResult,
    body: &mut BodyEmitter<'_>,
) -> Result<(Flow, ExprResult), CodegenError> {
    body.begin_value_block(block_type(annotation))?;
    let outcome = emit_value_contents(block, body)?;
    body.end();
    Ok(outcome)
}

fn emit_value_contents(
    block: &TypedValueBlock,
    body: &mut BodyEmitter<'_>,
) -> Result<(Flow, ExprResult), CodegenError> {
    body.scoped(|body| {
        let statements = body.emit_block(&block.statements)?;
        let (flow, actual) = if let Some(tail) = &block.tail {
            let flow = statements.then(body.emit_expr(tail, block.span)?);
            let actual = if flow.can_fall_through() {
                tail.ty
            } else {
                ExprResult::Never
            };
            (flow, actual)
        } else {
            (statements, result(statements, Type::Void))
        };
        if !flow.can_fall_through() {
            // Unreachable tails are checked/emitted, but cannot supply a region result.
            body.instruction(&Instruction::Unreachable);
        }
        Ok((flow, actual))
    })
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
