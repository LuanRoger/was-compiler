use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{ExprResult, LoopId, TypedBlock, TypedExpr, VariableId},
};
use wasm_encoder::{BlockType, Instruction, MemArg, ValType};

use super::{
    CodegenError, abi, expect_operand, expect_result, expression,
    function::{BodyEmitter, LoopKind},
    memory, range,
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

pub(super) fn emit_for(
    id: LoopId,
    variable: VariableId,
    iterable: &TypedExpr,
    block: &TypedBlock,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    if iterable.ty == ExprResult::Value(Type::NumberArray) {
        return emit_array_for(id, variable, iterable, block, emitter, span);
    }
    expect_operand(emitter.name(), Type::Range, iterable.ty, span)?;

    emitter.scoped(|emitter| {
        emitter.register_loop(id, span)?;
        let binding = emitter.declaration(variable, Type::Number, span)?;
        let handle = emitter.temporary(ValType::I32);
        let current = emitter.temporary(ValType::I64);
        let end = emitter.temporary(ValType::I64);
        let step = emitter.temporary(ValType::I64);
        let inclusive = emitter.temporary(ValType::I32);

        // Iterable transfers still see only enclosing loops and bindings.
        let iterable_flow = emitter.emit_expr(iterable, span)?;
        emitter.call(emitter.range_helper(range::Helper::Validate));
        emitter.local_set(handle);

        for (local, offset) in [
            (current, abi::RANGE_START_OFFSET),
            (end, abi::RANGE_END_OFFSET),
            (step, abi::RANGE_STEP_OFFSET),
        ] {
            emitter.local_get(handle);
            emitter.instruction(&Instruction::I64Load(MemArg {
                offset,
                align: abi::RANGE_INTEGER_ALIGN_LOG2,
                memory_index: 0,
            }));
            emitter.local_set(local);
        }
        emitter.local_get(handle);
        emitter.instruction(&Instruction::I32Load(MemArg {
            offset: abi::RANGE_INCLUSIVE_OFFSET,
            align: abi::RANGE_FLAG_ALIGN_LOG2,
            memory_index: 0,
        }));
        emitter.local_set(inclusive);

        let break_label = emitter.begin_block()?;
        let backedge = emitter.begin_loop()?;
        emitter.local_get(step);
        emitter.instruction(&Instruction::I64Const(0));
        emitter.instruction(&Instruction::I64GtS);
        emitter.begin_if(BlockType::Result(ValType::I32))?;
        emitter.local_get(current);
        emitter.local_get(end);
        emitter.instruction(&Instruction::I64GtS);
        emitter.otherwise();
        emitter.local_get(current);
        emitter.local_get(end);
        emitter.instruction(&Instruction::I64LtS);
        emitter.end();
        emitter.local_get(current);
        emitter.local_get(end);
        emitter.instruction(&Instruction::I64Eq);
        emitter.local_get(inclusive);
        emitter.instruction(&Instruction::I32Eqz);
        emitter.instruction(&Instruction::I32And);
        emitter.instruction(&Instruction::I32Or);
        emitter.branch_if(break_label)?;

        emitter.local_get(current);
        emitter.instruction(&Instruction::F64ConvertI64S);
        emitter.local_set(binding);
        emitter.promote(variable, Type::Number, span)?;
        emitter.initialize(variable);

        let continue_label = emitter.begin_block()?;
        emitter.enter_loop(id, LoopKind::Statement, break_label, continue_label);
        let body_flow = emitter.emit_block(block)?;
        emitter.end();

        // With M = 2^53 - 1, |current + step| <= 2M < 2^54 fits i64.
        // Retest before conversion/yield; an out-of-domain next value exits,
        // so no later addition can use it. Integer zero converts to +0.
        emitter.local_get(current);
        emitter.local_get(step);
        emitter.instruction(&Instruction::I64Add);
        emitter.local_set(current);
        emitter.branch(backedge)?;
        emitter.end();
        emitter.end();

        let flow = iterable_flow.then(body_flow.while_loop());
        if !flow.can_fall_through() {
            emitter.instruction(&Instruction::Unreachable);
        }

        Ok(flow)
    })
}

fn emit_array_for(
    id: LoopId,
    variable: VariableId,
    iterable: &TypedExpr,
    block: &TypedBlock,
    emitter: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    emitter.scoped(|emitter| {
        emitter.register_loop(id, span)?;
        let binding = emitter.declaration(variable, Type::Number, span)?;
        let handle = emitter.temporary(ValType::I32);
        let index = emitter.temporary(ValType::I32);
        let count = emitter.temporary(ValType::I32);

        // Validate even empty arrays before caching their fixed length.
        let iterable_flow = emitter.emit_expr(iterable, span)?;
        emitter.call(emitter.helper(memory::Helper::ValidateArray));
        emitter.local_set(handle);
        emitter.local_get(handle);
        emitter.instruction(&Instruction::I32Load(MemArg {
            offset: abi::DESCRIPTOR_LENGTH_OFFSET,
            align: abi::DESCRIPTOR_FIELD_ALIGN_LOG2,
            memory_index: 0,
        }));
        emitter.local_set(count);
        emitter.instruction(&Instruction::I32Const(0));
        emitter.local_set(index);

        let break_label = emitter.begin_block()?;
        let backedge = emitter.begin_loop()?;
        emitter.local_get(index);
        emitter.local_get(count);
        emitter.instruction(&Instruction::I32GeU);
        emitter.branch_if(break_label)?;

        // Keep checked addressing on each yield, not a cached payload pointer:
        // future element mutations through aliases must remain observable.
        emitter.local_get(handle);
        emitter.local_get(index);
        emitter.instruction(&Instruction::F64ConvertI32U);
        emitter.call(emitter.helper(memory::Helper::ArrayAddress));
        emitter.instruction(&Instruction::F64Load(MemArg {
            offset: 0,
            align: abi::NUMBER_ELEMENT_ALIGN_LOG2,
            memory_index: 0,
        }));
        emitter.local_set(binding);
        emitter.promote(variable, Type::Number, span)?;
        emitter.initialize(variable);

        let continue_label = emitter.begin_block()?;
        emitter.enter_loop(id, LoopKind::Statement, break_label, continue_label);
        let body_flow = emitter.emit_block(block)?;
        emitter.end();

        emitter.local_get(index);
        emitter.instruction(&Instruction::I32Const(1));
        emitter.instruction(&Instruction::I32Add);
        emitter.local_set(index);
        emitter.branch(backedge)?;
        emitter.end();
        emitter.end();

        let flow = iterable_flow.then(body_flow.while_loop());
        if !flow.can_fall_through() {
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
