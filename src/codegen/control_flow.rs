use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedBlock, TypedExpr},
};
use wasm_encoder::{BlockType, Instruction};

use super::{CodegenError, expect_type, function::BodyEmitter, indices::LabelDepth};

pub(super) fn emit_if(
    cond: &TypedExpr,
    then: &TypedBlock,
    else_: Option<&TypedBlock>,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<bool, CodegenError> {
    expect_type(emitter.name(), Type::Bool, cond.ty, span)?;
    emitter.emit_expr(cond, span)?;
    emitter.begin_if(BlockType::Empty);

    let then_returns = emitter.scoped(|emitter| emitter.emit_block(then))?;
    let else_returns = match else_ {
        Some(block) => {
            emitter.otherwise();

            emitter.scoped(|emitter| emitter.emit_block(block))?
        }

        None => false,
    };

    emitter.end();

    let always_returns = then_returns && else_returns;

    if always_returns {
        emitter.instruction(&Instruction::Unreachable);
    }

    Ok(always_returns)
}

pub(super) fn emit_while(
    cond: &TypedExpr,
    block: &TypedBlock,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<bool, CodegenError> {
    expect_type(emitter.name(), Type::Bool, cond.ty, span)?;
    emitter.begin_block();
    emitter.begin_loop();
    emitter.emit_expr(cond, span)?;

    emitter.instruction(&Instruction::I32Eqz);
    emitter.branch_if(LabelDepth::LOOP_EXIT);
    emitter.scoped(|emitter| emitter.emit_block(block))?;
    emitter.branch(LabelDepth::LOOP_BACK_EDGE);
    emitter.end();
    emitter.end();

    Ok(false)
}
