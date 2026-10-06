use std::{iter::Peekable, slice::Iter};

use crate::{
    span::Span,
    tokens::{Token, TokenKind},
};

const SYMBOLS: &[u8] = b"<>=!&|+-*/%";

pub struct Lexer<'a> {
    source_bytes: &'a [u8],
    source_iter: Peekable<Iter<'a, u8>>,
    current: usize,
    start: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        let source_bytes = source.as_bytes();
        let source_iter = source_bytes.iter().peekable();

        Self {
            source_bytes,
            source_iter,
            start: 0,
            current: 0,
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token<'a>> {
        let mut tokens: Vec<Token<'a>> = Vec::new();

        loop {
            let new_token = self.next_token();
            let is_eof = matches!(new_token.kind, TokenKind::Eof);

            tokens.push(new_token);

            if is_eof {
                break;
            }
        }

        tokens
    }

    pub fn advance(&mut self) -> Option<&'a u8> {
        let char = self.source_iter.next();
        if let Some(char) = char {
            self.current += 1;
            Some(char)
        } else {
            None
        }
    }

    pub fn next_token(&mut self) -> Token<'a> {
        self.skip_whitespace();

        self.start = self.current;

        if let Some(char) = self.advance() {
            match char {
                b'(' => self.make_token(TokenKind::LeftParen),
                b')' => self.make_token(TokenKind::RightParen),
                b'{' => self.make_token(TokenKind::LeftBrace),
                b'}' => self.make_token(TokenKind::RightBrace),
                b'[' => self.make_token(TokenKind::LeftBracket),
                b']' => self.make_token(TokenKind::RightBracket),
                b',' => self.make_token(TokenKind::Comma),
                b'"' => self.make_token(TokenKind::Semicolon),
                char if SYMBOLS.contains(char) => self.par_symbol(char),
                char if char.is_ascii_digit() || *char == b'.' => self.make_number(),
                char if char.is_ascii_alphabetic() || *char == b'_' => self.identifier(),
                _ => todo!(),
            }
        } else {
            self.make_token(TokenKind::Eof)
        }
    }

    pub fn skip_whitespace(&mut self) {
        while self
            .source_iter
            .peek()
            .is_some_and(|char| char.is_ascii_whitespace())
        {
            self.advance();
        }
    }

    pub fn make_token(&mut self, token_kind: TokenKind<'a>) -> Token<'a> {
        Token::new(token_kind, Span::new(self.start, self.current))
    }

    pub fn par_symbol(&mut self, current_char: &u8) -> Token<'a> {
        match *current_char {
            b'<' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'=') {
                    self.advance();
                    self.make_token(TokenKind::LessEqual)
                } else {
                    self.make_token(TokenKind::Less)
                }
            }
            b'>' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'=') {
                    self.advance();
                    self.make_token(TokenKind::GreaterEqual)
                } else {
                    self.make_token(TokenKind::Greater)
                }
            }
            b'=' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'=') {
                    self.advance();
                    self.make_token(TokenKind::EqualEqual)
                } else {
                    self.make_token(TokenKind::Equal)
                }
            }
            b'!' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'=') {
                    self.advance();
                    self.make_token(TokenKind::BangEqual)
                } else {
                    self.make_token(TokenKind::Bang)
                }
            }
            b'&' => self.make_token(TokenKind::AndAnd),
            b'|' => self.make_token(TokenKind::OrOr),
            b'+' => self.make_token(TokenKind::Plus),
            b'-' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'>') {
                    self.advance();
                    self.make_token(TokenKind::Arrow)
                } else {
                    self.make_token(TokenKind::Minus)
                }
            }
            b'*' => self.make_token(TokenKind::Star),
            b'/' => self.make_token(TokenKind::Slash),
            b'%' => self.make_token(TokenKind::Percent),
            _ => self.make_token(TokenKind::Invalid),
        }
    }

    pub fn make_number(&mut self) -> Token<'a> {
        while self
            .source_iter
            .peek()
            .is_some_and(|&&char| char.is_ascii_digit() || char == b'.')
        {
            self.advance();
        }

        let number_literal = String::from_utf8_lossy(&self.source_bytes[self.start..self.current]);
        self.make_token(TokenKind::NumberLiteral(number_literal.into_owned()))
    }

    pub fn identifier(&mut self) -> Token<'a> {
        while self
            .source_iter
            .peek()
            .is_some_and(|&&char| char.is_ascii_alphabetic() || char == b'_')
        {
            self.advance();
        }

        let value = String::from_utf8_lossy(&self.source_bytes[self.start..self.current]);

        match value.as_ref() {
            "fn" => self.make_token(TokenKind::Fn),
            "return" => self.make_token(TokenKind::Return),
            "let" => self.make_token(TokenKind::Let),
            "if" => self.make_token(TokenKind::If),
            "else" => self.make_token(TokenKind::Else),
            "while" => self.make_token(TokenKind::While),
            "number" => self.make_token(TokenKind::Number),
            "bool" => self.make_token(TokenKind::Bool),
            "true" => self.make_token(TokenKind::True),
            "false" => self.make_token(TokenKind::False),
            "string" => self.make_token(TokenKind::String),
            "void" => self.make_token(TokenKind::Void),
            _ => self.make_token(TokenKind::Identifier(value)),
        }
    }
}
