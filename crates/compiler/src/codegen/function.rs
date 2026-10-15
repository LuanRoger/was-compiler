use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet},
};

use crate::{
    ast::Type,
    flow::Flow,
    tokens::Span,
    typed_ast::{
        BindingOwner, CallableId, ExprResult, FunctionRef, LoopId, TypedBlock, TypedExpr,
        TypedFunction, TypedParam, VariableId,
    },
};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::{
    CodegenError, abi, checked_u32,
    closures::{BodyOwner, LexicalPrefix},
    expect_result, expect_type, expression,
    indices::{FunctionIndex, LabelDepth, LocalIndex},
    layout::{CallTarget, Layout},
    locals::{BindingKind, LocalLayout},
    memory::Helper,
    range, statement, temporaries,
};

// The child module owns callable lifecycle operations without exposing emitter state.
#[path = "closures.rs"]
pub(super) mod closures;

struct Context<'a> {
    name: &'a str,
    return_type: Type,
    module: &'a Layout<'a>,
    owner: BindingOwner,
    // Fast membership covers declared dependencies and owner-local declarations.
    available_callables: HashSet<CallableId>,
    lexical_prefix: LexicalPrefix,
    used_dependencies: BTreeSet<usize>,
    value_return_syntax: bool,
    storage: closures::Storage,
    cells_active: bool,
    used_captures: BTreeSet<usize>,
    capture_slots: HashMap<(BindingOwner, VariableId), usize>,
    locals: LocalLayout,
    initialized: HashSet<VariableId>,
    next_temporary: Cell<u32>,
    first_temporary: u32,
    temporary_types: Vec<ValType>,
}

impl<'a> Context<'a> {
    fn new(function: BodyOwner<'a>, module: &'a Layout<'a>) -> Result<Self, CodegenError> {
        let mut locals = LocalLayout::for_owner(function)?;
        let hidden = u32::from(
            matches!(function, BodyOwner::Closure(c) if module.callables[c.id.0].needs_context),
        );
        if hidden != 0 {
            locals.shift_for_environment()?;
        }
        let first_temporary = checked_u32(function.params().len(), "function parameter count")?
            .checked_add(hidden)
            .ok_or(CodegenError::LimitExceeded {
                what: "function parameter count",
            })?
            .checked_add(checked_u32(locals.types.len(), "function local count")?)
            .ok_or(CodegenError::LimitExceeded {
                what: "function local count",
            })?;
        let temporary_types = temporaries::for_owner(function)?;
        let first_storage = first_temporary
            .checked_add(checked_u32(temporary_types.len(), "temporary local count")?)
            .ok_or(CodegenError::LimitExceeded {
                what: "function local count",
            })?;

        let storage = closures::Storage::new(function, &locals, first_storage, &module.callables)?;
        let mut capture_slots = HashMap::new();
        if let BodyOwner::Closure(closure) = function {
            capture_slots.reserve(closure.captures.len());
            for (slot, capture) in closure.captures.iter().enumerate() {
                if capture_slots
                    .insert((capture.owner, capture.variable), slot)
                    .is_some()
                {
                    return Err(closures::invalid("invalid capture metadata", closure.span));
                }
            }
        }

        Ok(Self {
            capture_slots,
            storage,
            cells_active: false,
            used_captures: BTreeSet::new(),
            name: function.name(),
            return_type: function.return_type(),
            module,
            owner: function.owner(),
            available_callables: HashSet::new(),
            lexical_prefix: LexicalPrefix::default(),
            used_dependencies: BTreeSet::new(),
            value_return_syntax: false,
            locals,
            initialized: function.params().iter().map(|param| param.id).collect(),
            next_temporary: Cell::new(first_temporary),
            first_temporary,
            temporary_types,
        })
    }

    fn callable_available(&self, callable: CallableId) -> bool {
        // An omitted dependency still receives normal body checks before the exact
        // metadata check. The prefix distinguishes it from a lexically invalid call.
        self.available_callables.contains(&callable)
            || self
                .lexical_prefix
                .contains(callable, &self.module.callables)
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
            if self
                .locals
                .get(id)
                .is_some_and(|local| local.kind == BindingKind::Call)
            {
                return Err(CodegenError::InvalidNormalizedCall {
                    reason: "synthetic binding is only readable by its final call",
                    span,
                });
            }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LabelId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoopKind {
    Statement,
    Value(ExprResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LoopContext {
    id: LoopId,
    pub kind: LoopKind,
    pub break_label: LabelId,
    pub continue_label: LabelId,
}

pub(super) struct BodyEmitter<'a> {
    body: Function,
    context: Context<'a>,
    labels: Vec<LabelId>,
    next_label: usize,
    loops: Vec<LoopContext>,
    seen_loops: HashSet<LoopId>,
}

impl<'a> BodyEmitter<'a> {
    fn new(function: &'a TypedFunction, module: &'a Layout<'a>) -> Result<Self, CodegenError> {
        Self::for_owner(BodyOwner::Function(function), module)
    }

    fn for_owner(function: BodyOwner<'a>, module: &'a Layout<'a>) -> Result<Self, CodegenError> {
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
        if context.storage.count != 0 {
            locals.push((context.storage.count, ValType::I32));
        }

        Ok(Self {
            body: Function::new(locals),
            context,
            labels: Vec::new(),
            next_label: 0,
            loops: Vec::new(),
            seen_loops: HashSet::new(),
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

    pub fn range_helper(&self, helper: range::Helper) -> FunctionIndex {
        self.context
            .module
            .ranges
            .get(usize::try_from(helper.index()).expect("range helper index fits usize"))
            .expect("range usage planned before emission")
            .function_index
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

    pub fn resolve_expr(
        &self,
        id: VariableId,
        result: ExprResult,
        span: Span,
    ) -> Result<LocalIndex, CodegenError> {
        let local = self
            .context
            .locals
            .get(id)
            .ok_or_else(|| CodegenError::UnknownVariable {
                function: self.name().into(),
                id,
                span,
            })?;

        expect_result(self.name(), local.ty.into(), result, span)?;
        self.resolve(id, local.ty, span)
    }

    pub fn check_writable(&self, id: VariableId, span: Span) -> Result<(), CodegenError> {
        if self
            .context
            .locals
            .get(id)
            .is_some_and(|local| local.kind == BindingKind::Call)
        {
            return Err(CodegenError::InvalidNormalizedCall {
                reason: "synthetic binding is read-only",
                span,
            });
        }
        if self
            .context
            .locals
            .get(id)
            .is_some_and(|local| local.kind == BindingKind::Iteration)
        {
            return Err(CodegenError::ReadOnlyVariable {
                function: self.name().into(),
                id,
                span,
            });
        }

        Ok(())
    }

    pub fn check_capture_writable(
        &self,
        owner: BindingOwner,
        id: VariableId,
        span: Span,
    ) -> Result<(), CodegenError> {
        let invalid = || closures::invalid("invalid capture metadata", span);
        let BindingOwner::Closure(callable) = self.context.owner else {
            return Err(invalid());
        };
        let slot = *self
            .context
            .capture_slots
            .get(&(owner, id))
            .ok_or_else(invalid)?;
        // Creator checking has reconstructed this mutability from the original
        // declaration's provenance before any helper body is lowered.
        let capture = &self.context.module.callables[callable.0]
            .declaration
            .captures[slot];
        if capture.mutability == crate::typed_ast::CaptureMutability::ReadOnly {
            return Err(CodegenError::ReadOnlyVariable {
                function: self.name().into(),
                id,
                span,
            });
        }
        Ok(())
    }

    fn allocate(&mut self, size: u32, destination: LocalIndex) {
        self.instruction(&Instruction::I32Const(i32::from_ne_bytes(
            size.to_ne_bytes(),
        )));
        self.call(self.helper(Helper::Alloc));
        self.local_set(destination);
    }

    pub fn promote(&mut self, id: VariableId, ty: Type, span: Span) -> Result<(), CodegenError> {
        if let Some(cell) = self.context.storage.cells.get(&id).copied() {
            let local = self.declaration(id, ty, span)?;
            self.allocate(abi::CAPTURE_CELL_SIZE, cell);
            self.local_get(cell);
            self.local_get(local);
            self.store_cell(ty);
        }
        Ok(())
    }

    fn promote_parameters(&mut self, params: &[TypedParam]) -> Result<(), CodegenError> {
        for param in params {
            self.promote(param.id, param.ty, param.span)?;
        }
        self.context.cells_active = true;
        Ok(())
    }

    pub fn cell_address(
        &mut self,
        expr: &TypedExpr,
        span: Span,
    ) -> Result<Option<Type>, CodegenError> {
        use crate::typed_ast::TypedExprKind;
        match &expr.kind {
            TypedExprKind::Identifier { id, .. } => {
                self.resolve_expr(*id, expr.ty, span)?;
                if self.context.cells_active
                    && let Some(cell) = self.context.storage.cells.get(id).copied()
                {
                    self.local_get(cell);
                    return Ok(expr.ty.value_type());
                }
                Ok(None)
            }
            TypedExprKind::Captured { owner, id, .. } => {
                let invalid = || closures::invalid("invalid capture metadata", span);
                let BindingOwner::Closure(callable) = self.context.owner else {
                    return Err(invalid());
                };
                let closure = self
                    .context
                    .module
                    .callables
                    .get(callable.0)
                    .ok_or_else(invalid)?
                    .declaration;
                let slot = *self
                    .context
                    .capture_slots
                    .get(&(*owner, *id))
                    .ok_or_else(invalid)?;
                let capture = closure.captures.get(slot).ok_or_else(invalid)?;
                if capture.ty == Type::Void || expr.ty != ExprResult::from(capture.ty) {
                    return Err(invalid());
                }
                self.context.used_captures.insert(slot);
                self.local_get(LocalIndex::new(0));
                self.instruction(&Instruction::I32Load(abi::environment_slot(slot)?));
                Ok(Some(capture.ty))
            }
            _ => Ok(None),
        }
    }

    pub fn load_cell(&mut self, ty: Type) {
        self.instruction(&if ty == Type::Number {
            Instruction::F64Load(abi::cell_arg(ty))
        } else {
            Instruction::I32Load(abi::cell_arg(ty))
        });
    }

    pub fn store_cell(&mut self, ty: Type) {
        self.instruction(&if ty == Type::Number {
            Instruction::F64Store(abi::cell_arg(ty))
        } else {
            Instruction::I32Store(abi::cell_arg(ty))
        });
    }

    pub fn note_value_return(&mut self) {
        self.context.value_return_syntax = true;
    }

    pub fn initialize(&mut self, id: VariableId) {
        self.context.initialized.insert(id);
    }

    pub fn scoped<T>(
        &mut self,
        emit: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let initialized = self.context.initialized.clone();
        let available_callables = self.context.available_callables.clone();
        let lexical_prefix = self.context.lexical_prefix;
        let result = self.structured(emit);

        self.context.initialized = initialized;
        self.context.available_callables = available_callables;
        self.context.lexical_prefix = lexical_prefix;

        result
    }

    pub(super) fn structured<T>(
        &mut self,
        emit: impl FnOnce(&mut Self) -> Result<T, CodegenError>,
    ) -> Result<T, CodegenError> {
        let labels = self.labels.len();
        let loops = self.loops.len();
        let result = emit(self);

        self.labels.truncate(labels);
        self.loops.truncate(loops);
        // Identities remain consumed: labels are unique and loop IDs are function-wide.

        result
    }

    pub fn register_loop(&mut self, id: LoopId, span: Span) -> Result<(), CodegenError> {
        if !self.seen_loops.insert(id) {
            return Err(CodegenError::DuplicateLoopId {
                function: self.name().into(),
                id,
                span,
            });
        }

        Ok(())
    }

    pub fn enter_loop(
        &mut self,
        id: LoopId,
        kind: LoopKind,
        break_label: LabelId,
        continue_label: LabelId,
    ) {
        self.loops.push(LoopContext {
            id,
            kind,
            break_label,
            continue_label,
        });
    }

    pub fn loop_context(&self, target: LoopId, span: Span) -> Result<LoopContext, CodegenError> {
        self.loops
            .last()
            .copied()
            .filter(|context| context.id == target)
            .ok_or_else(|| CodegenError::InvalidLoopTarget {
                function: self.name().into(),
                target,
                span,
            })
    }

    pub fn normalize_bool(&mut self) {
        self.instruction(&Instruction::I32Eqz);
        self.instruction(&Instruction::I32Eqz);
    }

    fn begin_label(&mut self, instruction: &Instruction<'_>) -> Result<LabelId, CodegenError> {
        let label = LabelId(self.next_label);
        self.next_label = self
            .next_label
            .checked_add(1)
            .ok_or(CodegenError::LimitExceeded {
                what: "function label identity count",
            })?;
        checked_u32(self.labels.len(), "function label nesting")?;

        self.labels.push(label);
        self.instruction(instruction);

        Ok(label)
    }

    pub fn begin_if(&mut self, block_type: BlockType) -> Result<LabelId, CodegenError> {
        self.begin_label(&Instruction::If(block_type))
    }

    pub fn otherwise(&mut self) {
        self.instruction(&Instruction::Else);
    }

    pub fn begin_block(&mut self) -> Result<LabelId, CodegenError> {
        self.begin_value_block(BlockType::Empty)
    }

    pub fn begin_value_block(&mut self, block_type: BlockType) -> Result<LabelId, CodegenError> {
        self.begin_label(&Instruction::Block(block_type))
    }

    pub fn begin_loop(&mut self) -> Result<LabelId, CodegenError> {
        self.begin_label(&Instruction::Loop(BlockType::Empty))
    }

    pub fn end(&mut self) {
        self.labels.pop();
        self.instruction(&Instruction::End);
    }

    fn label_depth(&self, target: LabelId) -> Result<LabelDepth, CodegenError> {
        let depth = self
            .labels
            .iter()
            .rev()
            .position(|label| *label == target)
            .expect("branch targets an active emitter label");

        Ok(LabelDepth::new(checked_u32(depth, "branch label depth")?))
    }

    pub fn branch(&mut self, target: LabelId) -> Result<(), CodegenError> {
        let depth = self.label_depth(target)?;
        self.instruction(&Instruction::Br(depth.raw()));

        Ok(())
    }

    pub fn branch_if(&mut self, target: LabelId) -> Result<(), CodegenError> {
        let depth = self.label_depth(target)?;
        self.instruction(&Instruction::BrIf(depth.raw()));

        Ok(())
    }

    pub fn emit_expr(&mut self, expr: &TypedExpr, span: Span) -> Result<Flow, CodegenError> {
        self.structured(|emitter| expression::emit(expr, emitter, span))
    }

    pub fn emit_block(&mut self, block: &TypedBlock) -> Result<Flow, CodegenError> {
        statement::emit_block(block, self)
    }
}

pub(super) fn emit<'a>(
    function: &'a TypedFunction,
    module: &'a Layout<'a>,
) -> Result<Function, CodegenError> {
    let mut emitter = BodyEmitter::new(function, module)?;

    validate_parameters(&function.params, &mut emitter)?;

    let readable: Vec<_> = function.params.iter().map(|param| param.id).collect();
    for contract in &function.contracts {
        super::pure::check(&contract.value, &readable).map_err(|violation| {
            let reason = match violation {
                super::pure::Violation::Read => "contracts may only read formal parameters",
                super::pure::Violation::Membership => {
                    "range membership is not allowed in contracts"
                }
                super::pure::Violation::Expression => {
                    "contract expression is outside the pure-expression whitelist"
                }
            };
            CodegenError::InvalidContractExpression {
                reason,
                span: contract.span,
            }
        })?;
        expect_result(
            emitter.name(),
            Type::Bool.into(),
            contract.value.ty,
            contract.span,
        )?;
        emitter.emit_expr(&contract.value, contract.span)?;
        emitter.instruction(&Instruction::I32Eqz);
        emitter.begin_if(BlockType::Empty)?;
        emitter.instruction(&Instruction::Unreachable);
        emitter.end();
    }

    emitter.promote_parameters(&function.params)?;

    // Contracts are assertions, not trusted flow facts, even when constant false.
    let flow = emitter.emit_block(&function.body)?;
    if function.return_type != Type::Void && !flow.returns_or_diverges() {
        return Err(CodegenError::MissingReturnValue {
            function: function.name.clone(),
            return_type: function.return_type,
        });
    }
    emitter.end();
    Ok(emitter.body)
}

fn validate_parameters(
    params: &[TypedParam],
    emitter: &mut BodyEmitter<'_>,
) -> Result<(), CodegenError> {
    for param in params {
        if param.ty == Type::Range {
            let index = emitter.resolve(param.id, param.ty, param.span)?;
            emitter.local_get(index);
            emitter.call(emitter.range_helper(range::Helper::Validate));
            emitter.instruction(&Instruction::Drop);
            continue;
        }

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

    Ok(())
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
    fn loop_scopes_restore_labels_loops_and_bindings_after_success_and_nested_errors() {
        let program = Compiler::new()
            .analyze("fn test(param: number) -> void { while true { let inside: number = 1; if true { inside; } } }")
            .unwrap();
        let layout = Layout::new(&program).unwrap();
        let function = &program.functions[0];
        let TypedStatementKind::While { id, body, .. } = &function.body[0].kind else {
            panic!("expected loop");
        };
        let TypedStatementKind::Let { id: inside, .. } = body[0].kind else {
            panic!("expected loop local");
        };
        let span = Span::new(40, 60);

        for fail in [false, true] {
            let mut statement = function.body[0].clone();
            if fail {
                let TypedStatementKind::While { body, .. } = &mut statement.kind else {
                    unreachable!();
                };
                let TypedStatementKind::If { then, .. } = &mut body[1].kind else {
                    unreachable!();
                };
                then.push(crate::typed_ast::TypedStatement {
                    kind: TypedStatementKind::Break {
                        target: LoopId(999),
                    },
                    span,
                });
            }

            let mut emitter = BodyEmitter::new(function, &layout).unwrap();
            let exit = emitter.begin_block().unwrap();
            let back = emitter.begin_loop().unwrap();
            emitter.enter_loop(LoopId(5000), LoopKind::Statement, exit, back);
            let labels = emitter.labels.clone();
            let loops = emitter.loops.clone();
            let next_label = emitter.next_label;

            let result = emitter.emit_block(&vec![statement]);
            if fail {
                assert_eq!(
                    result,
                    Err(CodegenError::InvalidLoopTarget {
                        function: "test".into(),
                        target: LoopId(999),
                        span,
                    })
                );
            } else {
                assert_eq!(result, Ok(Flow::FALLTHROUGH));
            }
            assert_eq!(emitter.labels, labels);
            assert_eq!(emitter.loops, loops);
            assert!(emitter.next_label > next_label);
            assert_eq!(emitter.label_depth(exit).unwrap().raw(), 1);
            assert_eq!(emitter.label_depth(back).unwrap().raw(), 0);
            assert_eq!(
                emitter.loop_context(*id, span),
                Err(CodegenError::InvalidLoopTarget {
                    function: "test".into(),
                    target: *id,
                    span,
                })
            );
            assert_eq!(
                emitter.resolve(inside, Type::Number, span),
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
            assert_eq!(
                emitter
                    .loop_context(LoopId(5000), span)
                    .unwrap()
                    .continue_label,
                back
            );
        }
    }

    #[test]
    fn for_scopes_restore_after_iterable_and_body_errors_without_reusing_scratch() {
        use crate::typed_ast::{TypedExprKind, TypedStatement};

        let program = Compiler::new()
            .analyze(
                "fn test(param: number) -> void {
                for i in { let r = 0..2; r } { let inside = i; if true { inside; } }
                let after = [];
            }",
            )
            .unwrap();
        let layout = Layout::new(&program).unwrap();
        let function = &program.functions[0];
        let TypedStatementKind::For { id, variable, .. } = function.body[0].kind else {
            panic!("for");
        };
        let span = Span::new(40, 60);

        for failure in [None, Some("iterable"), Some("body")] {
            let mut statement = function.body[0].clone();
            let TypedStatementKind::For { iterable, body, .. } = &mut statement.kind else {
                unreachable!();
            };
            let invalid = TypedStatement {
                kind: TypedStatementKind::Continue {
                    target: LoopId(999),
                },
                span,
            };
            match failure {
                Some("iterable") => {
                    let TypedExprKind::Block(block) = &mut iterable.kind else {
                        unreachable!();
                    };
                    block.statements.push(invalid);
                }
                Some("body") => {
                    let TypedStatementKind::If { then, .. } = &mut body[1].kind else {
                        unreachable!();
                    };
                    then.push(invalid);
                }
                None => {}
                _ => unreachable!(),
            }

            let mut emitter = BodyEmitter::new(function, &layout).unwrap();
            let exit = emitter.begin_block().unwrap();
            let back = emitter.begin_loop().unwrap();
            emitter.enter_loop(LoopId(5000), LoopKind::Statement, exit, back);
            let labels = emitter.labels.clone();
            let loops = emitter.loops.clone();
            let initialized = emitter.context.initialized.clone();
            let first_scratch = emitter.context.next_temporary.get();
            let result = emitter.emit_block(&vec![statement]);
            if failure.is_some() {
                assert_eq!(
                    result,
                    Err(CodegenError::InvalidLoopTarget {
                        function: "test".into(),
                        target: LoopId(999),
                        span,
                    })
                );
            } else {
                assert_eq!(result, Ok(Flow::FALLTHROUGH));
            }

            assert_eq!(emitter.labels, labels);
            assert_eq!(emitter.loops, loops);
            assert_eq!(emitter.context.initialized, initialized);
            assert_eq!(emitter.context.next_temporary.get(), first_scratch + 5);
            assert!(emitter.seen_loops.contains(&id));
            assert!(matches!(
                emitter.resolve(variable, Type::Number, span),
                Err(CodegenError::UninitializedVariable { .. })
            ));
            assert_eq!(emitter.label_depth(back).unwrap().raw(), 0);
            assert_eq!(emitter.label_depth(exit).unwrap().raw(), 1);
            assert_eq!(emitter.check_writable(function.params[0].id, span), Ok(()));
            emitter.emit_block(&vec![function.body[1].clone()]).unwrap();
            assert_eq!(emitter.context.next_temporary.get(), first_scratch + 6);
        }
    }

    #[test]
    fn statement_and_value_loop_cleanup_restores_scopes_on_body_and_result_errors() {
        use crate::typed_ast::{TypedExpr, TypedExprKind, TypedStatement};

        let program = Compiler::new().analyze(
            "fn test(param: number) -> void { let inside: number[] = [1]; let after: number[] = [2]; }",
        ).unwrap();
        let layout = Layout::new(&program).unwrap();
        let function = &program.functions[0];
        let TypedStatementKind::Let { id: inside, .. } = function.body[0].kind else {
            panic!("let");
        };
        let span = Span::new(40, 60);

        for kind in [LoopKind::Statement, LoopKind::Value(Type::Number.into())] {
            for failure in [None, Some("target"), Some("kind"), Some("result")] {
                if kind == LoopKind::Statement && failure == Some("result") {
                    continue;
                }
                let mut emitter = BodyEmitter::new(function, &layout).unwrap();
                let exit = emitter.begin_block().unwrap();
                let back = emitter.begin_loop().unwrap();
                emitter.enter_loop(LoopId(5000), LoopKind::Value(Type::Bool.into()), exit, back);
                let labels = emitter.labels.clone();
                let loops = emitter.loops.clone();
                let initialized = emitter.context.initialized.clone();
                let first_temporary = emitter.context.next_temporary.get();
                let next_label = emitter.next_label;
                let target = if failure == Some("target") {
                    LoopId(999)
                } else {
                    LoopId(77)
                };
                let break_ = match (kind, failure) {
                    (LoopKind::Statement, Some("kind")) => TypedStatementKind::BreakValue {
                        target,
                        value: TypedExpr {
                            kind: TypedExprKind::Number("1".into()),
                            ty: Type::Number.into(),
                        },
                    },
                    (LoopKind::Statement, _) | (LoopKind::Value(_), Some("kind")) => {
                        TypedStatementKind::Break { target }
                    }
                    (LoopKind::Value(_), _) => TypedStatementKind::BreakValue {
                        target,
                        value: if failure == Some("result") {
                            TypedExpr {
                                kind: TypedExprKind::Bool(true),
                                ty: Type::Bool.into(),
                            }
                        } else {
                            TypedExpr {
                                kind: TypedExprKind::Number("1".into()),
                                ty: Type::Number.into(),
                            }
                        },
                    },
                };
                let body = vec![
                    function.body[0].clone(),
                    TypedStatement { kind: break_, span },
                ];
                let result = super::super::control_flow::emit_loop(
                    LoopId(77),
                    &body,
                    kind,
                    &mut emitter,
                    span,
                );
                match failure {
                    None => assert!(result.is_ok()),
                    Some("target") => assert!(matches!(
                        result,
                        Err(CodegenError::InvalidLoopTarget { .. })
                    )),
                    Some("kind") => assert!(matches!(
                        result,
                        Err(CodegenError::MissingBreakValue { .. }
                            | CodegenError::BreakValueNotAllowed { .. })
                    )),
                    Some("result") => {
                        assert!(matches!(result, Err(CodegenError::TypeMismatch { .. })))
                    }
                    _ => unreachable!(),
                }
                assert_eq!(emitter.labels, labels);
                assert_eq!(emitter.loops, loops);
                assert_eq!(emitter.context.initialized, initialized);
                assert_eq!(emitter.context.next_temporary.get(), first_temporary + 1);
                assert!(emitter.next_label >= next_label + 2);
                assert!(emitter.seen_loops.contains(&LoopId(77)));
                assert!(matches!(
                    emitter.resolve(inside, Type::NumberArray, span),
                    Err(CodegenError::UninitializedVariable { .. })
                ));
                assert_eq!(emitter.label_depth(exit).unwrap().raw(), 1);
                assert_eq!(emitter.label_depth(back).unwrap().raw(), 0);
                emitter.emit_block(&vec![function.body[1].clone()]).unwrap();
                assert_eq!(emitter.context.next_temporary.get(), first_temporary + 2);
            }
        }
    }

    #[test]
    fn short_circuit_condition_errors_restore_nested_expression_labels_and_loop_context() {
        let program = Compiler::new()
            .analyze("fn test() -> void { while false && (true || false) {} }")
            .unwrap();
        let layout = Layout::new(&program).unwrap();
        let mut statement = program.functions[0].body[0].clone();
        let TypedStatementKind::While { cond, .. } = &mut statement.kind else {
            panic!("expected loop");
        };
        let crate::typed_ast::TypedExprKind::Binary { right, .. } = &mut cond.kind else {
            panic!("expected and");
        };
        let crate::typed_ast::TypedExprKind::Binary { right, .. } = &mut right.kind else {
            panic!("expected or");
        };
        right.kind = crate::typed_ast::TypedExprKind::Identifier {
            id: VariableId(999),
            name: "missing".into(),
        };
        let span = statement.span;
        let mut emitter = BodyEmitter::new(&program.functions[0], &layout).unwrap();
        let label = emitter.begin_block().unwrap();

        assert_eq!(
            emitter.emit_block(&vec![statement]),
            Err(CodegenError::UnknownVariable {
                function: "test".into(),
                id: VariableId(999),
                span,
            })
        );
        assert_eq!(emitter.labels, vec![label]);
        assert!(emitter.loops.is_empty());
        assert_eq!(emitter.next_label, 5);
        assert_eq!(emitter.label_depth(label).unwrap().raw(), 0);
    }

    #[test]
    fn every_structured_label_has_a_unique_identity_and_dynamic_depth() {
        let program = Compiler::new().analyze("fn test() -> void {}").unwrap();
        let layout = Layout::new(&program).unwrap();
        let mut emitter = BodyEmitter::new(&program.functions[0], &layout).unwrap();
        let block = emitter.begin_block().unwrap();
        let loop_ = emitter.begin_loop().unwrap();
        let if_ = emitter.begin_if(BlockType::Empty).unwrap();
        let nested = emitter.begin_block().unwrap();
        let labels = [block, loop_, if_, nested];

        for (index, label) in labels.iter().enumerate() {
            assert!(!labels[..index].contains(label));
            assert_eq!(
                emitter.label_depth(*label).unwrap().raw(),
                u32::try_from(3 - index).unwrap()
            );
        }

        emitter.end();
        emitter.end();
        let next_if = emitter.begin_if(BlockType::Result(ValType::I32)).unwrap();
        assert!(!labels.contains(&next_if));
        assert_eq!(emitter.label_depth(block).unwrap().raw(), 2);
        assert_eq!(emitter.label_depth(loop_).unwrap().raw(), 1);
        assert_eq!(emitter.label_depth(next_if).unwrap().raw(), 0);
    }

    #[test]
    fn value_expression_scopes_restore_bindings_and_labels_after_tail_errors() {
        use crate::typed_ast::{TypedExprKind, VariableId};

        let original = Compiler::new().analyze(
            "fn test(param: number) -> void { { let inside = [1]; 2 }; { let after = [3]; 4 }; }",
        ).unwrap();

        for fail in [false, true] {
            let mut program = original.clone();
            let TypedStatementKind::Expression(first) = &mut program.functions[0].body[0].kind
            else {
                panic!("expression");
            };
            let TypedExprKind::Block(block) = &mut first.kind else {
                panic!("value block");
            };
            let TypedStatementKind::Let { id: inside, .. } = block.statements[0].kind else {
                panic!("declaration");
            };
            if fail {
                block.tail.as_mut().unwrap().kind = TypedExprKind::Identifier {
                    id: VariableId(999),
                    name: "unknown".into(),
                };
            }
            let layout = Layout::new(&program).unwrap();
            let function = &program.functions[0];
            let mut emitter = BodyEmitter::new(function, &layout).unwrap();
            let initialized = emitter.context.initialized.clone();
            let first_temporary = emitter.context.first_temporary;
            let result = statement::emit(&function.body[0], &mut emitter);
            if fail {
                assert!(matches!(
                    result,
                    Err(CodegenError::UnknownVariable {
                        id: VariableId(999),
                        ..
                    })
                ));
            } else {
                assert_eq!(result, Ok(Flow::FALLTHROUGH));
            }
            assert_eq!(emitter.context.initialized, initialized);
            assert!(emitter.labels.is_empty());
            assert!(emitter.loops.is_empty());
            assert_eq!(emitter.context.next_temporary.get(), first_temporary + 1);
            assert!(matches!(
                emitter.resolve(inside, Type::NumberArray, function.span),
                Err(CodegenError::UninitializedVariable { .. })
            ));
            let next_label = emitter.next_label;
            assert_eq!(
                statement::emit(&function.body[1], &mut emitter),
                Ok(Flow::FALLTHROUGH)
            );
            assert_eq!(emitter.context.initialized, initialized);
            assert_eq!(emitter.context.next_temporary.get(), first_temporary + 2);
            assert!(emitter.next_label > next_label);
        }
    }

    #[test]
    fn match_arm_errors_restore_labels_and_bindings_without_reusing_scratch() {
        use crate::{
            ast::{MatchPattern, MatchPatternKind},
            typed_ast::{TypedExpr, TypedExprKind, TypedMatchArm, VariableId},
        };

        for fail in [false, true] {
            let mut program = Compiler::new()
                .analyze("fn test(param: number) -> void { { let inside = [1]; 2 }; [3]; }")
                .unwrap();
            let function = &mut program.functions[0];
            let TypedStatementKind::Expression(mut arm_body) = function.body.remove(0).kind else {
                panic!("expression");
            };
            let TypedExprKind::Block(block) = &mut arm_body.kind else {
                panic!("block");
            };
            let TypedStatementKind::Let { id: inside, .. } = block.statements[0].kind else {
                panic!("declaration");
            };
            if fail {
                block.tail.as_mut().unwrap().kind = TypedExprKind::Identifier {
                    id: VariableId(999),
                    name: "unknown".into(),
                };
            }
            let span = function.span;
            function.body.insert(
                0,
                crate::typed_ast::TypedStatement {
                    kind: TypedStatementKind::Match {
                        scrutinee: Box::new(TypedExpr {
                            kind: TypedExprKind::Identifier {
                                id: function.params[0].id,
                                name: "param".into(),
                            },
                            ty: Type::Number.into(),
                        }),
                        arms: vec![
                            TypedMatchArm {
                                pattern: MatchPattern {
                                    kind: MatchPatternKind::Number("1".into()),
                                    span,
                                },
                                body: arm_body,
                                span,
                            },
                            TypedMatchArm {
                                pattern: MatchPattern {
                                    kind: MatchPatternKind::Wildcard,
                                    span,
                                },
                                body: TypedExpr {
                                    kind: TypedExprKind::Bool(true),
                                    ty: Type::Bool.into(),
                                },
                                span,
                            },
                        ],
                    },
                    span,
                },
            );
            let layout = Layout::new(&program).unwrap();
            let function = &program.functions[0];
            let mut emitter = BodyEmitter::new(function, &layout).unwrap();
            let outer = emitter.begin_block().unwrap();
            let initialized = emitter.context.initialized.clone();
            let first = emitter.context.first_temporary;
            let result = statement::emit(&function.body[0], &mut emitter);
            if fail {
                assert!(matches!(result, Err(CodegenError::UnknownVariable { .. })));
            } else {
                assert_eq!(result, Ok(Flow::FALLTHROUGH));
            }
            assert_eq!(emitter.labels, [outer]);
            assert!(emitter.loops.is_empty());
            assert_eq!(emitter.context.initialized, initialized);
            assert_eq!(emitter.context.next_temporary.get(), first + 2);
            assert!(matches!(
                emitter.resolve(inside, Type::NumberArray, span),
                Err(CodegenError::UninitializedVariable { .. })
            ));
            let next_label = emitter.next_label;
            assert_eq!(
                statement::emit(&function.body[1], &mut emitter),
                Ok(Flow::FALLTHROUGH)
            );
            assert_eq!(emitter.context.next_temporary.get(), first + 3);
            assert_eq!(emitter.next_label, next_label);
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
