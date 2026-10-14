use std::{cell::Cell, ops::ControlFlow};

use crate::{
    ast::Type,
    tokens::Span,
    typed_ast::{BindingOwner, CallableId, TypedClosure, TypedFunction, TypedParam, TypedProgram},
};

use super::{
    CodegenError,
    indices::{FunctionIndex, TypeIndex},
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
