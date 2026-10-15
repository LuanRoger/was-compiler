use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    convert::Infallible,
    ops::ControlFlow,
};

use wasm_encoder::{Function, Instruction};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{
        BindingOwner, CallableId, TypedClosure, TypedClosureBody, TypedFunction, TypedParam,
        TypedProgram, TypedStatement, TypedStatementKind, VariableId,
    },
};

use super::super::{
    CodegenError, abi,
    abi::Signature,
    checked_u32, expect_operand, expect_result, expression,
    indices::{FunctionIndex, LocalIndex, TypeIndex},
    layout::Layout,
    locals::{BindingKind, LocalLayout},
    walker::{self, Visitor},
};
use super::{BodyEmitter, validate_parameters};

pub(in crate::codegen) fn invalid(reason: &'static str, span: Span) -> CodegenError {
    CodegenError::InvalidClosure { reason, span }
}

/// A borrowed entry into an existing owned body, not a generated typed function.
#[derive(Clone, Copy)]
pub(in crate::codegen) enum BodyOwner<'a> {
    Function(&'a TypedFunction),
    Closure(&'a TypedClosure),
}

impl<'a> BodyOwner<'a> {
    pub fn owner(self) -> BindingOwner {
        match self {
            Self::Function(f) => BindingOwner::Function(f.id),
            Self::Closure(c) => BindingOwner::Closure(c.id),
        }
    }

    pub fn name(self) -> &'a str {
        match self {
            Self::Function(f) => &f.name,
            Self::Closure(_) => "<closure>",
        }
    }

    pub fn params(self) -> &'a [TypedParam] {
        match self {
            Self::Function(f) => &f.params,
            Self::Closure(c) => &c.params,
        }
    }

    pub fn return_type(self) -> Type {
        match self {
            Self::Function(f) => f.return_type,
            Self::Closure(c) => c.return_type,
        }
    }

    pub fn walk<V: Visitor<'a>>(self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Self::Function(f) => walker::walk_function(f, visitor),
            Self::Closure(c) => walker::walk_closure_body(&c.body, visitor),
        }
    }
}

/// Each activated declaration links to its previous lexical prefix. Retained
/// permissions therefore take one fixed-size link per declaration, not a set per helper.
#[derive(Clone, Copy, Default)]
pub(in crate::codegen) struct LexicalPrefix {
    last: Option<CallableId>,
}

impl LexicalPrefix {
    pub fn activate(&mut self, callable: CallableId) {
        self.last = Some(callable);
    }

    pub fn contains(self, callable: CallableId, plans: &[CallablePlan<'_>]) -> bool {
        let mut last = self.last;
        while let Some(candidate) = last {
            if candidate == callable {
                return true;
            }
            last = plans[candidate.0]
                .creator
                .get()
                .expect("activated declarations link checked lexical prefixes")
                .lexical_prefix
                .last;
        }
        false
    }
}

pub(in crate::codegen) struct CallablePlan<'a> {
    pub declaration: &'a TypedClosure,
    pub dependency_slots: HashMap<CallableId, usize>,
    pub needs_context: bool,
    environment_size: u32,
    pub type_index: TypeIndex,
    pub function_index: FunctionIndex,
    // Planned facts are usable before creator checking; these facts are not.
    creator: OnceCell<CreatorEstablished>,
}

struct CreatorEstablished {
    // Original owner/declaration order, independent of forged sparse IDs.
    capture_order: Vec<(usize, u32)>,
    lexical_prefix: LexicalPrefix,
}

/// Plan context requirements before creator checks, without trusting forged metadata.
pub(in crate::codegen) fn plan<'a>(
    program: &'a TypedProgram,
    first_callable: u32,
    mut intern: impl FnMut(Signature) -> Result<TypeIndex, CodegenError>,
) -> Result<Vec<CallablePlan<'a>>, CodegenError> {
    let mut plans: Vec<CallablePlan<'a>> = Vec::new();
    let declarations = discover(program)?;
    first_callable
        .checked_add(checked_u32(declarations.len(), "callable count")?)
        .ok_or(CodegenError::LimitExceeded {
            what: "total function count",
        })?;
    for (index, declaration) in declarations.into_iter().enumerate() {
        let mut signature = abi::internal_signature(
            "<closure>",
            declaration.params.iter().map(|p| p.ty),
            declaration.return_type,
        )?;
        let mut dependency_slots = HashMap::new();
        let mut slot = declaration.captures.len();
        for &dependency in &declaration.dependencies {
            // Source-preorder earlier dependencies make context discovery acyclic.
            if dependency.0 >= index {
                // Preserve declaration/body diagnostic staging for forged metadata.
                continue;
            }
            if plans[dependency.0].needs_context {
                dependency_slots.insert(dependency, slot);
                slot = slot.checked_add(1).ok_or(CodegenError::LimitExceeded {
                    what: "closure environment slots",
                })?;
            }
        }
        let needs_context = slot != 0;
        if needs_context {
            signature.params.insert(0, wasm_encoder::ValType::I32);
            checked_u32(signature.params.len(), "closure parameter count")?;
        }
        let environment_size = abi::environment_size(slot)?;
        let type_index = intern(signature)?;
        plans.push(CallablePlan {
            declaration,
            dependency_slots,
            needs_context,
            environment_size,
            type_index,
            function_index: FunctionIndex::new(
                first_callable + checked_u32(index, "callable index")?,
            ),
            creator: OnceCell::new(),
        });
    }
    Ok(plans)
}

fn discover(program: &TypedProgram) -> Result<Vec<&TypedClosure>, CodegenError> {
    struct Discovery<'a> {
        owner: BindingOwner,
        parents: Vec<BindingOwner>,
        declarations: Vec<&'a TypedClosure>,
    }

    impl<'a> Visitor<'a> for Discovery<'a> {
        type Break = CodegenError;

        fn enter_closure(&mut self, closure: &'a TypedClosure) -> ControlFlow<CodegenError> {
            if closure.id.0 != self.declarations.len() {
                return ControlFlow::Break(invalid("invalid callable identity", closure.span));
            }
            if closure.owner != self.owner {
                return ControlFlow::Break(invalid("invalid closure ownership", closure.span));
            }
            self.declarations.push(closure);
            self.parents.push(self.owner);
            self.owner = BindingOwner::Closure(closure.id);
            ControlFlow::Continue(())
        }

        fn exit_closure(&mut self, _: &'a TypedClosure) -> ControlFlow<CodegenError> {
            self.owner = self
                .parents
                .pop()
                .expect("structural closure entry precedes exit");
            ControlFlow::Continue(())
        }
    }

    let mut discovery = Discovery {
        owner: BindingOwner::Function(crate::typed_ast::FunctionId(0)),
        parents: vec![],
        declarations: vec![],
    };

    for function in &program.functions {
        discovery.owner = BindingOwner::Function(function.id);
        if let ControlFlow::Break(error) = walker::walk_function(function, &mut discovery) {
            return Err(error);
        }
    }

    Ok(discovery.declarations)
}

/// Owner-local storage discovery does not enter helper bodies or validate captures.
pub(in crate::codegen) struct Storage {
    pub cells: HashMap<VariableId, LocalIndex>,
    pub environments: HashMap<CallableId, LocalIndex>,
    pub count: u32,
}

impl Storage {
    pub fn new(
        owner: BodyOwner<'_>,
        locals: &LocalLayout,
        first: u32,
        plans: &[CallablePlan<'_>],
    ) -> Result<Self, CodegenError> {
        struct Find<'a, 'b> {
            plans: &'a [CallablePlan<'b>],
            bindings: HashSet<VariableId>,
            callables: Vec<CallableId>,
            owner: BindingOwner,
        }

        impl Visitor<'_> for Find<'_, '_> {
            type Break = Infallible;

            fn visit_closure_bodies(&self) -> bool {
                false
            }

            fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<Self::Break> {
                if let TypedStatementKind::Closure(c) = &statement.kind {
                    if self.plans[c.id.0].needs_context {
                        self.callables.push(c.id);
                    }
                    for capture in &c.captures {
                        if capture.owner == self.owner {
                            self.bindings.insert(capture.variable);
                        }
                    }
                }
                ControlFlow::Continue(())
            }
        }

        let mut find = Find {
            plans,
            bindings: Default::default(),
            callables: vec![],
            owner: owner.owner(),
        };
        let _ = owner.walk(&mut find);
        let mut bindings: Vec<_> = find
            .bindings
            .into_iter()
            .filter_map(|id| locals.get(id).map(|local| (local.index.raw(), id)))
            .collect();
        bindings.sort_by_key(|(index, _)| *index);
        let mut next = first;
        let mut allocate = || {
            let index = LocalIndex::new(next);
            next = next.checked_add(1).ok_or(CodegenError::LimitExceeded {
                what: "closure storage locals",
            })?;
            Ok::<_, CodegenError>(index)
        };

        let mut cells = HashMap::new();
        for (_, id) in bindings {
            cells.insert(id, allocate()?);
        }

        let mut environments = HashMap::new();
        for id in find.callables {
            environments.insert(id, allocate()?);
        }
        Ok(Self {
            cells,
            environments,
            count: next - first,
        })
    }
}

impl<'a> BodyEmitter<'a> {
    fn use_dependency(&mut self, id: CallableId) {
        let declaration = self.context.module.callables[id.0].declaration;
        if matches!(self.context.owner, BindingOwner::Closure(_))
            && declaration.owner != self.context.owner
        {
            self.context.used_dependencies.insert(id.0);
        }
    }

    pub fn declare_callable(&mut self, closure: &TypedClosure) -> Result<(), CodegenError> {
        let invalid = |reason| invalid(reason, closure.span);
        let plan = self
            .context
            .module
            .callables
            .get(closure.id.0)
            .filter(|plan| std::ptr::eq(plan.declaration, closure))
            .ok_or_else(|| invalid("invalid callable identity"))?;
        if closure.owner != self.context.owner {
            return Err(invalid("invalid closure ownership"));
        }
        let mut previous = None;
        let mut capture_order = Vec::with_capacity(closure.captures.len());
        for capture in &closure.captures {
            let key = if capture.owner == self.context.owner {
                let local = self
                    .context
                    .locals
                    .get(capture.variable)
                    .ok_or_else(|| invalid("invalid capture metadata"))?;
                let mutability = match local.kind {
                    BindingKind::Mutable => crate::typed_ast::CaptureMutability::Mutable,
                    BindingKind::Iteration => crate::typed_ast::CaptureMutability::ReadOnly,
                    BindingKind::Call => return Err(invalid("invalid capture metadata")),
                };
                if local.ty != capture.ty
                    || capture.mutability != mutability
                    || !self.context.initialized.contains(&capture.variable)
                {
                    return Err(invalid("invalid capture metadata"));
                }
                let owner_order = match self.context.owner {
                    BindingOwner::Function(_) => 0,
                    BindingOwner::Closure(id) => id.0 + 1,
                };
                (owner_order, local.index.raw())
            } else {
                let slot = self.inherited_capture(capture, closure.span)?;
                self.context.used_captures.insert(slot);
                let BindingOwner::Closure(id) = self.context.owner else {
                    return Err(invalid("invalid capture metadata"));
                };
                *self.context.module.callables[id.0]
                    .creator
                    .get()
                    .and_then(|creator| creator.capture_order.get(slot))
                    .ok_or_else(|| invalid("invalid capture metadata"))?
            };
            if previous.is_some_and(|previous| key <= previous) {
                return Err(invalid("invalid capture metadata"));
            }
            previous = Some(key);
            capture_order.push(key);
        }
        let mut previous = None;
        for &dependency in &closure.dependencies {
            if previous.is_some_and(|id| dependency.0 <= id)
                || dependency.0 >= closure.id.0
                || !self.context.callable_available(dependency)
                || self.context.module.callables.get(dependency.0).is_none()
            {
                return Err(invalid("invalid callable dependency"));
            }
            previous = Some(dependency.0);
            self.use_dependency(dependency);
        }
        if plan.needs_context {
            let env = self
                .context
                .storage
                .environments
                .get(&closure.id)
                .copied()
                .ok_or_else(|| invalid("invalid capture metadata"))?;
            self.allocate(plan.environment_size, env);
            for (index, capture) in closure.captures.iter().enumerate() {
                self.local_get(env);
                if capture.owner == self.context.owner {
                    let cell = self
                        .context
                        .storage
                        .cells
                        .get(&capture.variable)
                        .copied()
                        .ok_or_else(|| invalid("invalid capture metadata"))?;
                    self.local_get(cell);
                } else {
                    let slot = self.inherited_capture(capture, closure.span)?;
                    self.local_get(LocalIndex::new(0));
                    self.instruction(&Instruction::I32Load(abi::environment_slot(slot)?));
                }
                self.instruction(&Instruction::I32Store(abi::environment_slot(index)?));
            }
            for dependency in &closure.dependencies {
                if let Some(&slot) = plan.dependency_slots.get(dependency) {
                    self.local_get(env);
                    self.callable_environment(*dependency, closure.span)?;
                    self.instruction(&Instruction::I32Store(abi::environment_slot(slot)?));
                }
            }
        }
        // Publish only after checking and environment emission both succeed.
        // A failed compilation discards all plans; there is no retry protocol.
        assert!(
            plan.creator
                .set(CreatorEstablished {
                    capture_order,
                    lexical_prefix: self.context.lexical_prefix,
                })
                .is_ok(),
            "callable declarations are checked once per compilation"
        );
        self.context.lexical_prefix.activate(closure.id);
        self.context.available_callables.insert(closure.id);
        Ok(())
    }

    pub fn callable(
        &mut self,
        id: CallableId,
        span: Span,
    ) -> Result<&'a CallablePlan<'a>, CodegenError> {
        let plan = self
            .context
            .module
            .callables
            .get(id.0)
            .ok_or_else(|| invalid("invalid callable call", span))?;
        if !self.context.callable_available(id) {
            return Err(invalid("invalid callable call", span));
        }
        self.use_dependency(id);
        Ok(plan)
    }

    fn inherited_capture(
        &self,
        capture: &crate::typed_ast::TypedCapture,
        span: Span,
    ) -> Result<usize, CodegenError> {
        let invalid = || invalid("invalid capture metadata", span);
        let BindingOwner::Closure(id) = self.context.owner else {
            return Err(invalid());
        };
        let slot = *self
            .context
            .capture_slots
            .get(&(capture.owner, capture.variable))
            .ok_or_else(invalid)?;
        if self.context.module.callables[id.0]
            .declaration
            .captures
            .get(slot)
            != Some(capture)
        {
            return Err(invalid());
        }
        Ok(slot)
    }

    pub fn callable_environment(&mut self, id: CallableId, span: Span) -> Result<(), CodegenError> {
        if let Some(local) = self.context.storage.environments.get(&id).copied() {
            self.local_get(local);
        } else {
            let BindingOwner::Closure(owner) = self.context.owner else {
                return Err(invalid("invalid callable call", span));
            };
            let slot = self.context.module.callables[owner.0]
                .dependency_slots
                .get(&id)
                .copied()
                .ok_or_else(|| {
                    invalid(
                        "invalid callable dependency",
                        self.context.module.callables[owner.0].declaration.span,
                    )
                })?;
            self.local_get(LocalIndex::new(0));
            self.instruction(&Instruction::I32Load(abi::environment_slot(slot)?));
        }
        Ok(())
    }
}

pub(in crate::codegen) fn emit_closure<'a>(
    closure: &'a TypedClosure,
    module: &'a Layout<'a>,
) -> Result<Function, CodegenError> {
    let mut emitter = BodyEmitter::for_owner(BodyOwner::Closure(closure), module)?;
    emitter.context.lexical_prefix = module.callables[closure.id.0]
        .creator
        .get()
        .ok_or_else(|| invalid("invalid closure ownership", closure.span))?
        .lexical_prefix;
    emitter.context.available_callables = closure.dependencies.iter().copied().collect();
    validate_parameters(&closure.params, &mut emitter)?;
    emitter.promote_parameters(&closure.params)?;
    let (flow, result, has_tail) = match &closure.body {
        TypedClosureBody::Expression(expr) => {
            let flow = emitter.emit_expr(expr, closure.body_span)?;
            (flow, expr.ty, true)
        }
        TypedClosureBody::Block(block) => {
            let (flow, result) = expression::emit_value_contents(block, &mut emitter)?;
            (flow, result, block.tail.is_some())
        }
    };
    let tail = match &closure.body {
        TypedClosureBody::Expression(expr) => Some(expr.as_ref()),
        TypedClosureBody::Block(block) => block.tail.as_deref(),
    };
    if let Some(tail) = tail {
        expect_operand("<closure>", closure.return_type, tail.ty, closure.body_span)?;
    }
    if flow.can_fall_through() {
        if !has_tail && closure.return_type != Type::Void {
            return Err(invalid("invalid closure result", closure.body_span));
        }
        expect_result(
            "<closure>",
            closure.return_type.into(),
            result,
            closure.body_span,
        )?;
    }
    let returned = flow
        .return_type()
        .map_err(|_| invalid("invalid closure result", closure.body_span))?;
    if returned.is_none()
        && !flow.can_fall_through()
        && (has_tail || closure.return_type != Type::Void || emitter.context.value_return_syntax)
    {
        return Err(invalid("invalid closure result", closure.body_span));
    }
    if emitter.context.used_captures.len() != closure.captures.len() {
        return Err(invalid("invalid capture metadata", closure.span));
    }
    let used: Vec<_> = emitter
        .context
        .used_dependencies
        .iter()
        .copied()
        .map(CallableId)
        .collect();
    if used != closure.dependencies {
        return Err(invalid("invalid callable dependency", closure.span));
    }
    emitter.end();
    Ok(emitter.body)
}
