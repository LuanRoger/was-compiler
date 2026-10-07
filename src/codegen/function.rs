use std::collections::HashSet;

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedFunction, VariableId},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, layout::Layout, locals::LocalLayout, statement};

pub(super) struct Context<'a> {
    pub name: &'a str,
    pub return_type: Type,
    pub module: &'a Layout,
    locals: LocalLayout,
    initialized: HashSet<VariableId>,
}

impl<'a> Context<'a> {
    fn new(function: &'a TypedFunction, module: &'a Layout) -> Result<Self, CodegenError> {
        Ok(Self {
            name: &function.name,
            return_type: function.return_type,
            module,
            locals: LocalLayout::new(function)?,
            initialized: function.params.iter().map(|param| param.id).collect(),
        })
    }

    pub fn resolve(&self, id: VariableId, ty: Type, span: Span) -> Result<u32, CodegenError> {
        let index = self.declaration(id, ty, span)?;

        if !self.initialized.contains(&id) {
            return Err(CodegenError::UninitializedVariable {
                function: self.name.into(),
                id,
                span,
            });
        }

        Ok(index)
    }

    pub fn declaration(&self, id: VariableId, ty: Type, span: Span) -> Result<u32, CodegenError> {
        let local = self
            .locals
            .get(id)
            .ok_or_else(|| CodegenError::UnknownVariable {
                function: self.name.into(),
                id,
                span,
            })?;

        expect_type(self.name, local.ty, ty, span)?;

        Ok(local.index)
    }

    pub fn initialize(&mut self, id: VariableId) {
        self.initialized.insert(id);
    }

    pub fn scoped<T>(
        &mut self,
        emit: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let initialized = self.initialized.clone();
        let result = emit(self);

        self.initialized = initialized;

        result
    }
}

pub(super) fn emit(function: &TypedFunction, module: &Layout) -> Result<Function, CodegenError> {
    let mut context = Context::new(function, module)?;
    let mut body = Function::new(context.locals.types.iter().map(|ty| (1, *ty)));
    let always_returns = statement::emit_block(&function.body, &mut body, &mut context)?;

    if function.return_type != Type::Void && !always_returns {
        return Err(CodegenError::MissingReturnValue {
            function: function.name.clone(),
            return_type: function.return_type,
        });
    }

    body.instruction(&Instruction::End);

    Ok(body)
}
