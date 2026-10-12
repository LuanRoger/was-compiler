use crate::{
    ast::Type,
    exports::ExportOptions,
    tokens::Span,
    typed_ast::{BuiltinFunction, ExprResult, FunctionRef, LoopId, TypedProgram, VariableId},
};
use thiserror::Error;
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, ImportSection, MemorySection, MemoryType, Module, TypeSection,
    ValType,
};

mod abi;
mod call;
mod control_flow;
mod expression;
mod function;
mod indices;
mod layout;
mod locals;
mod memory;
mod numeric;
mod place;
mod range;
mod statement;
mod temporaries;
mod validation;
mod values;
mod walker;

#[cfg(test)]
mod export_tests;

#[cfg(test)]
mod memory_tests;

#[cfg(test)]
mod range_tests;

#[cfg(test)]
mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodegenError {
    #[error("user function `{function}` has ID {actual}, expected source-order ID {expected}")]
    InvalidFunctionId {
        function: String,
        expected: usize,
        actual: usize,
    },

    #[error(
        "host function `{function}` has ID {actual}, expected registration-order ID {expected}"
    )]
    InvalidHostFunctionId {
        function: String,
        expected: usize,
        actual: usize,
    },

    #[error("duplicate user function export name `{name}`")]
    DuplicateExportName { name: String },

    #[error("unknown user function export `{name}`")]
    UnknownExport { name: String },

    #[error("duplicate requested export `{name}`")]
    DuplicateExportSelection { name: String },

    #[error("unsupported {position} ABI type {ty:?} in function `{function}`")]
    UnsupportedAbiType {
        function: String,
        position: &'static str,
        ty: Type,
    },

    #[error(
        "function `{function}` has a private range signature and cannot be exported; use explicit export selection (ExportOptions::Only or ExportOptions::None)"
    )]
    RangeExport { function: String },

    #[error("parameter {index} in function `{function}` cannot have type void")]
    VoidParameter { function: String, index: usize },

    #[error("unsupported {kind} statement in function `{function}` at {span:?}")]
    UnsupportedStatement {
        function: String,
        kind: &'static str,
        span: Span,
    },

    #[error("unsupported {kind} expression in function `{function}` at {span:?}")]
    UnsupportedExpression {
        function: String,
        kind: &'static str,
        span: Span,
    },

    #[error("unsupported operator `{operator}` in function `{function}` at {span:?}")]
    UnsupportedOperator {
        function: String,
        operator: &'static str,
        span: Span,
    },

    #[error("invalid number literal `{literal}` in function `{function}` at {span:?}")]
    InvalidNumberLiteral {
        function: String,
        literal: String,
        span: Span,
    },

    #[error(
        "number literal `{literal}` is outside the finite f64 range in function `{function}` at {span:?}"
    )]
    NumberLiteralOutOfRange {
        function: String,
        literal: String,
        span: Span,
    },

    #[error(
        "type mismatch in function `{function}`: expected {expected:?}, found {found:?} at {span:?}"
    )]
    TypeMismatch {
        function: String,
        expected: Type,
        found: Type,
        span: Span,
    },

    #[error(
        "expression result mismatch in function `{function}`: expected {expected:?}, found {found:?} at {span:?}"
    )]
    ExprResultMismatch {
        function: String,
        expected: ExprResult,
        found: ExprResult,
        span: Span,
    },

    #[error("value-producing if expression requires an else in function `{function}` at {span:?}")]
    MissingElse { function: String, span: Span },

    #[error("unknown function reference {reference:?} in function `{function}` at {span:?}")]
    UnknownFunctionReference {
        function: String,
        reference: FunctionRef,
        span: Span,
    },

    #[error(
        "call to {reference:?} in function `{function}` expects {expected} arguments, found {found} at {span:?}"
    )]
    ArgumentCountMismatch {
        function: String,
        reference: FunctionRef,
        expected: usize,
        found: usize,
        span: Span,
    },

    #[error("invalid normalized call: {reason} at {span:?}")]
    InvalidNormalizedCall { reason: &'static str, span: Span },

    #[error("unsupported built-in {builtin:?} in function `{function}` at {span:?}")]
    UnsupportedBuiltin {
        function: String,
        builtin: BuiltinFunction,
        span: Span,
    },

    #[error("duplicate variable ID {id:?} in function `{function}` at {span:?}")]
    DuplicateVariableId {
        function: String,
        id: VariableId,
        span: Span,
    },

    #[error("duplicate loop ID {id:?} in function `{function}` at {span:?}")]
    DuplicateLoopId {
        function: String,
        id: LoopId,
        span: Span,
    },

    #[error(
        "loop control target {target:?} is not the innermost active loop in function `{function}` at {span:?}"
    )]
    InvalidLoopTarget {
        function: String,
        target: LoopId,
        span: Span,
    },

    #[error("break value is not allowed in this loop in function `{function}` at {span:?}")]
    BreakValueNotAllowed { function: String, span: Span },

    #[error("value loop requires a break value in function `{function}` at {span:?}")]
    MissingBreakValue { function: String, span: Span },

    #[error("invalid break value in function `{function}` at {span:?}")]
    InvalidBreakValue { function: String, span: Span },

    #[error("unknown variable ID {id:?} in function `{function}` at {span:?}")]
    UnknownVariable {
        function: String,
        id: VariableId,
        span: Span,
    },

    #[error(
        "variable ID {id:?} is used before initialization in function `{function}` at {span:?}"
    )]
    UninitializedVariable {
        function: String,
        id: VariableId,
        span: Span,
    },

    #[error("iteration variable ID {id:?} is read-only in function `{function}` at {span:?}")]
    ReadOnlyVariable {
        function: String,
        id: VariableId,
        span: Span,
    },

    #[error("unsupported local type {ty:?} in function `{function}` at {span:?}")]
    UnsupportedLocalType {
        function: String,
        ty: Type,
        span: Span,
    },

    #[error(
        "assignment target must be a variable or its array element in function `{function}` at {span:?}"
    )]
    InvalidAssignmentTarget { function: String, span: Span },

    #[error("void function `{function}` cannot return a value at {span:?}")]
    UnexpectedReturnValue { function: String, span: Span },

    #[error("function `{function}` cannot produce its declared {return_type:?} result")]
    MissingReturnValue { function: String, return_type: Type },

    #[error("{what} exceeds the WebAssembly u32 limit")]
    LimitExceeded { what: &'static str },
}

impl CodegenError {
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::UnsupportedStatement { span, .. }
            | Self::UnsupportedExpression { span, .. }
            | Self::UnsupportedOperator { span, .. }
            | Self::InvalidNumberLiteral { span, .. }
            | Self::NumberLiteralOutOfRange { span, .. }
            | Self::TypeMismatch { span, .. }
            | Self::ExprResultMismatch { span, .. }
            | Self::MissingElse { span, .. }
            | Self::UnknownFunctionReference { span, .. }
            | Self::ArgumentCountMismatch { span, .. }
            | Self::InvalidNormalizedCall { span, .. }
            | Self::UnsupportedBuiltin { span, .. }
            | Self::DuplicateVariableId { span, .. }
            | Self::DuplicateLoopId { span, .. }
            | Self::InvalidLoopTarget { span, .. }
            | Self::BreakValueNotAllowed { span, .. }
            | Self::MissingBreakValue { span, .. }
            | Self::InvalidBreakValue { span, .. }
            | Self::UnknownVariable { span, .. }
            | Self::UninitializedVariable { span, .. }
            | Self::ReadOnlyVariable { span, .. }
            | Self::UnsupportedLocalType { span, .. }
            | Self::InvalidAssignmentTarget { span, .. }
            | Self::UnexpectedReturnValue { span, .. } => Some(*span),

            Self::InvalidFunctionId { .. }
            | Self::InvalidHostFunctionId { .. }
            | Self::DuplicateExportName { .. }
            | Self::UnknownExport { .. }
            | Self::DuplicateExportSelection { .. }
            | Self::UnsupportedAbiType { .. }
            | Self::RangeExport { .. }
            | Self::VoidParameter { .. }
            | Self::MissingReturnValue { .. }
            | Self::LimitExceeded { .. } => None,
        }
    }
}

pub fn compile(program: &TypedProgram) -> Result<Vec<u8>, CodegenError> {
    compile_with_exports(program, &ExportOptions::All)
}

pub fn compile_with_exports(
    program: &TypedProgram,
    exports: &ExportOptions,
) -> Result<Vec<u8>, CodegenError> {
    let validated = validation::ValidatedProgram::new(program, exports)?;

    Ok(assemble(&validated))
}

fn assemble(validated: &validation::ValidatedProgram<'_>) -> Vec<u8> {
    let program = validated.program();
    let layout = validated.layout();
    let mut types = TypeSection::new();

    for signature in &layout.signatures {
        types.ty().function(
            signature.params.iter().copied(),
            signature.result.iter().copied(),
        );
    }

    let mut imports = ImportSection::new();

    for (host, type_index) in program.host_functions.iter().zip(&layout.host_type_indices) {
        imports.import(
            &host.module,
            &host.field,
            EntityType::Function(type_index.raw()),
        );
    }

    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();

    for ((user, plan), body) in program
        .functions
        .iter()
        .zip(&layout.users)
        .zip(validated.bodies())
    {
        functions.function(plan.type_index.raw());

        if plan.exported {
            exports.export(&user.name, ExportKind::Func, plan.function_index.raw());
        }

        code.function(body);
    }

    let mut memories = MemorySection::new();
    let mut globals = GlobalSection::new();
    let mut data = DataSection::new();

    if let Some(memory) = &layout.memory {
        for (helper, plan) in memory::Helper::ALL.into_iter().zip(&memory.helpers) {
            functions.function(plan.type_index.raw());
            code.function(&memory::emit(helper, memory.first_function.raw()));
        }

        memories.memory(MemoryType {
            minimum: u64::from(memory.plan.heap_start)
                .div_ceil(memory::PAGE_BYTES)
                .max(1),
            maximum: Some(memory::MAX_PAGES),
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
            &ConstExpr::i32_const(memory.plan.heap_start as i32),
        );
        exports.export("memory", ExportKind::Memory, 0);
        exports.export(
            "__alloc",
            ExportKind::Func,
            memory
                .first_function
                .offset(memory::Helper::Alloc.index())
                .raw(),
        );
        data.active(
            0,
            &ConstExpr::i32_const(0),
            memory.plan.data.iter().copied(),
        );
    }

    if let Some(remainder) = &layout.remainder {
        functions.function(remainder.type_index.raw());
        code.function(&numeric::emit_remainder());
    }

    if !layout.ranges.is_empty() {
        let allocator = layout
            .memory
            .as_ref()
            .expect("range memory planned before assembly")
            .first_function
            .offset(memory::Helper::Alloc.index());
        let validator_index = usize::try_from(range::Helper::Validate.index())
            .expect("range helper index fits usize");
        let validator = layout.ranges[validator_index].function_index;
        for (helper, plan) in range::Helper::ALL.into_iter().zip(&layout.ranges) {
            functions.function(plan.type_index.raw());
            code.function(&range::emit(helper, allocator, validator));
        }
    }

    let mut module = Module::new();

    module.section(&types);
    module.section(&imports);
    module.section(&functions);

    if layout.memory.is_some() {
        module.section(&memories);
        module.section(&globals);
    }

    module.section(&exports);
    module.section(&code);

    if layout.memory.is_some() {
        module.section(&data);
    }

    module.finish()
}

fn expect_type(
    function: &str,
    expected: Type,
    found: Type,
    span: Span,
) -> Result<(), CodegenError> {
    if expected != found {
        return Err(CodegenError::TypeMismatch {
            function: function.into(),
            expected,
            found,
            span,
        });
    }

    Ok(())
}

// Operand contexts admit Never, but annotations themselves must match exactly.
fn expect_operand(
    function: &str,
    expected: Type,
    found: ExprResult,
    span: Span,
) -> Result<(), CodegenError> {
    if found == ExprResult::Never {
        return Ok(());
    }

    expect_result(function, expected.into(), found, span)
}

fn expect_result(
    function: &str,
    expected: ExprResult,
    found: ExprResult,
    span: Span,
) -> Result<(), CodegenError> {
    if expected == found && found != ExprResult::Value(Type::Void) {
        return Ok(());
    }

    let concrete = |result| match result {
        ExprResult::Value(ty) if ty != Type::Void => Some(ty),
        ExprResult::Void => Some(Type::Void),
        _ => None,
    };

    if let (Some(expected), Some(found)) = (concrete(expected), concrete(found)) {
        return expect_type(function, expected, found, span);
    }

    Err(CodegenError::ExprResultMismatch {
        function: function.into(),
        expected,
        found,
        span,
    })
}

fn checked_u32(value: usize, what: &'static str) -> Result<u32, CodegenError> {
    u32::try_from(value).map_err(|_| CodegenError::LimitExceeded { what })
}
