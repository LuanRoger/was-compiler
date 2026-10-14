use std::{
    collections::{HashMap, hash_map::Entry},
    ops::ControlFlow,
};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{TypedCallBinding, TypedStatement, TypedStatementKind, VariableId},
};
use wasm_encoder::ValType;

use super::{
    CodegenError, abi, checked_u32, closures::BodyOwner, indices::LocalIndex, walker::Visitor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BindingKind {
    Mutable,
    Iteration,
    Call,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Local {
    pub index: LocalIndex,
    pub ty: Type,
    pub kind: BindingKind,
}

#[derive(Default)]
pub(super) struct LocalLayout {
    pub types: Vec<ValType>,
    bindings: HashMap<VariableId, Local>,
}

impl LocalLayout {
    #[cfg(test)]
    pub fn new(function: &crate::typed_ast::TypedFunction) -> Result<Self, CodegenError> {
        Self::for_owner(BodyOwner::Function(function))
    }

    pub fn for_owner(function: BodyOwner<'_>) -> Result<Self, CodegenError> {
        let mut layout = Self::default();

        for (index, param) in function.params().iter().enumerate() {
            if param.ty == Type::Void {
                return Err(CodegenError::VoidParameter {
                    function: function.name().into(),
                    index,
                });
            }

            abi::value_type(function.name(), "parameter", param.ty)?;

            layout.insert(
                param.id,
                param.ty,
                BindingKind::Mutable,
                function.name(),
                param.span,
            )?;
        }

        let mut visitor = LocalVisitor {
            layout: &mut layout,
            function: function.name(),
        };

        if let ControlFlow::Break(error) = function.walk(&mut visitor) {
            return Err(error);
        }

        Ok(layout)
    }

    pub fn get(&self, id: VariableId) -> Option<Local> {
        self.bindings.get(&id).copied()
    }

    pub fn shift_for_environment(&mut self) -> Result<(), CodegenError> {
        for local in self.bindings.values_mut() {
            local.index = LocalIndex::new(local.index.raw().checked_add(1).ok_or(
                CodegenError::LimitExceeded {
                    what: "function local count",
                },
            )?);
        }
        Ok(())
    }

    fn insert(
        &mut self,
        id: VariableId,
        ty: Type,
        kind: BindingKind,
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
                entry.insert(Local { index, ty, kind });

                Ok(())
            }
        }
    }
}

struct LocalVisitor<'a> {
    layout: &'a mut LocalLayout,
    function: &'a str,
}

impl Visitor<'_> for LocalVisitor<'_> {
    type Break = CodegenError;

    fn visit_closure_bodies(&self) -> bool {
        false
    }

    fn enter_call_binding(&mut self, binding: &TypedCallBinding) -> ControlFlow<CodegenError> {
        let value_type = match abi::value_type(self.function, "local", binding.ty) {
            Ok(ty) => ty,
            Err(_) => {
                return ControlFlow::Break(CodegenError::InvalidNormalizedCall {
                    reason: "binding requires a concrete storage type",
                    span: binding.span,
                });
            }
        };
        if let Err(error) = self.layout.insert(
            binding.id,
            binding.ty,
            BindingKind::Call,
            self.function,
            binding.span,
        ) {
            return ControlFlow::Break(error);
        }
        self.layout.types.push(value_type);
        ControlFlow::Continue(())
    }

    fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<CodegenError> {
        let (id, ty, kind) = match &statement.kind {
            TypedStatementKind::Let { id, ty, .. } => (*id, *ty, BindingKind::Mutable),
            TypedStatementKind::For { variable, .. } => {
                (*variable, Type::Number, BindingKind::Iteration)
            }
            _ => return ControlFlow::Continue(()),
        };

        {
            let value_type = match abi::value_type(self.function, "local", ty) {
                Ok(value_type) => value_type,
                Err(_) => {
                    return ControlFlow::Break(CodegenError::UnsupportedLocalType {
                        function: self.function.into(),
                        ty,
                        span: statement.span,
                    });
                }
            };

            if let Err(error) = self
                .layout
                .insert(id, ty, kind, self.function, statement.span)
            {
                return ControlFlow::Break(error);
            }

            self.layout.types.push(value_type);
        }

        ControlFlow::Continue(())
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
    fn local_collection_traverses_without_validating_initializers_and_conditions() {
        let mut program = Compiler::new()
            .analyze("fn test() -> void { let a: number = 1; if true { let b: bool = false; } }")
            .unwrap();
        let function = &mut program.functions[0];
        let invalid = TypedExpr {
            kind: TypedExprKind::Number("invalid".into()),
            ty: Type::Void.into(),
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
    fn expression_declarations_are_planned_even_when_their_contents_and_results_are_forged() {
        use crate::typed_ast::ExprResult;

        let mut program = Compiler::new().analyze(
            "fn test() -> void { let outer = { let inner = 1; inner }; if { let condition = true; condition } {} }",
        ).unwrap();
        let function = &mut program.functions[0];
        let TypedStatementKind::Let { value, .. } = &mut function.body[0].kind else {
            panic!("let")
        };
        value.ty = ExprResult::Value(Type::Void);
        let TypedExprKind::Block(block) = &mut value.kind else {
            panic!("block")
        };
        block.tail.as_mut().unwrap().kind = TypedExprKind::Number("NaN".into());
        let TypedStatementKind::If { cond, .. } = &mut function.body[1].kind else {
            panic!("if")
        };
        cond.ty = ExprResult::Never;

        let layout = LocalLayout::new(function).unwrap();
        assert_eq!(layout.types, [ValType::F64, ValType::F64, ValType::I32]);
        // Semantic IDs follow initializer analysis, slots retain declaration preorder.
        assert_eq!(layout.get(VariableId(1)).unwrap().index.raw(), 0);
        assert_eq!(layout.get(VariableId(0)).unwrap().index.raw(), 1);
        assert_eq!(layout.get(VariableId(2)).unwrap().index.raw(), 2);
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
