use std::collections::HashSet;

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedFunction, VariableId},
};
use wasm_encoder::{Function, Instruction};

use super::{CodegenError, expect_type, locals::LocalLayout, statement};

pub(super) struct Context<'a> {
    pub name: &'a str,
    pub return_type: Type,
    locals: LocalLayout,
    initialized: HashSet<VariableId>,
}

impl<'a> Context<'a> {
    fn new(function: &'a TypedFunction) -> Result<Self, CodegenError> {
        Ok(Self {
            name: &function.name,
            return_type: function.return_type,
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
}

pub(super) fn emit(function: &TypedFunction) -> Result<Function, CodegenError> {
    let mut context = Context::new(function)?;
    let mut body = Function::new(context.locals.types.iter().map(|ty| (1, *ty)));
    let mut always_returns = false;

    for statement in &function.body {
        let returns = statement::emit(statement, &mut body, &mut context)?;

        always_returns |= returns;
    }

    if function.return_type != Type::Void && !always_returns {
        return Err(CodegenError::MissingReturnValue {
            function: function.name.clone(),
            return_type: function.return_type,
        });
    }

    body.instruction(&Instruction::End);

    Ok(body)
}
