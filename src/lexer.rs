use std::{iter::Peekable, slice::Iter};

use crate::tokens::{Span, Token, TokenKind};

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
                b':' => self.make_token(TokenKind::Colon),
                b';' => self.make_token(TokenKind::Semicolon),
                b'"' => self.string(),
                char if SYMBOLS.contains(char) => self.par_symbol(char),
                char if char.is_ascii_digit() || *char == b'.' => self.number(),
                char if char.is_ascii_alphabetic() || *char == b'_' => self.identifier(),
                _ => self.make_token(TokenKind::Invalid),
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
            b'&' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'&') {
                    self.advance();
                    self.make_token(TokenKind::AndAnd)
                } else {
                    self.make_token(TokenKind::Invalid)
                }
            }
            b'|' => {
                if self.source_iter.peek().is_some_and(|&&char| char == b'|') {
                    self.advance();
                    self.make_token(TokenKind::OrOr)
                } else {
                    self.make_token(TokenKind::Invalid)
                }
            }
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

    fn number(&mut self) -> Token<'a> {
        while self
            .source_iter
            .peek()
            .is_some_and(|&&char| char.is_ascii_digit() || char == b'.')
        {
            self.advance();
        }

        let number_bytes = &self.source_bytes[self.start..self.current];
        if number_bytes[0] == b'.'
            || number_bytes.last() == Some(&b'.')
            || number_bytes.iter().filter(|&&char| char == b'.').count() > 1
        {
            return self.make_token(TokenKind::Invalid);
        }

        let number_literal = String::from_utf8_lossy(number_bytes);
        self.make_token(TokenKind::NumberLiteral(number_literal.into_owned()))
    }

    fn string(&mut self) -> Token<'a> {
        while self.source_iter.peek().is_some_and(|&&char| char != b'"') {
            self.advance();
        }

        if self.advance().is_none() {
            return self.make_token(TokenKind::Invalid);
        }

        let value = String::from_utf8_lossy(&self.source_bytes[self.start + 1..self.current - 1]);
        self.make_token(TokenKind::StringLiteral(value.into_owned()))
    }

    fn identifier(&mut self) -> Token<'a> {
        while self
            .source_iter
            .peek()
            .is_some_and(|&&char| char.is_ascii_alphanumeric() || char == b'_')
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

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind<'_>> {
        Lexer::new(source)
            .tokenize()
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn tokenizes_colons_and_semicolons() {
        assert!(matches!(
            kinds(":;").as_slice(),
            [TokenKind::Colon, TokenKind::Semicolon, TokenKind::Eof]
        ));
    }

    #[test]
    fn tokenizes_string_contents_without_quotes() {
        let tokens = kinds("\"\" \"Hello world_123-.,!?\";\"next\"");
        assert!(matches!(
            tokens.as_slice(),
            [
                TokenKind::StringLiteral(empty),
                TokenKind::StringLiteral(content),
                TokenKind::Semicolon,
                TokenKind::StringLiteral(next),
                TokenKind::Eof,
            ] if empty.is_empty() && content == "Hello world_123-.,!?" && next == "next"
        ));
    }

    #[test]
    fn rejects_unterminated_strings() {
        for source in ["\"", "\"unfinished", "\"unfinished;"] {
            assert!(matches!(
                kinds(source).as_slice(),
                [TokenKind::Invalid, TokenKind::Eof]
            ));
        }
    }

    #[test]
    fn consumes_both_bytes_of_logical_operators() {
        assert!(matches!(
            kinds("true&&false||true").as_slice(),
            [
                TokenKind::True,
                TokenKind::AndAnd,
                TokenKind::False,
                TokenKind::OrOr,
                TokenKind::True,
                TokenKind::Eof,
            ]
        ));
        assert!(matches!(
            kinds("&&& |||").as_slice(),
            [
                TokenKind::AndAnd,
                TokenKind::Invalid,
                TokenKind::OrOr,
                TokenKind::Invalid,
                TokenKind::Eof,
            ]
        ));
    }

    #[test]
    fn rejects_single_logical_operator_bytes_without_consuming_the_next_token() {
        assert!(matches!(
            kinds("&true |false & |").as_slice(),
            [
                TokenKind::Invalid,
                TokenKind::True,
                TokenKind::Invalid,
                TokenKind::False,
                TokenKind::Invalid,
                TokenKind::Invalid,
                TokenKind::Eof,
            ]
        ));
    }

    #[test]
    fn allows_digits_after_identifier_start() {
        let tokens = kinds("value1 _2 A3_b4 fn1 true2 1abc");
        assert!(matches!(
            tokens.as_slice(),
            [
                TokenKind::Identifier(a),
                TokenKind::Identifier(b),
                TokenKind::Identifier(c),
                TokenKind::Identifier(d),
                TokenKind::Identifier(e),
                TokenKind::NumberLiteral(number),
                TokenKind::Identifier(f),
                TokenKind::Eof,
            ] if a == "value1" && b == "_2" && c == "A3_b4" && d == "fn1"
                && e == "true2" && number == "1" && f == "abc"
        ));
    }

    #[test]
    fn tokenizes_integers_and_decimals() {
        let tokens = kinds("0 123 0.5 12.34+5;");
        assert!(matches!(
            tokens.as_slice(),
            [
                TokenKind::NumberLiteral(a),
                TokenKind::NumberLiteral(b),
                TokenKind::NumberLiteral(c),
                TokenKind::NumberLiteral(d),
                TokenKind::Plus,
                TokenKind::NumberLiteral(e),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ] if a == "0" && b == "123" && c == "0.5" && d == "12.34" && e == "5"
        ));
    }

    #[test]
    fn rejects_malformed_numbers_as_single_tokens() {
        for source in [".", ".5", "1.", "1.2.3", "1..2", "..", "1.2."] {
            let source = format!("{source};");
            assert!(matches!(
                kinds(&source).as_slice(),
                [TokenKind::Invalid, TokenKind::Semicolon, TokenKind::Eof]
            ));
        }
    }

    #[test]
    fn unknown_characters_are_invalid_and_lexing_continues() {
        assert!(matches!(
            kinds("@#$;true").as_slice(),
            [
                TokenKind::Invalid,
                TokenKind::Invalid,
                TokenKind::Invalid,
                TokenKind::Semicolon,
                TokenKind::True,
                TokenKind::Eof,
            ]
        ));
    }

    #[test]
    fn tokenizes_a_function_using_grammar_literals() {
        let tokens = kinds(
            "fn greet1(name2: string) -> string { let ready3: bool = true && false || true; return \"Hello!\"; }",
        );
        assert_eq!(
            tokens,
            vec![
                TokenKind::Fn,
                TokenKind::Identifier("greet1".into()),
                TokenKind::LeftParen,
                TokenKind::Identifier("name2".into()),
                TokenKind::Colon,
                TokenKind::String,
                TokenKind::RightParen,
                TokenKind::Arrow,
                TokenKind::String,
                TokenKind::LeftBrace,
                TokenKind::Let,
                TokenKind::Identifier("ready3".into()),
                TokenKind::Colon,
                TokenKind::Bool,
                TokenKind::Equal,
                TokenKind::True,
                TokenKind::AndAnd,
                TokenKind::False,
                TokenKind::OrOr,
                TokenKind::True,
                TokenKind::Semicolon,
                TokenKind::Return,
                TokenKind::StringLiteral("Hello!".into()),
                TokenKind::Semicolon,
                TokenKind::RightBrace,
                TokenKind::Eof,
            ]
        );
    }
}
