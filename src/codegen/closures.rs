use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    convert::Infallible,
    ops::ControlFlow,
};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{
        BindingOwner, CallableId, TypedClosure, TypedFunction, TypedParam, TypedProgram,
        TypedStatement, TypedStatementKind, VariableId,
    },
};

use super::{
    CodegenError,
    indices::{FunctionIndex, LocalIndex, TypeIndex},
    locals::LocalLayout,
    walker::{self, Visitor},
};

pub(super) fn invalid(reason: &'static str, span: Span) -> CodegenError {
    CodegenError::InvalidClosure { reason, span }
}

/// A borrowed entry into an existing owned body, not a generated typed function.
#[derive(Clone, Copy)]
pub(super) enum BodyOwner<'a> {
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
pub(super) struct LexicalPrefix {
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
                .lexical_prefix
                .get()
                .expect("activated declarations link checked lexical prefixes")
                .last;
        }
        false
    }
}

pub(super) struct CallablePlan<'a> {
    pub declaration: &'a TypedClosure,
    pub type_index: TypeIndex,
    pub function_index: FunctionIndex,
    // None means the creator has not checked this declaration yet.
    pub lexical_prefix: Cell<Option<LexicalPrefix>>,
}

pub(super) fn discover(program: &TypedProgram) -> Result<Vec<&TypedClosure>, CodegenError> {
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
pub(super) struct Storage {
    pub cells: HashMap<VariableId, LocalIndex>,
    pub environments: HashMap<CallableId, LocalIndex>,
    pub count: u32,
}

impl Storage {
    pub fn new(
        owner: BodyOwner<'_>,
        locals: &LocalLayout,
        first: u32,
    ) -> Result<Self, CodegenError> {
        struct Find {
            bindings: HashSet<VariableId>,
            callables: Vec<CallableId>,
            owner: BindingOwner,
        }

        impl Visitor<'_> for Find {
            type Break = Infallible;

            fn visit_closure_bodies(&self) -> bool {
                false
            }

            fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<Self::Break> {
                if let TypedStatementKind::Closure(c) = &statement.kind {
                    if !c.captures.is_empty() {
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
