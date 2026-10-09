use crate::{exports::ExportOptions, typed_ast::TypedProgram};
use wasm_encoder::Function;

use super::{CodegenError, function, layout::Layout};

pub(super) struct ValidatedProgram<'a> {
    program: &'a TypedProgram,
    layout: Layout,
    bodies: Vec<Function>,
}

impl<'a> ValidatedProgram<'a> {
    pub fn new(program: &'a TypedProgram, exports: &ExportOptions) -> Result<Self, CodegenError> {
        let layout = match exports {
            ExportOptions::All => Layout::new(program),
            _ => Layout::with_exports(program, exports),
        }?;
        let mut bodies = Vec::with_capacity(program.functions.len());

        for function in &program.functions {
            bodies.push(function::emit(function, &layout)?);
        }

        Ok(Self {
            program,
            layout,
            bodies,
        })
    }

    pub fn program(&self) -> &'a TypedProgram {
        self.program
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn bodies(&self) -> &[Function] {
        &self.bodies
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
