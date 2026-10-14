use std::ops::ControlFlow;

use crate::{
    ast::{BinaryOp, Type},
    flow::Flow,
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind, TypedProgram, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use super::{
    CodegenError,
    abi::{self, Signature},
    expect_operand,
    function::BodyEmitter,
    indices::FunctionIndex,
    memory,
    walker::{self, Visitor},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Helper {
    Construct,
    Validate,
    Contains,
}

impl Helper {
    pub const ALL: [Self; 3] = [Self::Construct, Self::Validate, Self::Contains];

    pub fn index(self) -> u32 {
        let index = Self::ALL
            .iter()
            .position(|helper| *helper == self)
            .expect("range helper registered in canonical order");

        u32::try_from(index).expect("range helper index fits u32")
    }

    pub fn signature(self) -> Signature {
        Signature {
            params: match self {
                Self::Construct => vec![ValType::F64, ValType::F64, ValType::F64, ValType::I32],
                Self::Validate => vec![ValType::I32],
                Self::Contains => vec![ValType::F64, ValType::I32],
            },
            result: Some(ValType::I32),
        }
    }
}

pub(super) fn helpers(program: &TypedProgram) -> &'static [Helper] {
    struct Membership;
    impl Visitor<'_> for Membership {
        type Break = ();

        fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<()> {
            if matches!(
                expr.kind,
                TypedExprKind::Binary {
                    op: BinaryOp::In,
                    ..
                }
            ) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }

    if program
        .functions
        .iter()
        .any(|function| walker::walk_function(function, &mut Membership).is_break())
    {
        &Helper::ALL
    } else if needed(program) {
        &Helper::ALL[..2]
    } else {
        &[]
    }
}

pub(super) fn needed(program: &TypedProgram) -> bool {
    struct Ranges;
    impl Visitor<'_> for Ranges {
        type Break = ();
        fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<()> {
            if let TypedStatementKind::Closure(closure) = &statement.kind
                && (closure.return_type == Type::Range
                    || closure.params.iter().any(|p| p.ty == Type::Range))
            {
                return ControlFlow::Break(());
            }
            if matches!(
                statement.kind,
                TypedStatementKind::Let {
                    ty: Type::Range,
                    ..
                }
            ) || matches!(&statement.kind, TypedStatementKind::For { iterable, .. }
                if iterable.ty != Type::NumberArray)
            {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
        fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<()> {
            if expr.ty.value_type() == Some(Type::Range)
                || matches!(
                    expr.kind,
                    TypedExprKind::Range { .. }
                        | TypedExprKind::Binary {
                            op: BinaryOp::In,
                            ..
                        }
                )
            {
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    program.functions.iter().any(|function| {
        function.return_type == Type::Range
            || function.params.iter().any(|param| param.ty == Type::Range)
            || walker::walk_function(function, &mut Ranges).is_break()
    })
}

pub(super) fn emit_construction(
    start: &TypedExpr,
    end: &TypedExpr,
    step: Option<&TypedExpr>,
    inclusive: bool,
    body: &mut BodyEmitter<'_>,
    span: Span,
) -> Result<Flow, CodegenError> {
    let mut flow = Flow::FALLTHROUGH;
    // The helper's parameters cache every input before its first validation.
    for operand in [Some(start), Some(end), step].into_iter().flatten() {
        expect_operand(body.name(), Type::Number, operand.ty, span)?;
        flow = flow.then(body.emit_expr(operand, span)?);
    }
    if step.is_none() {
        body.instruction(&Instruction::F64Const(1.0.into()));
    }
    body.instruction(&Instruction::I32Const(i32::from(inclusive)));
    body.call(body.range_helper(Helper::Construct));
    Ok(flow)
}

pub(super) fn emit(helper: Helper, allocator: FunctionIndex, validator: FunctionIndex) -> Function {
    match helper {
        Helper::Construct => construct(allocator),
        Helper::Validate => validate(),
        Helper::Contains => contains(validator),
    }
}

fn instructions(body: &mut Function, ops: &[Instruction<'_>]) {
    for op in ops {
        body.instruction(op);
    }
}

fn trap_if(body: &mut Function) {
    instructions(
        body,
        &[
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
        ],
    );
}

fn field(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

fn construct(allocator: FunctionIndex) -> Function {
    let mut body = Function::new([(1, ValType::I32)]);
    for index in 0..3 {
        // Negated <= rejects NaN as well as infinity and out-of-domain values.
        instructions(
            &mut body,
            &[
                Instruction::LocalGet(index),
                Instruction::F64Abs,
                Instruction::F64Const((abi::RANGE_SAFE_INTEGER as f64).into()),
                Instruction::F64Le,
                Instruction::I32Eqz,
            ],
        );
        trap_if(&mut body);
        instructions(
            &mut body,
            &[
                Instruction::LocalGet(index),
                Instruction::F64Trunc,
                Instruction::LocalGet(index),
                Instruction::F64Ne,
            ],
        );
        trap_if(&mut body);
    }
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(2),
            Instruction::F64Const(0.0.into()),
            Instruction::F64Eq,
        ],
    );
    trap_if(&mut body);
    instructions(
        &mut body,
        &[
            Instruction::I32Const(abi::RANGE_SIZE as i32),
            Instruction::Call(allocator.raw()),
            Instruction::LocalSet(4),
        ],
    );
    for (index, offset) in [
        abi::RANGE_START_OFFSET,
        abi::RANGE_END_OFFSET,
        abi::RANGE_STEP_OFFSET,
    ]
    .into_iter()
    .enumerate()
    {
        instructions(
            &mut body,
            &[
                Instruction::LocalGet(4),
                Instruction::LocalGet(u32::try_from(index).expect("three range fields")),
                // Domain/integrality checks precede this exact conversion; it canonicalizes -0.
                Instruction::I64TruncF64S,
                Instruction::I64Store(field(offset, abi::RANGE_INTEGER_ALIGN_LOG2)),
            ],
        );
    }
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(4),
            Instruction::LocalGet(3),
            Instruction::I32Store(field(
                abi::RANGE_INCLUSIVE_OFFSET,
                abi::RANGE_FLAG_ALIGN_LOG2,
            )),
            Instruction::LocalGet(4),
            Instruction::I32Const(0),
            Instruction::I32Store(field(
                abi::RANGE_RESERVED_OFFSET,
                abi::RANGE_FLAG_ALIGN_LOG2,
            )),
            Instruction::LocalGet(4),
            Instruction::End,
        ],
    );
    body
}

fn contains(validator: FunctionIndex) -> Function {
    // Candidate/start/end/step are bounded by M = 2^53 - 1. The largest
    // difference is 2M < 2^54, well inside signed i64. A validated nonzero
    // step cannot be i64::MIN, so signed remainder cannot overflow or divide by zero.
    let mut body = Function::new([(4, ValType::I64), (1, ValType::I32)]);
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(1),
            Instruction::Call(validator.raw()),
            Instruction::Drop,
            Instruction::LocalGet(0),
            Instruction::F64Abs,
            Instruction::F64Const((abi::RANGE_SAFE_INTEGER as f64).into()),
            Instruction::F64Le,
            Instruction::LocalGet(0),
            Instruction::F64Trunc,
            Instruction::LocalGet(0),
            Instruction::F64Eq,
            Instruction::I32And,
            Instruction::I32Eqz,
            Instruction::If(BlockType::Empty),
            Instruction::I32Const(0),
            Instruction::Return,
            Instruction::End,
            Instruction::LocalGet(0),
            Instruction::I64TruncF64S,
            Instruction::LocalSet(2),
        ],
    );
    for (local, offset) in [
        (3, abi::RANGE_START_OFFSET),
        (4, abi::RANGE_END_OFFSET),
        (5, abi::RANGE_STEP_OFFSET),
    ] {
        instructions(
            &mut body,
            &[
                Instruction::LocalGet(1),
                Instruction::I64Load(field(offset, abi::RANGE_INTEGER_ALIGN_LOG2)),
                Instruction::LocalSet(local),
            ],
        );
    }
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(1),
            Instruction::I32Load(field(
                abi::RANGE_INCLUSIVE_OFFSET,
                abi::RANGE_FLAG_ALIGN_LOG2,
            )),
            Instruction::LocalSet(6),
            Instruction::LocalGet(5),
            Instruction::I64Const(0),
            Instruction::I64GtS,
            Instruction::If(BlockType::Result(ValType::I32)),
            Instruction::LocalGet(2),
            Instruction::LocalGet(3),
            Instruction::I64GeS,
            Instruction::LocalGet(2),
            Instruction::LocalGet(4),
            Instruction::I64LtS,
            Instruction::LocalGet(2),
            Instruction::LocalGet(4),
            Instruction::I64Eq,
            Instruction::LocalGet(6),
            Instruction::I32And,
            Instruction::I32Or,
            Instruction::I32And,
            Instruction::Else,
            Instruction::LocalGet(2),
            Instruction::LocalGet(3),
            Instruction::I64LeS,
            Instruction::LocalGet(2),
            Instruction::LocalGet(4),
            Instruction::I64GtS,
            Instruction::LocalGet(2),
            Instruction::LocalGet(4),
            Instruction::I64Eq,
            Instruction::LocalGet(6),
            Instruction::I32And,
            Instruction::I32Or,
            Instruction::I32And,
            Instruction::End,
            Instruction::LocalGet(2),
            Instruction::LocalGet(3),
            Instruction::I64Sub,
            Instruction::LocalGet(5),
            Instruction::I64RemS,
            Instruction::I64Eqz,
            Instruction::I32And,
            Instruction::End,
        ],
    );
    body
}

fn validate() -> Function {
    let mut body = Function::new([]);
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(0),
            Instruction::I32Const(abi::MIN_ADDRESS as i32),
            Instruction::I32LtU,
        ],
    );
    trap_if(&mut body);
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(0),
            Instruction::I32Const((abi::RANGE_ALIGNMENT - 1) as i32),
            Instruction::I32And,
        ],
    );
    trap_if(&mut body);
    // Widen before adding the record size; a wrapping memory32 address is invalid.
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(0),
            Instruction::I64ExtendI32U,
            Instruction::I64Const(i64::from(abi::RANGE_SIZE)),
            Instruction::I64Add,
            Instruction::MemorySize(0),
            Instruction::I64ExtendI32U,
            Instruction::I64Const(i64::from(memory::PAGE_BYTES.trailing_zeros())),
            Instruction::I64Shl,
            Instruction::I64GtU,
        ],
    );
    trap_if(&mut body);
    for offset in [
        abi::RANGE_START_OFFSET,
        abi::RANGE_END_OFFSET,
        abi::RANGE_STEP_OFFSET,
    ] {
        for (bound, comparison) in [
            (-abi::RANGE_SAFE_INTEGER, Instruction::I64LtS),
            (abi::RANGE_SAFE_INTEGER, Instruction::I64GtS),
        ] {
            instructions(
                &mut body,
                &[
                    Instruction::LocalGet(0),
                    Instruction::I64Load(field(offset, abi::RANGE_INTEGER_ALIGN_LOG2)),
                    Instruction::I64Const(bound),
                    comparison,
                ],
            );
            trap_if(&mut body);
        }
    }
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(0),
            Instruction::I64Load(field(abi::RANGE_STEP_OFFSET, abi::RANGE_INTEGER_ALIGN_LOG2)),
            Instruction::I64Eqz,
        ],
    );
    trap_if(&mut body);
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(0),
            Instruction::I32Load(field(
                abi::RANGE_INCLUSIVE_OFFSET,
                abi::RANGE_FLAG_ALIGN_LOG2,
            )),
            Instruction::I32Const(1),
            Instruction::I32GtU,
        ],
    );
    trap_if(&mut body);
    // Reserved bytes are part of the canonical private record, not descriptor metadata.
    instructions(
        &mut body,
        &[
            Instruction::LocalGet(0),
            Instruction::I32Load(field(
                abi::RANGE_RESERVED_OFFSET,
                abi::RANGE_FLAG_ALIGN_LOG2,
            )),
        ],
    );
    trap_if(&mut body);
    instructions(&mut body, &[Instruction::LocalGet(0), Instruction::End]);
    body
}
