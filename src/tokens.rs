use std::borrow::Cow;

use crate::span::Span;

#[derive(Debug, Clone)]
pub struct Token<'a> {
    pub kind: TokenKind<'a>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    Fn,
    Let,
    If,
    Else,
    While,
    Return,

    True,
    False,

    Number,
    Bool,
    String,
    Void,

    Identifier(Cow<'a, str>),
    NumberLiteral(String),
    StringLiteral(String),

    LeftParen,
    RightParen,

    LeftBrace,
    RightBrace,

    LeftBracket,
    RightBracket,

    Comma,
    Colon,
    Semicolon,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    Equal,
    EqualEqual,
    Bang,
    BangEqual,

    Less,
    LessEqual,
    Greater,
    GreaterEqual,

    AndAnd,
    OrOr,

    Arrow,

    Eof,
    Invalid,
}

impl<'a> Token<'a> {
    pub fn new(kind: TokenKind<'a>, span: Span) -> Self {
        Self { kind, span }
    }
}
