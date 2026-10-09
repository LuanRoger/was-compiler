use std::{
    collections::{HashMap, hash_map::Entry},
    ops::ControlFlow,
};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedFunction, TypedStatement, TypedStatementKind, VariableId},
};
use wasm_encoder::ValType;

use super::{
    CodegenError, abi, checked_u32,
    indices::LocalIndex,
    walker::{self, Visitor},
};

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

        let mut visitor = LocalVisitor {
            layout: &mut layout,
            function: &function.name,
        };

        if let ControlFlow::Break(error) = walker::walk_block(&function.body, &mut visitor) {
            return Err(error);
        }

        Ok(layout)
    }

    pub fn get(&self, id: VariableId) -> Option<Local> {
        self.bindings.get(&id).copied()
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

struct LocalVisitor<'a> {
    layout: &'a mut LocalLayout,
    function: &'a str,
}

impl Visitor for LocalVisitor<'_> {
    type Break = CodegenError;

    fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<CodegenError> {
        if let TypedStatementKind::Let { id, ty, .. } = &statement.kind {
            let value_type = match abi::value_type(self.function, "local", *ty) {
                Ok(value_type) => value_type,
                Err(_) => {
                    return ControlFlow::Break(CodegenError::UnsupportedLocalType {
                        function: self.function.into(),
                        ty: *ty,
                        span: statement.span,
                    });
                }
            };

            if let Err(error) = self.layout.insert(*id, *ty, self.function, statement.span) {
                return ControlFlow::Break(error);
            }

            self.layout.types.push(value_type);
        }

        ControlFlow::Continue(())
    }

    fn visit_expressions(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compiler::Compiler,
        typed_ast::{TypedExpr, TypedExprKind},
    };

    #[test]
    fn slots_preserve_parameter_branch_loop_and_unreachable_declaration_order() {
        let program = Compiler::new()
            .analyze(
                "fn test(value: number, flag: bool) -> void {
                let a: number = 0;
                if flag { let b: string = \"b\"; } else { let c: number[] = []; }
                while flag { let d: bool = false; }
                return;
                let e: number = 1;
            }",
            )
            .unwrap();
        let original = program.clone();
        let layout = LocalLayout::new(&program.functions[0]).unwrap();

        assert_eq!(
            layout.types,
            [
                ValType::F64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::F64
            ]
        );
        for (index, ty) in [
            Type::Number,
            Type::Bool,
            Type::Number,
            Type::String,
            Type::NumberArray,
            Type::Bool,
            Type::Number,
        ]
        .into_iter()
        .enumerate()
        {
            let local = layout.get(VariableId(index)).unwrap();
            assert_eq!(local.index.raw(), index as u32);
            assert_eq!(local.ty, ty);
        }
        assert_eq!(program, original);
    }

    #[test]
    fn local_collection_does_not_validate_or_descend_into_initializers_and_conditions() {
        let mut program = Compiler::new()
            .analyze("fn test() -> void { let a: number = 1; if true { let b: bool = false; } }")
            .unwrap();
        let function = &mut program.functions[0];
        let invalid = TypedExpr {
            kind: TypedExprKind::Number("invalid".into()),
            ty: Type::Void,
        };
        let TypedStatementKind::Let { value, .. } = &mut function.body[0].kind else {
            panic!("let");
        };
        *value = invalid.clone();
        let TypedStatementKind::If { cond, .. } = &mut function.body[1].kind else {
            panic!("if");
        };
        **cond = invalid;

        let layout = LocalLayout::new(function).unwrap();

        assert_eq!(layout.types, [ValType::F64, ValType::I32]);
        assert_eq!(layout.get(VariableId(0)).unwrap().index.raw(), 0);
        assert_eq!(layout.get(VariableId(1)).unwrap().index.raw(), 1);
    }

    #[test]
    fn unsupported_local_type_precedes_duplicate_id_and_later_declaration_errors() {
        let mut program = Compiler::new().analyze(
            "fn test(value: number) -> void { if true { let a: number = 1; } else { let b: number = 2; } }",
        ).unwrap();
        let function = &mut program.functions[0];
        let TypedStatementKind::If { then, else_, .. } = &mut function.body[0].kind else {
            panic!("if");
        };
        let span = then[0].span;
        let TypedStatementKind::Let { id, ty, .. } = &mut then[0].kind else {
            panic!("let");
        };
        *id = VariableId(0);
        *ty = Type::Void;
        let TypedStatementKind::Let { id, .. } = &mut else_.as_mut().unwrap()[0].kind else {
            panic!("let");
        };
        *id = VariableId(0);

        assert!(
            matches!(LocalLayout::new(function), Err(CodegenError::UnsupportedLocalType {
            function, ty: Type::Void, span: actual,
        }) if function == "test" && actual == span)
        );
    }
}
