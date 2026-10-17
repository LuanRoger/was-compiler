use crate::{exports::ExportOptions, typed_ast::TypedProgram};
use wasm_encoder::Function;

use super::{CodegenError, closures, function, layout::Layout};

pub(super) struct ValidatedProgram<'a> {
    program: &'a TypedProgram,
    layout: Layout<'a>,
    bodies: Vec<Function>,
    callable_bodies: Vec<Function>,
}

impl<'a> ValidatedProgram<'a> {
    #[cfg(test)]
    pub fn new(program: &'a TypedProgram, exports: &ExportOptions) -> Result<Self, CodegenError> {
        Self::with_policy(program, exports, super::NumericPolicy::Baseline)
    }

    pub fn with_policy(
        program: &'a TypedProgram,
        exports: &ExportOptions,
        policy: super::NumericPolicy,
    ) -> Result<Self, CodegenError> {
        let layout = match exports {
            ExportOptions::All => Layout::new(program),
            _ => Layout::with_exports(program, exports),
        }?;
        let mut bodies = Vec::with_capacity(program.functions.len());

        for function in &program.functions {
            bodies.push(function::emit(function, &layout, policy)?);
        }

        let mut callable_bodies = Vec::with_capacity(layout.callables.len());
        for plan in &layout.callables {
            callable_bodies.push(closures::emit_closure(plan.declaration, &layout, policy)?);
        }

        Ok(Self {
            program,
            layout,
            bodies,
            callable_bodies,
        })
    }

    pub fn program(&self) -> &'a TypedProgram {
        self.program
    }

    pub fn layout(&self) -> &Layout<'a> {
        &self.layout
    }

    pub fn callable_bodies(&self) -> &[Function] {
        &self.callable_bodies
    }

    pub fn bodies(&self) -> &[Function] {
        &self.bodies
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
