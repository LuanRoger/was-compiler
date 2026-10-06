use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    host::{HostFunction, HostFunctionId},
    tokens::Span,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariableId(pub usize);

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

#[derive(Debug, PartialEq)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Type,
}

#[derive(Debug, PartialEq)]
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

    Call {
        func: FunctionRef,
        name: String,
        args: Vec<TypedExpr>,
    },

    Array(Vec<TypedExpr>),

    Index {
        expr: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
}

#[derive(Debug, PartialEq)]
pub struct TypedStatement {
    pub kind: TypedStatementKind,
    pub span: Span,
}

pub type TypedBlock = Vec<TypedStatement>;

#[derive(Debug, PartialEq)]
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

    If {
        cond: Box<TypedExpr>,
        then: TypedBlock,
        else_: Option<TypedBlock>,
    },

    While {
        cond: Box<TypedExpr>,
        body: TypedBlock,
    },

    Return(Option<TypedExpr>),
    Expression(TypedExpr),
}

#[derive(Debug, PartialEq)]
pub struct TypedFunction {
    pub id: FunctionId,
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: TypedBlock,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct TypedParam {
    pub id: VariableId,
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct TypedProgram {
    pub host_functions: Vec<HostFunction>,
    pub functions: Vec<TypedFunction>,
}
