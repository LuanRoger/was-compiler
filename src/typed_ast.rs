use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    host::{HostFunction, HostFunctionId},
    tokens::Span,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionId(pub usize);

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

    Array(Vec<TypedExpr>),

    Index {
        expr: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
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

    While {
        id: LoopId,
        cond: Box<TypedExpr>,
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
    pub body: TypedBlock,
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
