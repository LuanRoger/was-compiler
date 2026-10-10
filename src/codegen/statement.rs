use crate::{
    ast::Type,
    flow::Flow,
    typed_ast::{TypedBlock, TypedStatement, TypedStatementKind},
};
use wasm_encoder::Instruction;

use super::{CodegenError, control_flow, expect_type, function::BodyEmitter, place};

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

            expect_type(function, *ty, value.ty, span)?;
            emitter.emit_expr(value, span)?;
            emitter.local_set(index);
            emitter.initialize(*id);

            Ok(Flow::FALLTHROUGH)
        }

        TypedStatementKind::Assignment { target, value } => {
            place::emit_assignment(target, value, emitter, span)?;

            Ok(Flow::FALLTHROUGH)
        }

        TypedStatementKind::CompoundAssignment { target, op, value } => {
            place::emit_compound_assignment(target, op, value, emitter, span)?;

            Ok(Flow::FALLTHROUGH)
        }

        TypedStatementKind::Expression(expr) => {
            emitter.emit_expr(expr, span)?;

            if expr.ty != Type::Void {
                emitter.instruction(&Instruction::Drop);
            }

            Ok(Flow::FALLTHROUGH)
        }

        TypedStatementKind::Return(value) => {
            match value {
                Some(_) if emitter.return_type() == Type::Void => {
                    return Err(CodegenError::UnexpectedReturnValue {
                        function: function.into(),
                        span,
                    });
                }

                Some(expr) => {
                    expect_type(function, emitter.return_type(), expr.ty, span)?;
                    emitter.emit_expr(expr, span)?;
                }

                None => {
                    expect_type(function, emitter.return_type(), Type::Void, span)?;
                }
            }

            emitter.instruction(&Instruction::Return);

            Ok(Flow::RETURN)
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
