use crate::tokens::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Number,
    String,
    Bool,
    NumberArray,
    Range,
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
    In,

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

    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        step: Option<Box<Expr>>,
        inclusive: bool,
    },

    Pipeline {
        input: Box<Expr>,
        input_span: Span,
        stages: Vec<PipelineStage>,
    },

    Call {
        func: String,
        args: Vec<CallArgument>,
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

    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    Array(Vec<Expr>),
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
}

/// A literal-only match pattern with its original UTF-8 byte span.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchPattern {
    pub kind: MatchPatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchPatternKind {
    Wildcard,
    Number(String),
    String(String),
    Bool(bool),
    Relational {
        op: BinaryOp,
        value: String,
    },
    Interval {
        start: String,
        end: String,
        inclusive: bool,
    },
}

#[derive(Debug, PartialEq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct PipelineStage {
    pub name: String,
    pub args: Vec<CallArgument>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct CallArgument {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
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

    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    While {
        cond: Box<Expr>,
        body: Block,
    },

    For {
        name: String,
        iterable: Expr,
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
    pub return_type: Option<Type>,
    pub contracts: Vec<ContractExpression>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct ContractExpression {
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub default: Option<DefaultExpression>,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct DefaultExpression {
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, PartialEq)]
pub struct Program {
    pub functions: Vec<Function>,
}
