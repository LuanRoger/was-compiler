use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{LoopId, TypedBlock, TypedExpr},
};
use wasm_encoder::{BlockType, Instruction};

use super::{CodegenError, expect_operand, function::BodyEmitter};

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

        if flow.always_returns() || !cond_flow.can_fall_through() {
            emitter.instruction(&Instruction::Unreachable);
        }

        Ok(flow)
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

        emitter.enter_loop(id, break_label, continue_label);
        let cond_flow = emitter.emit_expr(cond, span)?;
        emitter.instruction(&Instruction::I32Eqz);
        emitter.branch_if(break_label)?;

        let flow = emitter.emit_block(block)?;

        emitter.branch(continue_label)?;
        emitter.end();
        emitter.end();

        if cond_flow.always_returns() {
            emitter.instruction(&Instruction::Unreachable);
            Ok(Flow::RETURN)
        } else {
            Ok(cond_flow.then(flow).while_loop())
        }
    })
}
