use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedBlock, TypedExpr},
};
use wasm_encoder::{BlockType, Function, Instruction};

use super::{CodegenError, expect_type, expression, function::Context, statement};

pub(super) fn emit_if(
    cond: &TypedExpr,
    then: &TypedBlock,
    else_: Option<&TypedBlock>,
    body: &mut Function,
    context: &mut Context<'_>,
    span: Span,
) -> Result<bool, CodegenError> {
    expect_type(context.name, Type::Bool, cond.ty, span)?;
    expression::emit(cond, body, context, span)?;

    body.instruction(&Instruction::If(BlockType::Empty));

    let then_returns = context.scoped(|context| statement::emit_block(then, body, context))?;
    let else_returns = match else_ {
        Some(block) => {
            body.instruction(&Instruction::Else);

            context.scoped(|context| statement::emit_block(block, body, context))?
        }

        None => false,
    };

    body.instruction(&Instruction::End);

    let always_returns = then_returns && else_returns;

    if always_returns {
        body.instruction(&Instruction::Unreachable);
    }

    Ok(always_returns)
}

pub(super) fn emit_while(
    cond: &TypedExpr,
    block: &TypedBlock,
    body: &mut Function,
    context: &mut Context<'_>,
    span: Span,
) -> Result<bool, CodegenError> {
    expect_type(context.name, Type::Bool, cond.ty, span)?;

    body.instruction(&Instruction::Block(BlockType::Empty));
    body.instruction(&Instruction::Loop(BlockType::Empty));

    expression::emit(cond, body, context, span)?;

    body.instruction(&Instruction::I32Eqz);
    body.instruction(&Instruction::BrIf(1));

    context.scoped(|context| statement::emit_block(block, body, context))?;

    body.instruction(&Instruction::Br(0));
    body.instruction(&Instruction::End);
    body.instruction(&Instruction::End);

    Ok(false)
}
