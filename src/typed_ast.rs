use crate::{
    ast::{BinaryOp, MatchPattern, Type, UnaryOp},
    host::{HostFunction, HostFunctionId},
    tokens::Span,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub usize);

/// Program-wide identity in lexical source-discovery order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallableId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingOwner {
    Function(FunctionId),
    Closure(CallableId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMutability {
    Mutable,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedCapture {
    pub owner: BindingOwner,
    pub variable: VariableId,
    pub ty: Type,
    pub mutability: CaptureMutability,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedClosureBody {
    Expression(Box<TypedExpr>),
    Block(TypedValueBlock),
}

/// A declaration owns its helper body; it is not a first-class value.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedClosure {
    pub id: CallableId,
    pub owner: BindingOwner,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: TypedClosureBody,
    pub captures: Vec<TypedCapture>,
    pub dependencies: Vec<CallableId>,
    pub span: Span,
    pub body_span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariableId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoopId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFunction {
    Len,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionRef {
    User(FunctionId),
    Host(HostFunctionId),
    Builtin(BuiltinFunction),
}

/// An expression's normal result, separate from storage and ABI types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprResult {
    Value(Type),
    Void,
    Never,
}

impl From<Type> for ExprResult {
    fn from(ty: Type) -> Self {
        match ty {
            Type::Void => Self::Void,
            ty => Self::Value(ty),
        }
    }
}

impl ExprResult {
    pub fn value_type(self) -> Option<Type> {
        match self {
            Self::Value(ty) => Some(ty),
            Self::Void | Self::Never => None,
        }
    }
}

impl PartialEq<Type> for ExprResult {
    fn eq(&self, other: &Type) -> bool {
        *self == Self::from(*other)
    }
}

impl PartialEq<ExprResult> for Type {
    fn eq(&self, other: &ExprResult) -> bool {
        ExprResult::from(*self) == *other
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: ExprResult,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    Number(String),
    String(String),
    Bool(bool),

    Identifier {
        id: VariableId,
        name: String,
    },

    /// Access to an original binding owned by the immediate creator.
    Captured {
        owner: BindingOwner,
        id: VariableId,
        name: String,
    },

    Unary {
        op: UnaryOp,
        expr: Box<TypedExpr>,
    },

    Binary {
        left: Box<TypedExpr>,
        op: BinaryOp,
        right: Box<TypedExpr>,
    },

    Range {
        start: Box<TypedExpr>,
        end: Box<TypedExpr>,
        step: Option<Box<TypedExpr>>,
        inclusive: bool,
    },

    Call {
        func: FunctionRef,
        name: String,
        args: Vec<TypedExpr>,
    },

    NormalizedCall(TypedNormalizedCall),

    LexicalCall {
        callable: CallableId,
        args: Vec<TypedExpr>,
        span: Span,
    },

    Loop {
        id: LoopId,
        body: TypedBlock,
    },
    Block(TypedValueBlock),
    If {
        cond: Box<TypedExpr>,
        then: TypedValueBlock,
        else_: Option<TypedValueBlock>,
    },

    Match {
        scrutinee: Box<TypedExpr>,
        arms: Vec<TypedMatchArm>,
    },

    Array(Vec<TypedExpr>),

    Index {
        expr: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedMatchArm {
    pub pattern: MatchPattern,
    pub body: TypedExpr,
    pub span: Span,
}

/// Ordered invocation-local storage followed by a canonical full-arity user call.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedNormalizedCall {
    pub bindings: Vec<TypedCallBinding>,
    pub formal_to_binding: Vec<VariableId>,
    pub call: Box<TypedExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedCallBinding {
    pub id: VariableId,
    pub ty: Type,
    pub value: TypedExpr,
    pub origin: CallBindingOrigin,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallBindingOrigin {
    PipelineInput,
    Explicit { argument: usize },
    Default { formal: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedStatement {
    pub kind: TypedStatementKind,
    pub span: Span,
}

pub type TypedBlock = Vec<TypedStatement>;

#[derive(Debug, Clone, PartialEq)]
pub struct TypedValueBlock {
    pub statements: TypedBlock,
    pub tail: Option<Box<TypedExpr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedStatementKind {
    Closure(TypedClosure),

    Let {
        id: VariableId,
        name: String,
        ty: Type,
        value: TypedExpr,
    },

    Assignment {
        target: TypedExpr,
        value: TypedExpr,
    },

    CompoundAssignment {
        target: TypedExpr,
        op: BinaryOp,
        value: TypedExpr,
    },

    If {
        cond: Box<TypedExpr>,
        then: TypedBlock,
        else_: Option<TypedBlock>,
    },

    Match {
        scrutinee: Box<TypedExpr>,
        arms: Vec<TypedMatchArm>,
    },

    While {
        id: LoopId,
        cond: Box<TypedExpr>,
        body: TypedBlock,
    },

    For {
        id: LoopId,
        variable: VariableId,
        name: String,
        iterable: Box<TypedExpr>,
        body: TypedBlock,
    },

    Loop {
        id: LoopId,
        body: TypedBlock,
    },
    Break {
        target: LoopId,
    },
    BreakValue {
        target: LoopId,
        value: TypedExpr,
    },
    Continue {
        target: LoopId,
    },
    Return(Option<TypedExpr>),
    Expression(TypedExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedFunction {
    pub id: FunctionId,
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub contracts: Vec<TypedContract>,
    pub body: TypedBlock,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedContract {
    pub value: TypedExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedParam {
    pub id: VariableId,
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedProgram {
    pub host_functions: Vec<HostFunction>,
    pub functions: Vec<TypedFunction>,
}
