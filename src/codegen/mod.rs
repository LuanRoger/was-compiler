use crate::{
    ast::Type,
    exports::ExportOptions,
    tokens::Span,
    typed_ast::{BuiltinFunction, FunctionRef, TypedProgram, VariableId},
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
mod layout;
mod locals;
mod memory;
mod numeric;
mod statement;
mod temporaries;
mod values;

#[cfg(test)]
mod export_tests;

#[cfg(test)]
mod memory_tests;

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

pub fn compile(program: &TypedProgram) -> Result<Vec<u8>, CodegenError> {
    compile_with_exports(program, &ExportOptions::All)
}

pub fn compile_with_exports(
    program: &TypedProgram,
    exports: &ExportOptions,
) -> Result<Vec<u8>, CodegenError> {
    let layout = match exports {
        ExportOptions::All => layout::Layout::new(program),
        _ => layout::Layout::with_exports(program, exports),
    }?;

    let mut types = TypeSection::new();

    for signature in &layout.signatures {
        types.ty().function(
            signature.params.iter().copied(),
            signature.result.iter().copied(),
        );
    }

    let mut imports = ImportSection::new();

    for (host, type_index) in program.host_functions.iter().zip(&layout.host_type_indices) {
        imports.import(&host.module, &host.field, EntityType::Function(*type_index));
    }

    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();

    for (user, plan) in program.functions.iter().zip(&layout.users) {
        let body = function::emit(user, &layout)?;

        functions.function(plan.type_index);

        if plan.exported {
            exports.export(&user.name, ExportKind::Func, plan.function_index);
        }

        code.function(&body);
    }

    let mut memories = MemorySection::new();
    let mut globals = GlobalSection::new();
    let mut data = DataSection::new();

    if let Some(memory) = &layout.memory {
        for ((helper, _), plan) in memory::helpers().into_iter().zip(&memory.helpers) {
            functions.function(plan.type_index);
            code.function(&memory::emit(helper, memory.first_function));
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
        exports.export("__alloc", ExportKind::Func, memory.first_function);
        data.active(
            0,
            &ConstExpr::i32_const(0),
            memory.plan.data.iter().copied(),
        );
    }

    if let Some(remainder) = &layout.remainder {
        functions.function(remainder.type_index);
        code.function(&numeric::emit_remainder());
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

    Ok(module.finish())
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

fn checked_u32(value: usize, what: &'static str) -> Result<u32, CodegenError> {
    u32::try_from(value).map_err(|_| CodegenError::LimitExceeded { what })
}
