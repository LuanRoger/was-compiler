pub mod ast;

pub mod codegen;
pub mod compiler;
pub mod diagnostic;
pub mod exports;
mod flow;
pub mod host;
pub mod lexer;
mod match_patterns;
mod number_literal;
mod optimization;
pub mod parser;
pub mod semantic;
pub mod tokens;
pub mod typed_ast;

#[cfg(test)]
#[path = "../tests/support/process.rs"]
mod test_process;

pub use ast::Type;
pub use codegen::CodegenError;
pub use compiler::{CompileError, Compiler, CompilerOptions};
pub use exports::ExportOptions;
pub use host::{HostFunction, HostFunctionId, HostFunctions, HostRegistrationError};
pub use optimization::OptimizationLevel;
pub use typed_ast::TypedProgram;
