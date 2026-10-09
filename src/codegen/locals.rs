use std::collections::{HashMap, hash_map::Entry};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedBlock, TypedFunction, TypedStatementKind, VariableId},
};
use wasm_encoder::ValType;

use super::{CodegenError, abi, checked_u32, indices::LocalIndex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Local {
    pub index: LocalIndex,
    pub ty: Type,
}

#[derive(Default)]
pub(super) struct LocalLayout {
    pub types: Vec<ValType>,
    bindings: HashMap<VariableId, Local>,
}

impl LocalLayout {
    pub fn new(function: &TypedFunction) -> Result<Self, CodegenError> {
        let mut layout = Self::default();

        for (index, param) in function.params.iter().enumerate() {
            if param.ty == Type::Void {
                return Err(CodegenError::VoidParameter {
                    function: function.name.clone(),
                    index,
                });
            }

            abi::value_type(&function.name, "parameter", param.ty)?;

            layout.insert(param.id, param.ty, &function.name, param.span)?;
        }

        layout.collect(&function.body, &function.name)?;

        Ok(layout)
    }

    pub fn get(&self, id: VariableId) -> Option<Local> {
        self.bindings.get(&id).copied()
    }

    fn collect(&mut self, block: &TypedBlock, function: &str) -> Result<(), CodegenError> {
        for statement in block {
            match &statement.kind {
                TypedStatementKind::Let { id, ty, .. } => {
                    let value_type = abi::value_type(function, "local", *ty).map_err(|_| {
                        CodegenError::UnsupportedLocalType {
                            function: function.into(),
                            ty: *ty,
                            span: statement.span,
                        }
                    })?;

                    self.insert(*id, *ty, function, statement.span)?;

                    self.types.push(value_type);
                }

                TypedStatementKind::If { then, else_, .. } => {
                    self.collect(then, function)?;

                    if let Some(block) = else_ {
                        self.collect(block, function)?;
                    }
                }

                TypedStatementKind::While { body, .. } => {
                    self.collect(body, function)?;
                }

                TypedStatementKind::Assignment { .. }
                | TypedStatementKind::Return(_)
                | TypedStatementKind::Expression(_) => {}
            }
        }

        Ok(())
    }

    fn insert(
        &mut self,
        id: VariableId,
        ty: Type,
        function: &str,
        span: Span,
    ) -> Result<(), CodegenError> {
        let index = LocalIndex::new(checked_u32(self.bindings.len(), "function local index")?);

        match self.bindings.entry(id) {
            Entry::Occupied(_) => Err(CodegenError::DuplicateVariableId {
                function: function.into(),
                id,
                span,
            }),

            Entry::Vacant(entry) => {
                entry.insert(Local { index, ty });

                Ok(())
            }
        }
    }
}
