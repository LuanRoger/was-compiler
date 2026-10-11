use std::{collections::HashMap, ops::ControlFlow};

use crate::{
    ast::Type,
    typed_ast::{
        BuiltinFunction, FunctionRef, TypedExpr, TypedExprKind, TypedProgram, TypedStatement,
        TypedStatementKind,
    },
};
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use super::{
    CodegenError,
    abi::{self, Signature},
    walker::{self, Visitor},
};

pub(super) const PAGE_BYTES: u64 = 65_536;
const PAGE_ALIGN_LOG2: u32 = PAGE_BYTES.trailing_zeros();
pub(super) const MAX_PAGES: u64 = 65_535;
pub(super) const MAX_BYTES: u64 = MAX_PAGES * PAGE_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Helper {
    Alloc,
    ValidateString,
    ValidateArray,
    ArrayAddress,
    ArrayLen,
    StringEqual,
    ArrayEqual,
}

impl Helper {
    pub(super) const ALL: [Self; 7] = [
        Self::Alloc,
        Self::ValidateString,
        Self::ValidateArray,
        Self::ArrayAddress,
        Self::ArrayLen,
        Self::StringEqual,
        Self::ArrayEqual,
    ];

    pub(super) fn index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|helper| *helper == self)
            .expect("memory helper registered in canonical order") as u32
    }

    pub(super) fn signature(self) -> Signature {
        use ValType::{F64, I32};

        let (params, result) = match self {
            Self::Alloc => (vec![I32], I32),
            Self::ValidateString => (vec![I32], I32),
            Self::ValidateArray => (vec![I32], I32),
            Self::ArrayAddress => (vec![I32, F64], I32),
            Self::ArrayLen => (vec![I32], F64),
            Self::StringEqual => (vec![I32, I32], I32),
            Self::ArrayEqual => (vec![I32, I32], I32),
        };

        Signature {
            params,
            result: Some(result),
        }
    }
}

pub(super) fn helpers() -> Vec<(Helper, Signature)> {
    Helper::ALL
        .into_iter()
        .map(|helper| (helper, helper.signature()))
        .collect()
}

pub struct MemoryPlan {
    pub data: Vec<u8>,
    pub heap_start: u32,
    strings: HashMap<String, u32>,
}

impl MemoryPlan {
    pub fn new(program: &TypedProgram) -> Result<Option<Self>, CodegenError> {
        let mut plan = Self {
            data: vec![0; abi::MIN_ADDRESS as usize],
            heap_start: abi::MIN_ADDRESS,
            strings: HashMap::new(),
        };
        let mut used = false;

        for host in &program.host_functions {
            used |= memory_type(host.return_type) || host.params.iter().copied().any(memory_type);
        }

        for function in &program.functions {
            used |= memory_type(function.return_type)
                || function.params.iter().any(|param| memory_type(param.ty));

            let mut visitor = MemoryVisitor {
                plan: &mut plan,
                used: &mut used,
            };

            if let ControlFlow::Break(error) = walker::walk_block(&function.body, &mut visitor) {
                return Err(error);
            }
        }

        if !used {
            return Ok(None);
        }

        plan.heap_start = aligned_offset(plan.data.len() as u64)?;

        Ok(Some(plan))
    }

    pub fn string(&self, value: &str) -> Option<u32> {
        self.strings.get(value).copied()
    }

    fn intern(&mut self, value: &str) -> Result<(), CodegenError> {
        if self.strings.contains_key(value) {
            return Ok(());
        }

        let length = u32::try_from(value.len()).map_err(|_| CodegenError::LimitExceeded {
            what: "string byte length",
        })?;
        let handle = aligned_offset(self.data.len() as u64)?;
        let payload = u64::from(handle) + u64::from(abi::DESCRIPTOR_SIZE);
        let end = payload + u64::from(length);

        aligned_offset(end)?;

        let end = usize::try_from(end).map_err(|_| CodegenError::LimitExceeded {
            what: "static memory data",
        })?;

        self.data
            .try_reserve(end - self.data.len())
            .map_err(|_| CodegenError::LimitExceeded {
                what: "static memory data",
            })?;
        self.data.resize(handle as usize, 0);
        self.data.extend_from_slice(&(payload as u32).to_le_bytes());
        self.data.extend_from_slice(&length.to_le_bytes());
        self.data.extend_from_slice(value.as_bytes());
        self.strings.insert(value.to_owned(), handle);

        Ok(())
    }
}

struct MemoryVisitor<'a> {
    plan: &'a mut MemoryPlan,
    used: &'a mut bool,
}

impl Visitor for MemoryVisitor<'_> {
    type Break = CodegenError;

    fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<CodegenError> {
        if let TypedStatementKind::Let { ty, .. } = &statement.kind {
            *self.used |= memory_type(*ty);
        }

        ControlFlow::Continue(())
    }

    fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<CodegenError> {
        *self.used |= expr.ty.value_type().is_some_and(memory_type);

        match &expr.kind {
            TypedExprKind::String(value) => {
                *self.used = true;

                if let Err(error) = self.plan.intern(value) {
                    return ControlFlow::Break(error);
                }
            }

            TypedExprKind::Call { func, .. } => {
                *self.used |= matches!(func, FunctionRef::Builtin(BuiltinFunction::Len));
            }

            TypedExprKind::Array(_) | TypedExprKind::Index { .. } | TypedExprKind::Range { .. } => {
                *self.used = true
            }

            _ => {}
        }

        ControlFlow::Continue(())
    }
}

fn memory_type(ty: Type) -> bool {
    matches!(ty, Type::String | Type::NumberArray | Type::Range)
}

fn aligned_offset(value: u64) -> Result<u32, CodegenError> {
    let mask = u64::from(abi::ALLOCATION_ALIGNMENT - 1);
    let aligned = value.checked_add(mask).map(|value| value & !mask);

    match aligned {
        Some(aligned) if aligned <= MAX_BYTES => Ok(aligned as u32),
        _ => Err(CodegenError::LimitExceeded {
            what: "static memory layout",
        }),
    }
}

pub(super) fn emit(helper: Helper, first_function: u32) -> Function {
    match helper {
        Helper::Alloc => emit_alloc(),
        Helper::ValidateString => emit_validate(false),
        Helper::ValidateArray => emit_validate(true),
        Helper::ArrayAddress => emit_array_address(first_function),
        Helper::ArrayLen => emit_array_len(first_function),
        Helper::StringEqual => emit_string_equal(first_function),
        Helper::ArrayEqual => emit_array_equal(first_function),
    }
}

fn instructions(body: &mut Function, instructions: &[Instruction<'_>]) {
    for instruction in instructions {
        body.instruction(instruction);
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

fn memory_bytes(body: &mut Function) {
    instructions(
        body,
        &[
            Instruction::MemorySize(0),
            Instruction::I64ExtendI32U,
            Instruction::I64Const(i64::from(PAGE_ALIGN_LOG2)),
            Instruction::I64Shl,
        ],
    );
}

fn header(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: abi::DESCRIPTOR_FIELD_ALIGN_LOG2,
        memory_index: 0,
    }
}

fn emit_alloc() -> Function {
    use Instruction::*;

    let mut body = Function::new([(4, ValType::I64)]);

    instructions(
        &mut body,
        &[
            GlobalGet(0),
            I64ExtendI32U,
            LocalTee(1),
            I64Const(i64::from(abi::MIN_ADDRESS)),
            I64LtU,
            LocalGet(1),
            I64Const(i64::from(abi::ALLOCATION_ALIGNMENT - 1)),
            I64And,
            I64Const(0),
            I64Ne,
            I32Or,
        ],
    );
    trap_if(&mut body);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            I64ExtendI32U,
            I64Const(i64::from(abi::ALLOCATION_ALIGNMENT - 1)),
            I64Add,
            I64Const(-i64::from(abi::ALLOCATION_ALIGNMENT)),
            I64And,
            LocalTee(2),
            I64Eqz,
            If(BlockType::Empty),
            I64Const(i64::from(abi::MIN_ALLOCATION_BYTES)),
            LocalSet(2),
            End,
            LocalGet(1),
            LocalGet(2),
            I64Add,
            LocalTee(3),
            I64Const(MAX_BYTES as i64),
            I64GtU,
        ],
    );
    trap_if(&mut body);

    instructions(
        &mut body,
        &[
            LocalGet(3),
            I64Const((PAGE_BYTES - 1) as i64),
            I64Add,
            I64Const(i64::from(PAGE_ALIGN_LOG2)),
            I64ShrU,
            LocalTee(4),
            MemorySize(0),
            I64ExtendI32U,
            I64GtU,
            If(BlockType::Empty),
            LocalGet(4),
            MemorySize(0),
            I64ExtendI32U,
            I64Sub,
            I32WrapI64,
            MemoryGrow(0),
            I32Const(-1),
            I32Eq,
        ],
    );
    trap_if(&mut body);

    instructions(
        &mut body,
        &[
            End,
            LocalGet(3),
            I32WrapI64,
            GlobalSet(0),
            LocalGet(1),
            I32WrapI64,
            End,
        ],
    );

    body
}

fn emit_validate(array: bool) -> Function {
    use Instruction::*;

    let mut body = Function::new([(1, ValType::I64)]);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            I32Const(abi::MIN_ADDRESS as i32),
            I32LtU,
            LocalGet(0),
            I32Const((abi::DESCRIPTOR_ALIGNMENT - 1) as i32),
            I32And,
            I32Const(0),
            I32Ne,
            I32Or,
        ],
    );
    trap_if(&mut body);

    memory_bytes(&mut body);
    instructions(
        &mut body,
        &[
            LocalSet(1),
            LocalGet(0),
            I64ExtendI32U,
            I64Const(i64::from(abi::DESCRIPTOR_SIZE)),
            I64Add,
            LocalGet(1),
            I64GtU,
        ],
    );
    trap_if(&mut body);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            I32Const(abi::MIN_ADDRESS as i32),
            I32LtU,
        ],
    );
    trap_if(&mut body);

    if array {
        instructions(
            &mut body,
            &[
                LocalGet(0),
                I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
                I32Const((abi::NUMBER_ELEMENT_ALIGNMENT - 1) as i32),
                I32And,
                I32Const(0),
                I32Ne,
            ],
        );
        trap_if(&mut body);
    }

    instructions(
        &mut body,
        &[
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            I64ExtendI32U,
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            I64ExtendI32U,
            I64Const(i64::from(if array {
                abi::NUMBER_ELEMENT_SIZE
            } else {
                abi::STRING_ELEMENT_SIZE
            })),
            I64Mul,
            I64Add,
            LocalGet(1),
            I64GtU,
        ],
    );
    trap_if(&mut body);

    instructions(&mut body, &[LocalGet(0), End]);

    body
}

fn emit_array_address(first_function: u32) -> Function {
    use Instruction::*;

    let mut body = Function::new([]);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            Call(first_function + Helper::ValidateArray.index()),
            LocalSet(0),
            LocalGet(1),
            F64Const(0.0.into()),
            F64Ge,
            LocalGet(1),
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            F64ConvertI32U,
            F64Lt,
            I32And,
            LocalGet(1),
            LocalGet(1),
            F64Trunc,
            F64Eq,
            I32And,
            I32Eqz,
        ],
    );
    trap_if(&mut body);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            I64ExtendI32U,
            LocalGet(1),
            I64TruncF64U,
            I64Const(i64::from(abi::NUMBER_ELEMENT_SIZE)),
            I64Mul,
            I64Add,
            I32WrapI64,
            End,
        ],
    );

    body
}

fn emit_array_len(first_function: u32) -> Function {
    use Instruction::*;

    let mut body = Function::new([]);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            Call(first_function + Helper::ValidateArray.index()),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            F64ConvertI32U,
            End,
        ],
    );

    body
}

fn emit_string_equal(first_function: u32) -> Function {
    use Instruction::*;

    let byte = MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    };
    let mut body = Function::new([(4, ValType::I32)]);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            Call(first_function + Helper::ValidateString.index()),
            LocalSet(0),
            LocalGet(1),
            Call(first_function + Helper::ValidateString.index()),
            LocalSet(1),
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            LocalTee(4),
            LocalGet(1),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            I32Ne,
            If(BlockType::Empty),
            I32Const(0),
            Return,
            End,
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            LocalSet(2),
            LocalGet(1),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            LocalSet(3),
            Block(BlockType::Empty),
            Loop(BlockType::Empty),
            LocalGet(5),
            LocalGet(4),
            I32GeU,
            BrIf(1),
            LocalGet(2),
            LocalGet(5),
            I32Add,
            I32Load8U(byte),
            LocalGet(3),
            LocalGet(5),
            I32Add,
            I32Load8U(byte),
            I32Ne,
            If(BlockType::Empty),
            I32Const(0),
            Return,
            End,
            LocalGet(5),
            I32Const(1),
            I32Add,
            LocalSet(5),
            Br(0),
            End,
            End,
            I32Const(1),
            End,
        ],
    );

    body
}

fn emit_array_equal(first_function: u32) -> Function {
    use Instruction::*;

    let element = MemArg {
        offset: 0,
        align: abi::NUMBER_ELEMENT_ALIGN_LOG2,
        memory_index: 0,
    };
    let mut body = Function::new([(4, ValType::I32)]);

    instructions(
        &mut body,
        &[
            LocalGet(0),
            Call(first_function + Helper::ValidateArray.index()),
            LocalSet(0),
            LocalGet(1),
            Call(first_function + Helper::ValidateArray.index()),
            LocalSet(1),
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            LocalTee(4),
            LocalGet(1),
            I32Load(header(abi::DESCRIPTOR_LENGTH_OFFSET)),
            I32Ne,
            If(BlockType::Empty),
            I32Const(0),
            Return,
            End,
            LocalGet(0),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            LocalSet(2),
            LocalGet(1),
            I32Load(header(abi::DESCRIPTOR_DATA_OFFSET)),
            LocalSet(3),
            Block(BlockType::Empty),
            Loop(BlockType::Empty),
            LocalGet(5),
            LocalGet(4),
            I32GeU,
            BrIf(1),
            LocalGet(2),
            LocalGet(5),
            I32Const(abi::NUMBER_ELEMENT_SIZE as i32),
            I32Mul,
            I32Add,
            F64Load(element),
            LocalGet(3),
            LocalGet(5),
            I32Const(abi::NUMBER_ELEMENT_SIZE as i32),
            I32Mul,
            I32Add,
            F64Load(element),
            F64Ne,
            If(BlockType::Empty),
            I32Const(0),
            Return,
            End,
            LocalGet(5),
            I32Const(1),
            I32Add,
            LocalSet(5),
            Br(0),
            End,
            End,
            I32Const(1),
            End,
        ],
    );

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        host::{HostFunction, HostFunctionId},
        tokens::Span,
        typed_ast::{FunctionId, TypedBlock, TypedFunction, TypedStatement, VariableId},
    };
    use wasm_encoder::{
        CodeSection, ConstExpr, FunctionSection, GlobalSection, GlobalType, ImportSection,
        MemorySection, MemoryType, Module, TypeSection,
    };

    fn expr(kind: TypedExprKind, ty: Type) -> TypedExpr {
        TypedExpr {
            kind,
            ty: ty.into(),
        }
    }

    fn string(value: &str) -> TypedExpr {
        expr(TypedExprKind::String(value.to_owned()), Type::String)
    }

    fn statement(kind: TypedStatementKind) -> TypedStatement {
        TypedStatement {
            kind,
            span: Span::new(0, 0),
        }
    }

    fn program(body: TypedBlock) -> TypedProgram {
        TypedProgram {
            host_functions: vec![],
            functions: vec![TypedFunction {
                id: FunctionId(0),
                name: "test".to_owned(),
                params: vec![],
                return_type: Type::Void,
                body,
                span: Span::new(0, 0),
            }],
        }
    }

    #[test]
    fn scalar_only_program_has_no_memory_plan() {
        let program = program(vec![statement(TypedStatementKind::Expression(expr(
            TypedExprKind::Number("1".to_owned()),
            Type::Number,
        )))]);

        assert!(MemoryPlan::new(&program).unwrap().is_none());
    }

    #[test]
    fn strings_are_interned_in_preorder_with_byte_lengths() {
        let program = program(vec![statement(TypedStatementKind::While {
            id: crate::typed_ast::LoopId(0),
            cond: Box::new(string("é")),
            body: vec![statement(TypedStatementKind::If {
                cond: Box::new(string("")),
                then: vec![statement(TypedStatementKind::Return(Some(string("é"))))],
                else_: Some(vec![statement(TypedStatementKind::Expression(string(
                    "last",
                )))]),
            })],
        })]);
        let plan = MemoryPlan::new(&program).unwrap().unwrap();

        assert_eq!(plan.string("é"), Some(8));
        assert_eq!(plan.string(""), Some(24));
        assert_eq!(plan.string("last"), Some(32));
        assert_eq!(plan.string("missing"), None);
        assert_eq!(&plan.data[..8], &[0; 8]);
        assert_eq!(&plan.data[8..12], &16_u32.to_le_bytes());
        assert_eq!(&plan.data[12..16], &2_u32.to_le_bytes());
        assert_eq!(&plan.data[16..18], "é".as_bytes());
        assert_eq!(&plan.data[18..24], &[0; 6]);
        assert_eq!(&plan.data[24..28], &32_u32.to_le_bytes());
        assert_eq!(&plan.data[28..32], &0_u32.to_le_bytes());
        assert_eq!(plan.data.len(), 44);
        assert_eq!(plan.heap_start, 48);
    }

    #[test]
    fn signatures_locals_expression_types_and_kinds_require_memory() {
        let mut host_program = program(vec![]);
        host_program.host_functions.push(HostFunction {
            id: HostFunctionId(0),
            name: "host".to_owned(),
            module: "env".to_owned(),
            field: "host".to_owned(),
            params: vec![Type::String],
            return_type: Type::Void,
        });

        assert!(MemoryPlan::new(&host_program).unwrap().is_some());

        let mut return_program = program(vec![]);
        return_program.functions[0].return_type = Type::NumberArray;

        assert!(MemoryPlan::new(&return_program).unwrap().is_some());

        let local_program = program(vec![statement(TypedStatementKind::Let {
            id: VariableId(0),
            name: "local".to_owned(),
            ty: Type::String,
            value: expr(TypedExprKind::Bool(false), Type::Bool),
        })]);

        assert!(MemoryPlan::new(&local_program).unwrap().is_some());

        for value in [
            expr(TypedExprKind::Bool(false), Type::NumberArray),
            expr(TypedExprKind::Array(vec![]), Type::Number),
            expr(
                TypedExprKind::Index {
                    expr: Box::new(expr(TypedExprKind::Bool(false), Type::Bool)),
                    index: Box::new(expr(TypedExprKind::Bool(false), Type::Bool)),
                },
                Type::Number,
            ),
            expr(
                TypedExprKind::Call {
                    func: FunctionRef::Builtin(BuiltinFunction::Len),
                    name: "len".to_owned(),
                    args: vec![],
                },
                Type::Number,
            ),
        ] {
            let program = program(vec![statement(TypedStatementKind::Expression(value))]);
            let plan = MemoryPlan::new(&program).unwrap().unwrap();

            assert_eq!(plan.data, vec![0; 8]);
            assert_eq!(plan.heap_start, 8);
        }
    }

    #[test]
    fn static_layout_limits_are_checked_before_narrowing() {
        assert_eq!(aligned_offset(MAX_BYTES).unwrap(), MAX_BYTES as u32);
        assert_eq!(aligned_offset(MAX_BYTES - 7).unwrap(), MAX_BYTES as u32);
        assert!(aligned_offset(MAX_BYTES + 1).is_err());
        assert!(aligned_offset(u64::MAX).is_err());
    }

    #[test]
    fn helper_indices_and_signatures_are_fixed() {
        use ValType::{F64, I32};

        let expected = [
            (Helper::Alloc, 0, vec![I32], I32),
            (Helper::ValidateString, 1, vec![I32], I32),
            (Helper::ValidateArray, 2, vec![I32], I32),
            (Helper::ArrayAddress, 3, vec![I32, F64], I32),
            (Helper::ArrayLen, 4, vec![I32], F64),
            (Helper::StringEqual, 5, vec![I32, I32], I32),
            (Helper::ArrayEqual, 6, vec![I32, I32], I32),
        ];
        let actual = helpers();

        assert_eq!(actual.len(), 7);

        for ((actual_helper, actual_signature), (helper, index, params, result)) in
            actual.into_iter().zip(expected)
        {
            let signature = Signature {
                params,
                result: Some(result),
            };

            assert_eq!(actual_helper, helper);
            assert_eq!(helper.index(), index);
            assert_eq!(helper.signature(), signature);
            assert_eq!(actual_signature, signature);
        }
    }

    #[test]
    fn descriptor_layout_and_constants_are_fixed() {
        assert_eq!(abi::DESCRIPTOR_SIZE, 8);
        assert_eq!(abi::DESCRIPTOR_DATA_OFFSET, 0);
        assert_eq!(abi::DESCRIPTOR_LENGTH_OFFSET, 4);
        assert_eq!(abi::DESCRIPTOR_ALIGNMENT, 8);
        assert_eq!(abi::DESCRIPTOR_FIELD_ALIGN_LOG2, 2);
        assert_eq!(abi::NUMBER_ELEMENT_SIZE, 8);
        assert_eq!(abi::NUMBER_ELEMENT_ALIGNMENT, 8);
        assert_eq!(abi::NUMBER_ELEMENT_ALIGN_LOG2, 3);
        assert_eq!(abi::STRING_ELEMENT_SIZE, 1);
        assert_eq!(abi::MIN_ADDRESS, 8);
        assert_eq!(abi::ALLOCATION_ALIGNMENT, 8);
        assert_eq!(abi::MIN_ALLOCATION_BYTES, 8);
        assert_eq!(PAGE_BYTES, 65_536);
        assert_eq!(PAGE_ALIGN_LOG2, 16);

        let program = program(vec![statement(TypedStatementKind::Expression(string("é")))]);
        let plan = MemoryPlan::new(&program).unwrap().unwrap();

        assert_eq!(plan.string("é"), Some(8));
        assert_eq!(plan.heap_start, 24);
        assert_eq!(
            plan.data,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0, 2, 0, 0, 0, 0xc3, 0xa9]
        );
    }

    #[test]
    fn helper_signatures_and_bodies_validate_with_a_function_offset() {
        let helpers = helpers();
        let mut module = Module::new();
        let mut types = TypeSection::new();
        let mut imports = ImportSection::new();
        let mut functions = FunctionSection::new();
        let mut memory = MemorySection::new();
        let mut globals = GlobalSection::new();
        let mut code = CodeSection::new();
        let expected = [
            (vec![ValType::I32], ValType::I32),
            (vec![ValType::I32], ValType::I32),
            (vec![ValType::I32], ValType::I32),
            (vec![ValType::I32, ValType::F64], ValType::I32),
            (vec![ValType::I32], ValType::F64),
            (vec![ValType::I32, ValType::I32], ValType::I32),
            (vec![ValType::I32, ValType::I32], ValType::I32),
        ];

        assert_eq!(helpers.len(), expected.len());

        for (index, ((helper, signature), (params, result))) in
            helpers.iter().zip(expected).enumerate()
        {
            assert_eq!(helper.index(), index as u32);
            assert_eq!(signature.params, params);
            assert_eq!(signature.result, Some(result));

            types.ty().function(
                signature.params.iter().copied(),
                signature.result.iter().copied(),
            );
            functions.function(index as u32);
            code.function(&emit(*helper, 1));
        }

        imports.import("env", "unused", wasm_encoder::EntityType::Function(0));
        memory.memory(MemoryType {
            minimum: 1,
            maximum: Some(MAX_PAGES),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(8),
        );
        module.section(&types);
        module.section(&imports);
        module.section(&functions);
        module.section(&memory);
        module.section(&globals);
        module.section(&code);

        wasmparser::Validator::new()
            .validate_all(&module.finish())
            .unwrap();
    }
}
