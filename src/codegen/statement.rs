use crate::{
    ast::Type,
    flow::Flow,
    typed_ast::{ExprResult, TypedBlock, TypedStatement, TypedStatementKind},
};
use wasm_encoder::Instruction;

use super::{
    CodegenError, control_flow, expect_operand, expect_type, function::BodyEmitter, place,
};

pub(super) fn emit_block(
    block: &TypedBlock,
    emitter: &mut BodyEmitter<'_>,
) -> Result<Flow, CodegenError> {
    let mut flow = Flow::FALLTHROUGH;

    for statement in block {
        let next = emit(statement, emitter)?;

        flow = flow.then(next);
    }

    Ok(flow)
}

pub(super) fn emit(
    statement: &TypedStatement,
    emitter: &mut BodyEmitter<'_>,
) -> Result<Flow, CodegenError> {
    let function = emitter.name();
    let span = statement.span;

    match &statement.kind {
        TypedStatementKind::Let { id, ty, value, .. } => {
            let index = emitter.declaration(*id, *ty, span)?;

            expect_operand(function, *ty, value.ty, span)?;
            let flow = emitter.emit_expr(value, span)?;
            if flow.can_fall_through() {
                emitter.local_set(index);
            }
            emitter.initialize(*id);

            Ok(flow)
        }

        TypedStatementKind::Assignment { target, value } => {
            place::emit_assignment(target, value, emitter, span)
        }

        TypedStatementKind::CompoundAssignment { target, op, value } => {
            place::emit_compound_assignment(target, op, value, emitter, span)
        }

        TypedStatementKind::Expression(expr) => {
            let flow = emitter.emit_expr(expr, span)?;

            if matches!(expr.ty, ExprResult::Value(_)) {
                emitter.instruction(&Instruction::Drop);
            }

            Ok(flow)
        }

        TypedStatementKind::Return(value) => {
            let flow = match value {
                Some(_) if emitter.return_type() == Type::Void => {
                    return Err(CodegenError::UnexpectedReturnValue {
                        function: function.into(),
                        span,
                    });
                }

                Some(expr) => {
                    expect_operand(function, emitter.return_type(), expr.ty, span)?;
                    emitter.emit_expr(expr, span)?
                }

                None => {
                    expect_type(function, emitter.return_type(), Type::Void, span)?;
                    Flow::FALLTHROUGH
                }
            };

            emitter.instruction(&Instruction::Return);

            Ok(flow.then(Flow::RETURN))
        }

        TypedStatementKind::If { cond, then, else_ } => {
            control_flow::emit_if(cond, then, else_.as_ref(), emitter, span)
        }

        TypedStatementKind::While {
            id,
            cond,
            body: block,
        } => control_flow::emit_while(*id, cond, block, emitter, span),

        TypedStatementKind::Break { target } => {
            let context = emitter.loop_context(*target, span)?;
            emitter.branch(context.break_label)?;

            Ok(Flow::BREAK)
        }

        TypedStatementKind::Continue { target } => {
            let context = emitter.loop_context(*target, span)?;
            emitter.branch(context.continue_label)?;

            Ok(Flow::CONTINUE)
        }
    }
}
