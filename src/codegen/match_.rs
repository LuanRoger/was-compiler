use crate::{
    ast::{BinaryOp, MatchPattern, MatchPatternKind, Type},
    flow::Flow,
    tokens::Span,
    typed_ast::{ExprResult, TypedExpr, TypedMatchArm},
};
use wasm_encoder::{Instruction, ValType};

use super::{
    CodegenError, expect_result, expression, function::BodyEmitter, indices::LocalIndex, memory,
};

// Never needs neither a cached handle nor a fabricated value. Keep this policy
// shared with preorder temporary planning, including wildcard-only matches.
pub(super) fn scratch_type(result: ExprResult) -> Option<ValType> {
    match result {
        ExprResult::Value(Type::Number) => Some(ValType::F64),
        ExprResult::Value(Type::Bool | Type::String) => Some(ValType::I32),
        _ => None,
    }
}

pub(super) fn emit(
    scrutinee: &TypedExpr,
    arms: &[TypedMatchArm],
    annotation: Option<ExprResult>,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(Flow, ExprResult), CodegenError> {
    crate::match_patterns::check(scrutinee.ty, arms.iter().map(|arm| &arm.pattern)).map_err(
        |(message, origin)| CodegenError::InvalidMatch {
            message,
            span: if origin == Span::new(0, 0) {
                span
            } else {
                origin
            },
        },
    )?;

    body.structured(|body| {
        let cached = scratch_type(scrutinee.ty).map(|ty| body.temporary(ty));
        let scrutinee_flow = body.emit_expr(scrutinee, span)?;
        if let Some(cached) = cached {
            body.local_set(cached);
        }

        let region = expression::block_type(annotation.unwrap_or(ExprResult::Void));
        let mut regions = 0;
        let mut arms_flow = Flow::NONE;
        let mut joined = ExprResult::Never;
        let mut wildcard = false;

        for arm in arms {
            wildcard = matches!(arm.pattern.kind, MatchPatternKind::Wildcard);
            if !wildcard {
                if let Some(cached) = cached {
                    emit_test(&arm.pattern, cached, body)?;
                } else {
                    // A Never scrutinee has no operand to compare. The chain is
                    // unreachable, but its bodies still undergo checked lowering.
                    body.instruction(&Instruction::I32Const(0));
                }
                body.begin_if(region)?;
                regions += 1;
            }

            let arm_flow = body.scoped(|body| body.emit_expr(&arm.body, arm.span))?;
            if annotation.is_some() {
                let result = if arm_flow.can_fall_through() {
                    arm.body.ty
                } else {
                    ExprResult::Never
                };
                if joined == ExprResult::Never {
                    joined = result;
                } else if result != ExprResult::Never {
                    expect_result(body.name(), joined, result, arm.span)?;
                }
            } else if matches!(arm.body.ty, ExprResult::Value(_)) {
                body.instruction(&Instruction::Drop);
            }
            arms_flow = arms_flow.join(arm_flow);

            if !scrutinee_flow.can_fall_through() {
                // Arm values cannot supply a result after a noncompleting
                // scrutinee, even if the arm itself completes normally.
                body.instruction(&Instruction::Unreachable);
            }
            if !wildcard {
                body.otherwise();
            }
        }

        if !wildcard {
            // Exhaustive bool patterns cover every normalized boolean. This
            // residual Wasm path supplies no invented result or flow outcome.
            body.instruction(&Instruction::Unreachable);
        }
        for _ in 0..regions {
            body.end();
        }

        let flow = scrutinee_flow.then(arms_flow);
        let actual = if !flow.can_fall_through() {
            body.instruction(&Instruction::Unreachable);
            ExprResult::Never
        } else if annotation.is_some() {
            joined
        } else {
            ExprResult::Void
        };
        if let Some(annotation) = annotation {
            expect_result(body.name(), actual, annotation, span)?;
        }
        Ok((flow, actual))
    })
}

fn constant(value: &str, span: Span) -> Result<f64, CodegenError> {
    crate::match_patterns::number(value).ok_or(CodegenError::InvalidMatch {
        message: "invalid numeric pattern constant",
        span,
    })
}

fn emit_test(
    pattern: &MatchPattern,
    cached: LocalIndex,
    body: &mut BodyEmitter<'_>,
) -> Result<(), CodegenError> {
    body.local_get(cached);
    match &pattern.kind {
        MatchPatternKind::Number(value) => {
            body.instruction(&Instruction::F64Const(
                constant(value, pattern.span)?.into(),
            ));
            body.instruction(&Instruction::F64Eq);
        }
        MatchPatternKind::Bool(value) => {
            body.normalize_bool();
            body.instruction(&Instruction::I32Const(i32::from(*value)));
            body.instruction(&Instruction::I32Eq);
        }
        MatchPatternKind::String(value) => {
            body.instruction(&Instruction::I32Const(body.string(value) as i32));
            body.call(body.helper(memory::Helper::StringEqual));
        }
        MatchPatternKind::Relational { op, value } => {
            body.instruction(&Instruction::F64Const(
                constant(value, pattern.span)?.into(),
            ));
            let instruction = match op {
                BinaryOp::LessThan => Instruction::F64Lt,
                BinaryOp::LessThanOrEqual => Instruction::F64Le,
                BinaryOp::GreaterThan => Instruction::F64Gt,
                BinaryOp::GreaterThanOrEqual => Instruction::F64Ge,
                _ => {
                    return Err(CodegenError::InvalidMatch {
                        message: "invalid relational pattern operator",
                        span: pattern.span,
                    });
                }
            };
            body.instruction(&instruction);
        }
        MatchPatternKind::Interval {
            start,
            end,
            inclusive,
        } => {
            body.instruction(&Instruction::F64Const(
                constant(start, pattern.span)?.into(),
            ));
            body.instruction(&Instruction::F64Ge);
            body.local_get(cached);
            body.instruction(&Instruction::F64Const(constant(end, pattern.span)?.into()));
            body.instruction(&if *inclusive {
                Instruction::F64Le
            } else {
                Instruction::F64Lt
            });
            body.instruction(&Instruction::I32And);
        }
        MatchPatternKind::Wildcard => unreachable!("wildcards have no comparison"),
    }
    Ok(())
}
