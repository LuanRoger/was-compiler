use crate::{
    ast::{BinaryOp, Type},
    flow::Flow,
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind},
};
use wasm_encoder::{Instruction, MemArg, ValType};

use super::{
    CodegenError, abi, expect_operand, expression, function::BodyEmitter, indices::LocalIndex,
    values,
};

enum Place<'a> {
    Local(LocalIndex),
    Element {
        base: &'a TypedExpr,
        index: &'a TypedExpr,
    },
}

impl<'a> Place<'a> {
    fn check(
        target: &'a TypedExpr,
        body: &mut BodyEmitter<'_>,
        span: Span,
    ) -> Result<Self, CodegenError> {
        match &target.kind {
            TypedExprKind::Identifier { id, .. } => {
                Ok(Self::Local(body.resolve_expr(*id, target.ty, span)?))
            }

            TypedExprKind::Index { expr: base, index } => {
                let TypedExprKind::Identifier { id, .. } = &base.kind else {
                    return Err(invalid_target(body, span));
                };

                values::expect_index_types(base, index, target.ty, body, span)?;
                body.resolve_expr(*id, base.ty, span)?;

                Ok(Self::Element { base, index })
            }

            _ => Err(invalid_target(body, span)),
        }
    }

    fn store(&self, body: &mut BodyEmitter<'_>) {
        match self {
            Self::Local(index) => body.local_set(*index),
            Self::Element { .. } => body.instruction(&Instruction::F64Store(element_arg())),
        }
    }
}

pub(super) fn emit_assignment(
    target: &TypedExpr,
    value: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    let place = Place::check(target, body, span)?;
    let ty = match &place {
        Place::Local(_) => target
            .ty
            .value_type()
            .expect("checked local has a storage type"),
        Place::Element { .. } => Type::Number,
    };
    expect_operand(body.name(), ty, value.ty, span)?;

    let target_flow = if let Place::Element { base, index } = &place {
        values::emit_address(base, index, body, span)?
    } else {
        Flow::FALLTHROUGH
    };

    let flow = target_flow.then(body.emit_expr(value, span)?);
    place.store(body);

    Ok(flow)
}

pub(super) fn emit_compound_assignment(
    target: &TypedExpr,
    op: &BinaryOp,
    value: &TypedExpr,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    let instruction = expression::arithmetic_instruction(op, body).map_err(|operator| {
        CodegenError::UnsupportedOperator {
            function: body.name().into(),
            operator,
            span,
        }
    })?;
    let place = Place::check(target, body, span)?;

    expect_operand(body.name(), Type::Number, target.ty, span)?;
    expect_operand(body.name(), Type::Number, value.ty, span)?;

    let target_flow = match &place {
        Place::Local(index) => {
            // Calls and nested expressions preserve this live f64 stack prefix.
            // No scratch local is needed to retain the pre-RHS value.
            body.local_get(*index);
            Flow::FALLTHROUGH
        }

        Place::Element { base, index } => {
            let address = body.temporary(ValType::I32);
            let old_value = body.temporary(ValType::F64);

            let flow = values::emit_address(base, index, body, span)?;
            body.local_set(address);
            body.local_get(address);
            body.instruction(&Instruction::F64Load(element_arg()));
            body.local_set(old_value);

            // Save the checked original address and old value before RHS effects,
            // including aliased element mutation and possible memory growth.
            body.local_get(address);
            body.local_get(old_value);
            flow
        }
    };

    let flow = target_flow.then(body.emit_expr(value, span)?);
    body.instruction(&instruction);
    place.store(body);

    Ok(flow)
}

fn invalid_target(body: &BodyEmitter<'_>, span: Span) -> CodegenError {
    CodegenError::InvalidAssignmentTarget {
        function: body.name().into(),
        span,
    }
}

fn element_arg() -> MemArg {
    MemArg {
        offset: 0,
        align: abi::NUMBER_ELEMENT_ALIGN_LOG2,
        memory_index: 0,
    }
}
