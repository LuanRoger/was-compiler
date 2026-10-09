use crate::{
    ast::Type,
    typed_ast::{TypedBlock, TypedStatement, TypedStatementKind},
};
use wasm_encoder::Instruction;

use super::{CodegenError, control_flow, expect_type, function::BodyEmitter, values};

pub(super) fn emit_block(
    block: &TypedBlock,
    emitter: &mut BodyEmitter<'_>,
) -> Result<bool, CodegenError> {
    let mut always_returns = false;

    for statement in block {
        let returns = emit(statement, emitter)?;

        always_returns |= returns;
    }

    Ok(always_returns)
}

pub(super) fn emit(
    statement: &TypedStatement,
    emitter: &mut BodyEmitter<'_>,
) -> Result<bool, CodegenError> {
    let function = emitter.name();
    let span = statement.span;

    match &statement.kind {
        TypedStatementKind::Let { id, ty, value, .. } => {
            let index = emitter.declaration(*id, *ty, span)?;

            expect_type(function, *ty, value.ty, span)?;
            emitter.emit_expr(value, span)?;
            emitter.local_set(index);
            emitter.initialize(*id);

            Ok(false)
        }

        TypedStatementKind::Assignment { target, value } => {
            values::emit_assignment(target, value, emitter, span)?;

            Ok(false)
        }

        TypedStatementKind::Expression(expr) => {
            emitter.emit_expr(expr, span)?;

            if expr.ty != Type::Void {
                emitter.instruction(&Instruction::Drop);
            }

            Ok(false)
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

            Ok(true)
        }

        TypedStatementKind::If { cond, then, else_ } => {
            control_flow::emit_if(cond, then, else_.as_ref(), emitter, span)
        }

        TypedStatementKind::While { cond, body: block } => {
            control_flow::emit_while(cond, block, emitter, span)
        }
    }
}
