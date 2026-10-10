use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{ExprResult, LoopId, TypedBlock, TypedExpr},
};
use wasm_encoder::{BlockType, Instruction};

use super::{
    CodegenError, expect_operand, expect_result, expression,
    function::{BodyEmitter, LoopKind},
};

pub(super) fn emit_if(
    cond: &TypedExpr,
    then: &TypedBlock,
    else_: Option<&TypedBlock>,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    expect_operand(emitter.name(), Type::Bool, cond.ty, span)?;

    emitter.structured(|emitter| {
        let cond_flow = emitter.emit_expr(cond, span)?;
        emitter.begin_if(BlockType::Empty)?;

        let then_flow = emitter.scoped(|emitter| emitter.emit_block(then))?;
        let else_flow = match else_ {
            Some(block) => {
                emitter.otherwise();

                emitter.scoped(|emitter| emitter.emit_block(block))?
            }

            None => Flow::FALLTHROUGH,
        };

        emitter.end();

        let flow = cond_flow.then(then_flow.join(else_flow));

        if flow.returns_or_diverges() || !cond_flow.can_fall_through() {
            emitter.instruction(&Instruction::Unreachable);
        }

        Ok(flow)
    })
}

pub(super) fn emit_loop(
    id: LoopId,
    block: &TypedBlock,
    kind: LoopKind,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<(Flow, ExprResult), CodegenError> {
    emitter.scoped(|emitter| {
        emitter.register_loop(id, span)?;
        let annotation = match kind {
            LoopKind::Statement => ExprResult::Void,
            LoopKind::Value(result) => result,
        };
        let break_label = emitter.begin_value_block(expression::block_type(annotation))?;
        let continue_label = emitter.begin_loop()?;
        emitter.enter_loop(id, kind, break_label, continue_label);

        let body_flow = emitter.emit_block(block)?;
        emitter.branch(continue_label)?;
        emitter.end();
        // The empty backedge loop cannot supply the outer block's result.
        // Only a value break to the outer label can supply that result.
        emitter.instruction(&Instruction::Unreachable);
        emitter.end();

        let flow = body_flow.infinite_loop();
        let actual = match kind {
            LoopKind::Statement if flow.can_fall_through() => ExprResult::Void,
            LoopKind::Statement => ExprResult::Never,
            LoopKind::Value(_) => {
                let ty = body_flow.break_type().map_err(|(expected, found)| {
                    CodegenError::TypeMismatch {
                        function: emitter.name().into(),
                        expected,
                        found,
                        span,
                    }
                })?;
                let actual = ty.map_or(ExprResult::Never, ExprResult::from);
                expect_result(emitter.name(), actual, annotation, span)?;
                actual
            }
        };

        Ok((flow, actual))
    })
}

pub(super) fn emit_while(
    id: LoopId,
    cond: &TypedExpr,
    block: &TypedBlock,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    expect_operand(emitter.name(), Type::Bool, cond.ty, span)?;

    emitter.scoped(|emitter| {
        emitter.register_loop(id, span)?;

        let break_label = emitter.begin_block()?;
        let continue_label = emitter.begin_loop()?;

        emitter.enter_loop(id, LoopKind::Statement, break_label, continue_label);
        let cond_flow = emitter.emit_expr(cond, span)?;
        emitter.instruction(&Instruction::I32Eqz);
        emitter.branch_if(break_label)?;

        let flow = emitter.emit_block(block)?;

        emitter.branch(continue_label)?;
        emitter.end();
        emitter.end();

        let flow = if cond_flow.can_fall_through() {
            cond_flow.then(flow).while_loop()
        } else {
            cond_flow.infinite_loop()
        };

        if !flow.can_fall_through() {
            emitter.instruction(&Instruction::Unreachable);
        }

        Ok(flow)
    })
}
