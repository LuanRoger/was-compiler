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
        if let Some(token) = self.skip_trivia() {
            return token;
        }

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
                b'.' if self.source_iter.peek() == Some(&&b'.') => {
                    self.advance();
                    let kind = if self.source_iter.peek() == Some(&&b'=') {
                        self.advance();
                        TokenKind::DotDotEqual
                    } else {
                        TokenKind::DotDot
                    };

                    self.make_token(kind)
                }
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

    fn skip_trivia(&mut self) -> Option<Token<'a>> {
        loop {
            self.skip_whitespace();

            match self.source_bytes[self.current..] {
                [b'/', b'/', ..] => {
                    self.advance();
                    self.advance();

                    while self
                        .source_iter
                        .peek()
                        .is_some_and(|&&char| !matches!(char, b'\n' | b'\r'))
                    {
                        self.advance();
                    }
                }

                [b'/', b'*', ..] => {
                    self.start = self.current;
                    self.advance();
                    self.advance();

                    if !self.skip_block_comment() {
                        return Some(self.make_token(TokenKind::Invalid));
                    }
                }

                _ => return None,
            }
        }
    }

    fn skip_block_comment(&mut self) -> bool {
        while let Some(&char) = self.advance() {
            if char == b'*' && self.source_iter.peek().is_some_and(|&&next| next == b'/') {
                self.advance();

                return true;
            }
        }

        false
    }

    pub fn make_token(&mut self, token_kind: TokenKind<'a>) -> Token<'a> {
        Token::new(token_kind, Span::new(self.start, self.current))
    }

    pub fn par_symbol(&mut self, current_char: &u8) -> Token<'a> {
        let compound = match (*current_char, self.source_iter.peek().map(|&&char| char)) {
            (b'+', Some(b'=')) => Some(TokenKind::PlusEqual),
            (b'-', Some(b'=')) => Some(TokenKind::MinusEqual),
            (b'*', Some(b'=')) => Some(TokenKind::StarEqual),
            (b'/', Some(b'=')) => Some(TokenKind::SlashEqual),
            (b'%', Some(b'=')) => Some(TokenKind::PercentEqual),
            _ => None,
        };

        if let Some(kind) = compound {
            self.advance();

            return self.make_token(kind);
        }

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
            if self.source_bytes[self.current..].starts_with(b"..") {
                break;
            }

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
            "loop" => self.make_token(TokenKind::Loop),
            "break" => self.make_token(TokenKind::Break),
            "continue" => self.make_token(TokenKind::Continue),
            "in" => self.make_token(TokenKind::In),
            "number" => self.make_token(TokenKind::Number),
            "bool" => self.make_token(TokenKind::Bool),
            "true" => self.make_token(TokenKind::True),
            "false" => self.make_token(TokenKind::False),
            "string" => self.make_token(TokenKind::String),
            "range" => self.make_token(TokenKind::Range),
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
    fn skips_line_comments_through_lf_cr_crlf_or_eof() {
        for ending in ["\n", "\r", "\r\n"] {
            let source = format!("1// \" /* ignored é */{ending}2// EOF");
            let tokens = Lexer::new(&source).tokenize();
            let second_start = "1// \" /* ignored é */".len() + ending.len();

            assert_eq!(tokens.len(), 3);
            assert_eq!(tokens[0].kind, TokenKind::NumberLiteral("1".into()));
            assert_eq!(tokens[0].span, Span::new(0, 1));
            assert_eq!(tokens[1].kind, TokenKind::NumberLiteral("2".into()));
            assert_eq!(tokens[1].span, Span::new(second_start, second_start + 1));
            assert_eq!(tokens[2].kind, TokenKind::Eof);
            assert_eq!(tokens[2].span, Span::new(source.len(), source.len()));
        }
    }

    #[test]
    fn skips_adjacent_and_multiline_block_comments() {
        assert_eq!(
            kinds("/**//***/ /* \" // ignored\n é */true/* trailing */"),
            vec![TokenKind::True, TokenKind::Eof]
        );

        for source in ["// EOF", "/**/", "/* multi\nline */ // EOF"] {
            let tokens = Lexer::new(source).tokenize();

            assert_eq!(tokens.len(), 1);
            assert_eq!(tokens[0].kind, TokenKind::Eof);
            assert_eq!(tokens[0].span, Span::new(source.len(), source.len()));
        }
    }

    #[test]
    fn block_comments_close_at_the_first_delimiter_without_nesting() {
        assert_eq!(
            kinds("/* outer /* inner */true */"),
            vec![
                TokenKind::True,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn comments_separate_identifier_number_and_operator_fragments() {
        assert_eq!(
            kinds("fo/**/o 1/**/2 =/**/= &/**/& -/**/>"),
            vec![
                TokenKind::Identifier("fo".into()),
                TokenKind::Identifier("o".into()),
                TokenKind::NumberLiteral("1".into()),
                TokenKind::NumberLiteral("2".into()),
                TokenKind::Equal,
                TokenKind::Equal,
                TokenKind::Invalid,
                TokenKind::Invalid,
                TokenKind::Minus,
                TokenKind::Greater,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn comment_delimiters_inside_strings_remain_literal_text() {
        assert_eq!(
            kinds("\"https://example.test/* text */\"/* ignored */\"// /* */\""),
            vec![
                TokenKind::StringLiteral("https://example.test/* text */".into()),
                TokenKind::StringLiteral("// /* */".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn preserves_division_and_star_tokens_around_comments() {
        assert_eq!(
            kinds("6/2 8/ /* divide */ 4 / * / /**/ /"),
            vec![
                TokenKind::NumberLiteral("6".into()),
                TokenKind::Slash,
                TokenKind::NumberLiteral("2".into()),
                TokenKind::NumberLiteral("8".into()),
                TokenKind::Slash,
                TokenKind::NumberLiteral("4".into()),
                TokenKind::Slash,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Slash,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn rejects_unterminated_block_comments_with_original_byte_spans() {
        for comment in ["/*", "/*/", "/* unfinished", "/* trailing *", "/* é\n/*"] {
            let source = format!("true /* prior */\r\n{comment}");
            let tokens = Lexer::new(&source).tokenize();
            let start = "true /* prior */\r\n".len();

            assert_eq!(tokens.len(), 3);
            assert_eq!(tokens[0].kind, TokenKind::True);
            assert_eq!(tokens[0].span, Span::new(0, 4));
            assert_eq!(tokens[1].kind, TokenKind::Invalid);
            assert_eq!(tokens[1].span, Span::new(start, source.len()));
            assert_eq!(tokens[2].kind, TokenKind::Eof);
            assert_eq!(tokens[2].span, Span::new(source.len(), source.len()));
        }
    }

    #[test]
    fn preserves_utf8_byte_spans_after_comments() {
        let mut lexer = Lexer::new("/*é*/\"λ\"//🙂\r\n+");
        let text = lexer.next_token();
        let plus = lexer.next_token();
        let eof = lexer.next_token();

        assert_eq!(text.kind, TokenKind::StringLiteral("λ".into()));
        assert_eq!(text.span, Span::new(6, 10));
        assert_eq!(plus.kind, TokenKind::Plus);
        assert_eq!(plus.span, Span::new(18, 19));
        assert_eq!(eof.kind, TokenKind::Eof);
        assert_eq!(eof.span, Span::new(19, 19));
        assert_eq!(lexer.next_token().span, eof.span);
    }

    #[test]
    fn compound_operators_use_longest_match_and_preserve_arrow_and_single_tokens() {
        for (symbol, compound, single) in [
            ("+", TokenKind::PlusEqual, TokenKind::Plus),
            ("-", TokenKind::MinusEqual, TokenKind::Minus),
            ("*", TokenKind::StarEqual, TokenKind::Star),
            ("/", TokenKind::SlashEqual, TokenKind::Slash),
            ("%", TokenKind::PercentEqual, TokenKind::Percent),
        ] {
            let source = format!("{symbol}== {symbol} ->");
            let tokens = Lexer::new(&source).tokenize();

            assert_eq!(tokens[0].kind, compound);
            assert_eq!(tokens[0].span, Span::new(0, 2));
            assert_eq!(tokens[1].kind, TokenKind::Equal);
            assert_eq!(tokens[1].span, Span::new(2, 3));
            assert_eq!(tokens[2].kind, single);
            assert_eq!(tokens[2].span, Span::new(4, 5));
            assert_eq!(tokens[3].kind, TokenKind::Arrow);
            assert_eq!(tokens[3].span, Span::new(6, 8));
            assert_eq!(tokens[4].kind, TokenKind::Eof);
            assert_eq!(tokens[4].span, Span::new(8, 8));
        }
    }

    #[test]
    fn compound_operator_bytes_cannot_be_joined_across_trivia() {
        for (symbol, single) in [
            ("+", TokenKind::Plus),
            ("-", TokenKind::Minus),
            ("*", TokenKind::Star),
            ("/", TokenKind::Slash),
            ("%", TokenKind::Percent),
        ] {
            for trivia in [" ", "\r\n", " /*é*/", " //é\n"] {
                let source = format!("{symbol}{trivia}=");
                let tokens = Lexer::new(&source).tokenize();

                assert_eq!(tokens.len(), 3);
                assert_eq!(tokens[0].kind, single);
                assert_eq!(tokens[0].span, Span::new(0, 1));
                assert_eq!(tokens[1].kind, TokenKind::Equal);
                assert_eq!(tokens[1].span, Span::new(1 + trivia.len(), source.len()));
                assert_eq!(tokens[2].kind, TokenKind::Eof);
            }
        }
    }

    #[test]
    fn compound_operators_preserve_comment_and_string_boundaries_and_byte_spans() {
        let source = "/*é*/+=/**/-=/*λ*/*=//🙂\r\n/=/**/%= \"+= -= *= /= %=\"";
        let tokens = Lexer::new(source).tokenize();

        assert_eq!(tokens.len(), 7);

        for (token, (text, kind)) in tokens.iter().zip([
            ("+=", TokenKind::PlusEqual),
            ("-=", TokenKind::MinusEqual),
            ("*=", TokenKind::StarEqual),
            ("/=", TokenKind::SlashEqual),
            ("%=", TokenKind::PercentEqual),
        ]) {
            let start = source.find(text).unwrap();

            assert_eq!(token.kind, kind);
            assert_eq!(token.span, Span::new(start, start + 2));
        }

        assert_eq!(
            tokens[5].kind,
            TokenKind::StringLiteral("+= -= *= /= %=".into())
        );
        assert_eq!(tokens[6].kind, TokenKind::Eof);
        assert_eq!(tokens[6].span, Span::new(source.len(), source.len()));
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
    fn loop_control_keywords_preserve_boundaries_and_utf8_spans() {
        let source = "/* é */break/* λ */;continue; break1 continued _break Break Continue \"break continue\" bre/*x*/ak";
        let tokens = Lexer::new(source).tokenize();

        assert_eq!(tokens[0].kind, TokenKind::Break);
        assert_eq!(tokens[2].kind, TokenKind::Continue);

        for (index, text) in [(0, "break"), (2, "continue")] {
            let start = source.find(text).unwrap();

            assert_eq!(tokens[index].span, Span::new(start, start + text.len()));
        }

        assert_eq!(
            tokens
                .into_iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                TokenKind::Break,
                TokenKind::Semicolon,
                TokenKind::Continue,
                TokenKind::Semicolon,
                TokenKind::Identifier("break1".into()),
                TokenKind::Identifier("continued".into()),
                TokenKind::Identifier("_break".into()),
                TokenKind::Identifier("Break".into()),
                TokenKind::Identifier("Continue".into()),
                TokenKind::StringLiteral("break continue".into()),
                TokenKind::Identifier("bre".into()),
                TokenKind::Identifier("ak".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn loop_keyword_preserves_boundaries_comments_and_byte_spans() {
        let source = "/* é */loop/* λ */{} loop1 _loop Loop \"loop\" lo/*x*/op";
        let tokens = Lexer::new(source).tokenize();
        let start = source.find("loop").unwrap();

        assert_eq!(tokens[0].span, Span::new(start, start + 4));
        assert_eq!(
            tokens
                .into_iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                TokenKind::Loop,
                TokenKind::LeftBrace,
                TokenKind::RightBrace,
                TokenKind::Identifier("loop1".into()),
                TokenKind::Identifier("_loop".into()),
                TokenKind::Identifier("Loop".into()),
                TokenKind::StringLiteral("loop".into()),
                TokenKind::Identifier("lo".into()),
                TokenKind::Identifier("op".into()),
                TokenKind::Eof,
            ]
        );
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
        for source in [".", ".5", "1.", "1.2.3", "1.2."] {
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
