use std::{cell::Cell, collections::HashSet};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{FunctionRef, TypedBlock, TypedExpr, TypedFunction, VariableId},
};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::{
    CodegenError, checked_u32, expect_type, expression,
    indices::{FunctionIndex, LabelDepth, LocalIndex},
    layout::{CallTarget, Layout},
    locals::LocalLayout,
    memory::Helper,
    statement, temporaries,
};

struct Context<'a> {
    name: &'a str,
    return_type: Type,
    module: &'a Layout,
    locals: LocalLayout,
    initialized: HashSet<VariableId>,
    next_temporary: Cell<u32>,
    first_temporary: u32,
    temporary_types: Vec<ValType>,
}

impl<'a> Context<'a> {
    fn new(function: &'a TypedFunction, module: &'a Layout) -> Result<Self, CodegenError> {
        let locals = LocalLayout::new(function)?;
        let first_temporary = checked_u32(function.params.len(), "function parameter count")?
            .checked_add(checked_u32(locals.types.len(), "function local count")?)
            .ok_or(CodegenError::LimitExceeded {
                what: "function local count",
            })?;
        let temporary_types = temporaries::plan(&function.body)?;
        first_temporary
            .checked_add(checked_u32(temporary_types.len(), "temporary local count")?)
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
            first_temporary,
            temporary_types,
        })
    }

    fn helper(&self, helper: Helper) -> FunctionIndex {
        let memory = self
            .module
            .memory
            .as_ref()
            .expect("memory usage planned before emission");

        memory.first_function.offset(helper.index())
    }

    fn remainder(&self) -> FunctionIndex {
        self.module
            .remainder
            .as_ref()
            .expect("remainder usage planned before emission")
            .function_index
    }

    fn string(&self, value: &str) -> u32 {
        self.module
            .memory
            .as_ref()
            .and_then(|memory| memory.plan.string(value))
            .expect("string literals interned before emission")
    }

    fn temporary(&self, ty: ValType) -> LocalIndex {
        let index = self.next_temporary.get();
        let offset = usize::try_from(index - self.first_temporary)
            .expect("planned temporary index fits usize");

        assert_eq!(
            self.temporary_types.get(offset),
            Some(&ty),
            "typed temporaries planned in emission order"
        );

        self.next_temporary.set(index + 1);

        LocalIndex::new(index)
    }

    fn resolve(&self, id: VariableId, ty: Type, span: Span) -> Result<LocalIndex, CodegenError> {
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

    fn declaration(
        &self,
        id: VariableId,
        ty: Type,
        span: Span,
    ) -> Result<LocalIndex, CodegenError> {
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
}

pub(super) struct BodyEmitter<'a> {
    body: Function,
    context: Context<'a>,
}

impl<'a> BodyEmitter<'a> {
    fn new(function: &'a TypedFunction, module: &'a Layout) -> Result<Self, CodegenError> {
        let context = Context::new(function, module)?;
        let mut locals: Vec<_> = context.locals.types.iter().map(|ty| (1, *ty)).collect();

        let mut temporary_groups: Vec<(u32, ValType)> = Vec::new();

        for ty in &context.temporary_types {
            if let Some((count, previous)) = temporary_groups.last_mut()
                && previous == ty
            {
                // The total temporary count has already been checked against u32.
                *count += 1;
            } else {
                temporary_groups.push((1, *ty));
            }
        }

        // Keep declaration groups separate from scratch groups for legacy bytes.
        locals.extend(temporary_groups);

        Ok(Self {
            body: Function::new(locals),
            context,
        })
    }

    pub fn name(&self) -> &'a str {
        self.context.name
    }

    pub fn return_type(&self) -> Type {
        self.context.return_type
    }

    pub fn instruction(&mut self, instruction: &Instruction<'_>) {
        self.body.instruction(instruction);
    }

    pub fn local_get(&mut self, index: LocalIndex) {
        self.instruction(&Instruction::LocalGet(index.raw()));
    }

    pub fn local_set(&mut self, index: LocalIndex) {
        self.instruction(&Instruction::LocalSet(index.raw()));
    }

    pub fn call(&mut self, index: FunctionIndex) {
        self.instruction(&Instruction::Call(index.raw()));
    }

    pub fn helper(&self, helper: Helper) -> FunctionIndex {
        self.context.helper(helper)
    }

    pub fn remainder(&self) -> FunctionIndex {
        self.context.remainder()
    }

    pub fn string(&self, value: &str) -> u32 {
        self.context.string(value)
    }

    pub fn temporary(&self, ty: ValType) -> LocalIndex {
        self.context.temporary(ty)
    }

    pub fn target(
        &self,
        reference: FunctionRef,
        span: Span,
    ) -> Result<&'a CallTarget, CodegenError> {
        self.context.module.resolve(reference).ok_or_else(|| {
            CodegenError::UnknownFunctionReference {
                function: self.name().into(),
                reference,
                span,
            }
        })
    }

    pub fn resolve(
        &self,
        id: VariableId,
        ty: Type,
        span: Span,
    ) -> Result<LocalIndex, CodegenError> {
        self.context.resolve(id, ty, span)
    }

    pub fn declaration(
        &self,
        id: VariableId,
        ty: Type,
        span: Span,
    ) -> Result<LocalIndex, CodegenError> {
        self.context.declaration(id, ty, span)
    }

    pub fn initialize(&mut self, id: VariableId) {
        self.context.initialized.insert(id);
    }

    pub fn scoped<T>(
        &mut self,
        emit: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let initialized = self.context.initialized.clone();
        let result = emit(self);

        self.context.initialized = initialized;

        result
    }

    pub fn normalize_bool(&mut self) {
        self.instruction(&Instruction::I32Eqz);
        self.instruction(&Instruction::I32Eqz);
    }

    pub fn begin_if(&mut self, block_type: BlockType) {
        self.instruction(&Instruction::If(block_type));
    }

    pub fn otherwise(&mut self) {
        self.instruction(&Instruction::Else);
    }

    pub fn begin_block(&mut self) {
        self.instruction(&Instruction::Block(BlockType::Empty));
    }

    pub fn begin_loop(&mut self) {
        self.instruction(&Instruction::Loop(BlockType::Empty));
    }

    pub fn end(&mut self) {
        self.instruction(&Instruction::End);
    }

    pub fn branch(&mut self, depth: LabelDepth) {
        self.instruction(&Instruction::Br(depth.raw()));
    }

    pub fn branch_if(&mut self, depth: LabelDepth) {
        self.instruction(&Instruction::BrIf(depth.raw()));
    }

    pub fn emit_expr(&mut self, expr: &TypedExpr, span: Span) -> Result<(), CodegenError> {
        expression::emit(expr, self, span)
    }

    pub fn emit_block(&mut self, block: &TypedBlock) -> Result<bool, CodegenError> {
        statement::emit_block(block, self)
    }
}

pub(super) fn emit(function: &TypedFunction, module: &Layout) -> Result<Function, CodegenError> {
    let mut emitter = BodyEmitter::new(function, module)?;

    for param in &function.params {
        let helper = match param.ty {
            Type::String => Some(Helper::ValidateString),
            Type::NumberArray => Some(Helper::ValidateArray),
            _ => None,
        };

        if let Some(helper) = helper {
            let index = emitter.resolve(param.id, param.ty, param.span)?;
            emitter.local_get(index);
            emitter.call(emitter.helper(helper));
            emitter.instruction(&Instruction::Drop);
        }
    }

    let always_returns = emitter.emit_block(&function.body)?;

    if function.return_type != Type::Void && !always_returns {
        return Err(CodegenError::MissingReturnValue {
            function: function.name.clone(),
            return_type: function.return_type,
        });
    }

    emitter.end();

    Ok(emitter.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compiler::Compiler,
        host::HostFunctionId,
        typed_ast::{FunctionId, TypedStatementKind},
    };

    #[test]
    fn scoped_bindings_restore_on_success_and_error_without_rewinding_temporaries() {
        let program = Compiler::new()
            .analyze(
                "fn test(param: number) -> void { if true { let inside: number[] = [1]; } let after: number[] = [2]; }",
            )
            .unwrap();
        let layout = Layout::new(&program).unwrap();
        let function = &program.functions[0];
        let TypedStatementKind::If { then, .. } = &function.body[0].kind else {
            panic!("expected conditional");
        };
        let TypedStatementKind::Let { id: inside, .. } = then[0].kind else {
            panic!("expected scoped local");
        };
        let TypedStatementKind::Let { id: after, .. } = function.body[1].kind else {
            panic!("expected outer local");
        };
        let span = Span::new(40, 50);

        for fail in [false, true] {
            let mut emitter = BodyEmitter::new(function, &layout).unwrap();
            let result = emitter.scoped(|emitter| {
                emitter.initialize(inside);
                assert_eq!(emitter.resolve(inside, Type::NumberArray, span)?.raw(), 1);
                assert_eq!(emitter.temporary(ValType::I32).raw(), 3);

                if fail {
                    Err(CodegenError::InvalidAssignmentTarget {
                        function: "test".into(),
                        span,
                    })
                } else {
                    Ok(())
                }
            });

            if fail {
                assert_eq!(
                    result,
                    Err(CodegenError::InvalidAssignmentTarget {
                        function: "test".into(),
                        span,
                    })
                );
            } else {
                assert_eq!(result, Ok(()));
            }

            assert_eq!(
                emitter.resolve(inside, Type::NumberArray, span),
                Err(CodegenError::UninitializedVariable {
                    function: "test".into(),
                    id: inside,
                    span,
                })
            );
            assert_eq!(
                emitter
                    .resolve(function.params[0].id, Type::Number, span)
                    .unwrap()
                    .raw(),
                0
            );
            emitter.initialize(after);
            assert_eq!(
                emitter
                    .resolve(after, Type::NumberArray, span)
                    .unwrap()
                    .raw(),
                2
            );
            assert_eq!(emitter.temporary(ValType::I32).raw(), 4);
        }
    }

    #[test]
    fn borrowed_call_targets_preserve_function_indices_with_shared_type_indices() {
        let mut compiler = Compiler::new();
        compiler
            .register_host_function("tick", "env", "tick", vec![], Type::Number)
            .unwrap();
        let program = compiler
            .analyze(
                "fn first() -> number { return tick(); } fn second() -> number { return first(); }",
            )
            .unwrap();
        let layout = Layout::new(&program).unwrap();
        let mut emitter = BodyEmitter::new(&program.functions[1], &layout).unwrap();
        let span = Span::new(0, 10);
        let target = emitter
            .target(FunctionRef::User(FunctionId(0)), span)
            .unwrap();

        assert!(std::ptr::eq(
            target,
            layout.resolve(FunctionRef::User(FunctionId(0))).unwrap()
        ));
        assert_eq!(layout.host_type_indices[0].raw(), 0);
        assert_eq!(layout.users[0].type_index.raw(), 0);
        assert_eq!(layout.users[1].type_index.raw(), 0);
        assert_eq!(target.function_index.raw(), 1);
        assert_eq!(layout.users[1].function_index.raw(), 2);

        emitter.call(target.function_index);
        assert_eq!(target.return_type, Type::Number);
        assert_eq!(
            emitter
                .target(FunctionRef::Host(HostFunctionId(0)), span)
                .unwrap()
                .function_index
                .raw(),
            0
        );
        assert!(matches!(
            emitter.target(FunctionRef::User(FunctionId(2)), span),
            Err(CodegenError::UnknownFunctionReference { span: found, .. }) if found == span
        ));
    }
}
