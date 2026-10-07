use std::{cell::Cell, collections::HashSet};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedFunction, VariableId},
};
use wasm_encoder::{Function, Instruction, ValType};

use super::{
    CodegenError, checked_u32, expect_type, layout::Layout, locals::LocalLayout, memory::Helper,
    statement, temporaries,
};

pub(super) struct Context<'a> {
    pub name: &'a str,
    pub return_type: Type,
    pub module: &'a Layout,
    locals: LocalLayout,
    initialized: HashSet<VariableId>,
    next_temporary: Cell<u32>,
    temporary_count: u32,
    temporary_end: u32,
}

impl<'a> Context<'a> {
    fn new(function: &'a TypedFunction, module: &'a Layout) -> Result<Self, CodegenError> {
        let locals = LocalLayout::new(function)?;
        let first_temporary = checked_u32(function.params.len(), "function parameter count")?
            .checked_add(checked_u32(locals.types.len(), "function local count")?)
            .ok_or(CodegenError::LimitExceeded {
                what: "function local count",
            })?;
        let temporary_count = temporaries::count(&function.body)?;
        let temporary_end =
            first_temporary
                .checked_add(temporary_count)
                .ok_or(CodegenError::LimitExceeded {
                    what: "function local count",
                })?;

        Ok(Self {
            name: &function.name,
            return_type: function.return_type,
            module,
            locals,
            initialized: function.params.iter().map(|param| param.id).collect(),
            next_temporary: Cell::new(first_temporary),
            temporary_count,
            temporary_end,
        })
    }

    pub fn helper(&self, helper: Helper) -> u32 {
        let memory = self
            .module
            .memory
            .as_ref()
            .expect("memory usage planned before emission");

        memory.first_function + helper.index()
    }

    pub fn string(&self, value: &str) -> u32 {
        self.module
            .memory
            .as_ref()
            .and_then(|memory| memory.plan.string(value))
            .expect("string literals interned before emission")
    }

    pub fn temporary(&self) -> u32 {
        let index = self.next_temporary.get();

        assert!(
            index < self.temporary_end,
            "array temporaries planned before emission"
        );

        self.next_temporary.set(index + 1);

        index
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
    let mut locals: Vec<_> = context.locals.types.iter().map(|ty| (1, *ty)).collect();

    if context.temporary_count != 0 {
        locals.push((context.temporary_count, ValType::I32));
    }

    let mut body = Function::new(locals);

    for param in &function.params {
        let helper = match param.ty {
            Type::String => Some(Helper::ValidateString),
            Type::NumberArray => Some(Helper::ValidateArray),
            _ => None,
        };

        if let Some(helper) = helper {
            body.instruction(&Instruction::LocalGet(
                context.resolve(param.id, param.ty, param.span)?,
            ));
            body.instruction(&Instruction::Call(context.helper(helper)));
            body.instruction(&Instruction::Drop);
        }
    }

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
