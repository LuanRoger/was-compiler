use crate::{ast::Type, tokens::Span, typed_ast::TypedProgram};
use thiserror::Error;
use wasm_encoder::{
    CodeSection, EntityType, ExportKind, ExportSection, FunctionSection, ImportSection, Module,
    TypeSection,
};

mod abi;
mod expression;
mod function;
mod layout;
mod statement;

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

    #[error("void function `{function}` cannot return a value at {span:?}")]
    UnexpectedReturnValue { function: String, span: Span },

    #[error("function `{function}` cannot produce its declared {return_type:?} result")]
    MissingReturnValue { function: String, return_type: Type },

    #[error("{what} exceeds the WebAssembly u32 limit")]
    LimitExceeded { what: &'static str },
}

pub fn compile(program: &TypedProgram) -> Result<Vec<u8>, CodegenError> {
    let layout = layout::Layout::new(program)?;

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
        let body = function::emit(user)?;

        functions.function(plan.type_index);
        exports.export(&user.name, ExportKind::Func, plan.function_index);
        code.function(&body);
    }

    let mut module = Module::new();

    module.section(&types);
    module.section(&imports);
    module.section(&functions);
    module.section(&exports);
    module.section(&code);

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
