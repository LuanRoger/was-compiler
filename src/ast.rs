use crate::tokens::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    NumberArray,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,

    Equal,
    NotEqual,

    LessThan,
    GreaterThan,
    LessThanOrEqual,
    GreaterThanOrEqual,

    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Negate,
}

#[derive(Debug, PartialEq)]
pub enum Expr {
    Number(String),
    String(String),
    Bool(bool),

    Identifier(String),

    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },

    Call {
        func: String,
        args: Vec<Expr>,
    },

    Loop {
        body: Block,
    },
    Block(ValueBlock),
    If {
        cond: Box<Expr>,
        then: ValueBlock,
        else_: Option<ValueBlock>,
    },

    Array(Vec<Expr>),
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
}

#[derive(Debug, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

pub type Block = Vec<Statement>;

#[derive(Debug, PartialEq)]
pub struct ValueBlock {
    pub statements: Block,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub enum StatementKind {
    Let {
        name: String,
        ty: Option<Type>,
        value: Expr,
    },

    Assignment {
        target: Expr,
        value: Expr,
    },

    CompoundAssignment {
        target: Expr,
        op: BinaryOp,
        value: Expr,
    },

    If {
        cond: Box<Expr>,
        then: Block,
        else_: Option<Block>,
    },

    While {
        cond: Box<Expr>,
        body: Block,
    },

    Loop {
        body: Block,
    },
    Break,
    BreakValue(Expr),
    Continue,
    Return(Option<Expr>),
    Expression(Expr),
}

#[derive(Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct Program {
    pub functions: Vec<Function>,
}
