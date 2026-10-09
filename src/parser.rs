use thiserror::Error;

use crate::{
    ast::{
        BinaryOp, Block, Expr, Function, Param, Program, Statement, StatementKind, Type, UnaryOp,
    },
    tokens::{Span, Token, TokenKind},
};

#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error("Unexpected token {found} (expected {expected}) at {span:?}")]
    UnexpectedToken {
        expected: &'static str,
        found: &'static str,
        span: Span,
    },

    #[error("Assignment target must be a variable or array element at {span:?}")]
    InvalidAssignmentTarget { span: Span },
}

impl ParseError {
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::UnexpectedToken { span, .. } | Self::InvalidAssignmentTarget { span } => {
                Some(*span)
            }
        }
    }
}

pub struct Parser<'tokens, 'source> {
    tokens: &'tokens [Token<'source>],
    current: usize,
}

impl<'tokens, 'source> Parser<'tokens, 'source> {
    pub fn new(tokens: &'tokens [Token<'source>]) -> Self {
        Self { tokens, current: 0 }
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut functions = vec![self.parse_function()?];

        while !self.at_end() {
            functions.push(self.parse_function()?);
        }

        if self.check(&TokenKind::Eof) {
            self.advance();

            if self.peek().is_some() {
                return Err(self.unexpected("end of input"));
            }
        }

        Ok(Program { functions })
    }

    fn parse_function(&mut self) -> Result<Function, ParseError> {
        let start = self.expect(TokenKind::Fn)?.span.start();
        let name = self.parse_identifier()?;

        self.expect(TokenKind::LeftParen)?;

        let mut params = Vec::new();

        if !self.check(&TokenKind::RightParen) {
            loop {
                params.push(self.parse_param()?);

                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }

        self.expect(TokenKind::RightParen)?;
        self.expect(TokenKind::Arrow)?;

        let return_type = self.parse_type()?;
        let body = self.parse_block()?;

        Ok(Function {
            name,
            params,
            return_type,
            body,
            span: self.span_since(start),
        })
    }

    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let start = self.current_span().start();
        let name = self.parse_identifier()?;

        self.expect(TokenKind::Colon)?;

        let ty = self.parse_type()?;

        Ok(Param {
            name,
            ty,
            span: self.span_since(start),
        })
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        let ty = match self.peek().map(|token| &token.kind) {
            Some(TokenKind::Number) => Type::Number,
            Some(TokenKind::Bool) => Type::Bool,
            Some(TokenKind::String) => Type::String,
            Some(TokenKind::Void) => Type::Void,
            _ => return Err(self.unexpected("type")),
        };

        self.advance();

        if ty == Type::Number && self.consume(TokenKind::LeftBracket) {
            self.expect(TokenKind::RightBracket)?;

            Ok(Type::NumberArray)
        } else {
            Ok(ty)
        }
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect(TokenKind::LeftBrace)?;

        let mut statements = Vec::new();

        while !self.check(&TokenKind::RightBrace) {
            if self.at_end() {
                return Err(self.unexpected("}"));
            }

            statements.push(self.parse_statement()?);
        }

        self.expect(TokenKind::RightBrace)?;

        Ok(statements)
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.current_span().start();

        let kind = match self.peek().map(|token| &token.kind) {
            Some(TokenKind::Let) => self.parse_let()?,
            Some(TokenKind::If) => self.parse_if()?,
            Some(TokenKind::While) => self.parse_while()?,
            Some(TokenKind::Return) => self.parse_return()?,
            _ => self.parse_expression_statement()?,
        };

        Ok(Statement {
            kind,
            span: self.span_since(start),
        })
    }

    fn parse_let(&mut self) -> Result<StatementKind, ParseError> {
        self.expect(TokenKind::Let)?;
        let name = self.parse_identifier()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        self.expect(TokenKind::Equal)?;
        let value = self.parse_expression()?;
        self.expect(TokenKind::Semicolon)?;

        Ok(StatementKind::Let { name, ty, value })
    }

    fn parse_if(&mut self) -> Result<StatementKind, ParseError> {
        self.expect(TokenKind::If)?;

        let cond = Box::new(self.parse_expression()?);
        let then = self.parse_block()?;

        let else_ = if self.consume(TokenKind::Else) {
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(StatementKind::If { cond, then, else_ })
    }

    fn parse_while(&mut self) -> Result<StatementKind, ParseError> {
        self.expect(TokenKind::While)?;

        let cond = Box::new(self.parse_expression()?);
        let body = self.parse_block()?;

        Ok(StatementKind::While { cond, body })
    }

    fn parse_return(&mut self) -> Result<StatementKind, ParseError> {
        self.expect(TokenKind::Return)?;

        let value = if self.check(&TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expression()?)
        };

        self.expect(TokenKind::Semicolon)?;

        Ok(StatementKind::Return(value))
    }

    fn parse_expression_statement(&mut self) -> Result<StatementKind, ParseError> {
        let start = self.current_span().start();

        let starts_with_identifier = matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::Identifier(_))
        );

        let expr = self.parse_expression()?;

        let kind = if self.check(&TokenKind::Equal) {
            if !starts_with_identifier || !matches!(expr, Expr::Identifier(_) | Expr::Index { .. })
            {
                return Err(ParseError::InvalidAssignmentTarget {
                    span: self.span_since(start),
                });
            }

            self.advance();

            let value = self.parse_expression()?;
            StatementKind::Assignment {
                target: expr,
                value,
            }
        } else {
            StatementKind::Expression(expr)
        };

        self.expect(TokenKind::Semicolon)?;

        Ok(kind)
    }

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(1)
    }

    fn parse_binary(&mut self, min_precedence: u8) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;

        while let Some((op, precedence)) = self.binary_operator() {
            if precedence < min_precedence {
                break;
            }

            self.advance();

            let right = self.parse_binary(precedence + 1)?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn binary_operator(&self) -> Option<(BinaryOp, u8)> {
        Some(match &self.peek()?.kind {
            TokenKind::OrOr => (BinaryOp::Or, 1),
            TokenKind::AndAnd => (BinaryOp::And, 2),

            TokenKind::EqualEqual => (BinaryOp::Equal, 3),
            TokenKind::BangEqual => (BinaryOp::NotEqual, 3),

            TokenKind::Less => (BinaryOp::LessThan, 4),
            TokenKind::LessEqual => (BinaryOp::LessThanOrEqual, 4),
            TokenKind::Greater => (BinaryOp::GreaterThan, 4),
            TokenKind::GreaterEqual => (BinaryOp::GreaterThanOrEqual, 4),

            TokenKind::Plus => (BinaryOp::Add, 5),
            TokenKind::Minus => (BinaryOp::Subtract, 5),

            TokenKind::Star => (BinaryOp::Multiply, 6),
            TokenKind::Slash => (BinaryOp::Divide, 6),
            TokenKind::Percent => (BinaryOp::Remainder, 6),

            _ => return None,
        })
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let op = match self.peek().map(|token| &token.kind) {
            Some(TokenKind::Bang) => UnaryOp::Not,
            Some(TokenKind::Minus) => UnaryOp::Negate,
            _ => return self.parse_primary(),
        };

        self.advance();

        Ok(Expr::Unary {
            op,
            expr: Box::new(self.parse_unary()?),
        })
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let expr = match self.peek().map(|token| &token.kind) {
            Some(TokenKind::NumberLiteral(value)) => Expr::Number(value.clone()),
            Some(TokenKind::StringLiteral(value)) => Expr::String(value.clone()),
            Some(TokenKind::True) => Expr::Bool(true),
            Some(TokenKind::False) => Expr::Bool(false),

            Some(TokenKind::Identifier(_)) => {
                let name = self.parse_identifier()?;

                return if self.consume(TokenKind::LeftParen) {
                    let args = self.parse_expression_list(TokenKind::RightParen)?;

                    Ok(Expr::Call { func: name, args })
                } else if self.consume(TokenKind::LeftBracket) {
                    let index = Box::new(self.parse_expression()?);

                    self.expect(TokenKind::RightBracket)?;

                    Ok(Expr::Index {
                        expr: Box::new(Expr::Identifier(name)),
                        index,
                    })
                } else {
                    Ok(Expr::Identifier(name))
                };
            }

            Some(TokenKind::LeftBracket) => {
                self.advance();

                return Ok(Expr::Array(
                    self.parse_expression_list(TokenKind::RightBracket)?,
                ));
            }

            Some(TokenKind::LeftParen) => {
                self.advance();

                let expr = self.parse_expression()?;

                self.expect(TokenKind::RightParen)?;

                return Ok(expr);
            }

            _ => return Err(self.unexpected("expression")),
        };

        self.advance();

        Ok(expr)
    }

    fn parse_expression_list(&mut self, end: TokenKind<'source>) -> Result<Vec<Expr>, ParseError> {
        let mut expressions = Vec::new();

        if !self.check(&end) {
            loop {
                expressions.push(self.parse_expression()?);

                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }

        self.expect(end)?;

        Ok(expressions)
    }

    fn parse_identifier(&mut self) -> Result<String, ParseError> {
        match self.peek().map(|token| &token.kind) {
            Some(TokenKind::Identifier(name)) => {
                let name = name.to_string();

                self.advance();

                Ok(name)
            }

            _ => Err(self.unexpected("identifier")),
        }
    }

    fn peek(&self) -> Option<&'tokens Token<'source>> {
        self.tokens.get(self.current)
    }

    fn advance(&mut self) -> Option<&'tokens Token<'source>> {
        let token = self.peek()?;

        self.current += 1;

        Some(token)
    }

    fn check(&self, kind: &TokenKind<'source>) -> bool {
        self.peek().is_some_and(|token| token.kind == *kind)
    }

    fn consume(&mut self, kind: TokenKind<'source>) -> bool {
        if self.check(&kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind<'source>) -> Result<&'tokens Token<'source>, ParseError> {
        if self.check(&kind) {
            Ok(self.advance().unwrap())
        } else {
            Err(self.unexpected(token_name(&kind)))
        }
    }

    fn at_end(&self) -> bool {
        self.peek().is_none() || self.check(&TokenKind::Eof)
    }

    fn current_span(&self) -> Span {
        self.peek().map_or_else(
            || {
                let end = self.tokens.last().map_or(0, |token| token.span.end());
                Span::new(end, end)
            },
            |token| token.span,
        )
    }

    fn span_since(&self, start: usize) -> Span {
        let end = self
            .current
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map_or(start, |token| token.span.end());

        Span::new(start, end)
    }

    fn unexpected(&self, expected: &'static str) -> ParseError {
        ParseError::UnexpectedToken {
            expected,
            found: self
                .peek()
                .map_or("end of input", |token| token_name(&token.kind)),
            span: self.current_span(),
        }
    }
}

fn token_name(kind: &TokenKind<'_>) -> &'static str {
    match kind {
        TokenKind::Fn => "fn",
        TokenKind::Let => "let",
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::While => "while",
        TokenKind::Return => "return",

        TokenKind::True => "true",
        TokenKind::False => "false",

        TokenKind::Number => "number",
        TokenKind::Bool => "bool",
        TokenKind::String => "string",
        TokenKind::Void => "void",

        TokenKind::Identifier(_) => "identifier",
        TokenKind::NumberLiteral(_) => "number literal",
        TokenKind::StringLiteral(_) => "string literal",

        TokenKind::LeftParen => "(",
        TokenKind::RightParen => ")",

        TokenKind::LeftBrace => "{",
        TokenKind::RightBrace => "}",

        TokenKind::LeftBracket => "[",
        TokenKind::RightBracket => "]",

        TokenKind::Comma => ",",
        TokenKind::Colon => ":",
        TokenKind::Semicolon => ";",

        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",

        TokenKind::Equal => "=",
        TokenKind::EqualEqual => "==",
        TokenKind::Bang => "!",
        TokenKind::BangEqual => "!=",

        TokenKind::Less => "<",
        TokenKind::LessEqual => "<=",
        TokenKind::Greater => ">",
        TokenKind::GreaterEqual => ">=",

        TokenKind::AndAnd => "&&",
        TokenKind::OrOr => "||",

        TokenKind::Arrow => "->",

        TokenKind::Eof => "end of input",
        TokenKind::Invalid => "invalid token",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(source: &str) -> Result<Program, ParseError> {
        let tokens = Lexer::new(source).tokenize();

        Parser::new(&tokens).parse_program()
    }

    fn expression(source: &str) -> Expr {
        let tokens = Lexer::new(source).tokenize();
        let mut parser = Parser::new(&tokens);

        let expr = parser.parse_expression().unwrap();

        assert!(
            parser.check(&TokenKind::Eof),
            "unconsumed expression tokens"
        );

        expr
    }

    fn number(value: &str) -> Expr {
        Expr::Number(value.into())
    }

    fn identifier(name: &str) -> Expr {
        Expr::Identifier(name.into())
    }

    fn binary(left: Expr, op: BinaryOp, right: Expr) -> Expr {
        Expr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    #[test]
    fn parses_functions_parameters_and_types() {
        let source = "fn compute(n: number, xs: number[], flag: bool, text: string) -> number { return n; }\nfn main() -> void {}";
        let program = parse(source).unwrap();

        assert_eq!(program.functions.len(), 2);

        let function = &program.functions[0];

        assert_eq!(function.name, "compute");
        assert_eq!(function.return_type, Type::Number);
        assert_eq!(
            function
                .params
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>(),
            ["n", "xs", "flag", "text"]
        );
        assert_eq!(
            function
                .params
                .iter()
                .map(|param| &param.ty)
                .collect::<Vec<_>>(),
            [
                &Type::Number,
                &Type::NumberArray,
                &Type::Bool,
                &Type::String
            ]
        );
        assert_eq!(
            function.params[1].span,
            Span::new(
                source.find("xs:").unwrap(),
                source.find("xs:").unwrap() + "xs: number[]".len()
            )
        );
        assert_eq!(function.span, Span::new(0, source.find('\n').unwrap()));
        assert!(program.functions[1].body.is_empty());
        assert_eq!(program.functions[1].return_type, Type::Void);
        assert_eq!(
            parse("fn values() -> number[] {}").unwrap().functions[0].return_type,
            Type::NumberArray
        );
    }

    #[test]
    fn parses_declarations_and_both_assignment_targets() {
        let source = "fn main() -> void { let xs: number[] = [1, 2]; xs[0] = 3; value = xs[1]; }";
        let program = parse(source).unwrap();
        let statements = &program.functions[0].body;

        assert_eq!(
            statements[0].kind,
            StatementKind::Let {
                name: "xs".into(),
                ty: Type::NumberArray,
                value: Expr::Array(vec![number("1"), number("2")]),
            }
        );
        assert_eq!(
            statements[1].kind,
            StatementKind::Assignment {
                target: Expr::Index {
                    expr: Box::new(identifier("xs")),
                    index: Box::new(number("0"))
                },
                value: number("3"),
            }
        );
        assert_eq!(
            statements[2].kind,
            StatementKind::Assignment {
                target: identifier("value"),
                value: Expr::Index {
                    expr: Box::new(identifier("xs")),
                    index: Box::new(number("1"))
                },
            }
        );

        let start = source.find("let").unwrap();

        assert_eq!(
            statements[0].span,
            Span::new(start, start + "let xs: number[] = [1, 2];".len())
        );
    }

    #[test]
    fn parses_nested_control_flow_and_return_forms() {
        let source = "fn main() -> void { if true { while false { return; } } else { if false { return 1; } } if false {} }";
        let program = parse(source).unwrap();
        let statements = &program.functions[0].body;

        let StatementKind::If {
            cond,
            then,
            else_: Some(otherwise),
        } = &statements[0].kind
        else {
            panic!("expected if/else");
        };

        assert_eq!(**cond, Expr::Bool(true));

        let StatementKind::While { cond, body } = &then[0].kind else {
            panic!("expected while");
        };

        assert_eq!(**cond, Expr::Bool(false));
        assert_eq!(body[0].kind, StatementKind::Return(None));

        let StatementKind::If {
            then, else_: None, ..
        } = &otherwise[0].kind
        else {
            panic!("expected nested if");
        };

        assert_eq!(then[0].kind, StatementKind::Return(Some(number("1"))));
        assert!(matches!(
            statements[1].kind,
            StatementKind::If { else_: None, .. }
        ));

        let start = source.find("if true").unwrap();
        let end = source.find(" if false {}").unwrap();

        assert_eq!(statements[0].span, Span::new(start, end));
    }

    #[test]
    fn parses_calls_array_access_and_literals() {
        assert_eq!(
            expression("call(1, other(), xs[2 + 3], [true, false], \"hello\")"),
            Expr::Call {
                func: "call".into(),
                args: vec![
                    number("1"),
                    Expr::Call {
                        func: "other".into(),
                        args: vec![]
                    },
                    Expr::Index {
                        expr: Box::new(identifier("xs")),
                        index: Box::new(binary(number("2"), BinaryOp::Add, number("3")))
                    },
                    Expr::Array(vec![Expr::Bool(true), Expr::Bool(false)]),
                    Expr::String("hello".into())
                ],
            }
        );
        assert_eq!(expression("[]"), Expr::Array(vec![]));
        assert_eq!(expression("12.34"), number("12.34"));
        assert_eq!(expression("\"\""), Expr::String(String::new()));
        assert_eq!(expression("name1"), identifier("name1"));

        let program = parse("fn main() -> void { call(); }").unwrap();

        assert_eq!(
            program.functions[0].body[0].kind,
            StatementKind::Expression(Expr::Call {
                func: "call".into(),
                args: vec![]
            })
        );
    }

    #[test]
    fn respects_every_binary_precedence_level() {
        assert_eq!(
            expression("a || b && c == d < e + f * g"),
            binary(
                identifier("a"),
                BinaryOp::Or,
                binary(
                    identifier("b"),
                    BinaryOp::And,
                    binary(
                        identifier("c"),
                        BinaryOp::Equal,
                        binary(
                            identifier("d"),
                            BinaryOp::LessThan,
                            binary(
                                identifier("e"),
                                BinaryOp::Add,
                                binary(identifier("f"), BinaryOp::Multiply, identifier("g"))
                            )
                        )
                    )
                )
            )
        );
    }

    #[test]
    fn binary_operators_are_left_associative() {
        assert_eq!(
            expression("10 - 3 - 2"),
            binary(
                binary(number("10"), BinaryOp::Subtract, number("3")),
                BinaryOp::Subtract,
                number("2")
            )
        );
        assert_eq!(
            expression("8 / 4 * 2 % 3"),
            binary(
                binary(
                    binary(number("8"), BinaryOp::Divide, number("4")),
                    BinaryOp::Multiply,
                    number("2")
                ),
                BinaryOp::Remainder,
                number("3")
            )
        );
    }

    #[test]
    fn parentheses_override_precedence_and_unary_binds_tightly() {
        assert_eq!(
            expression("-(1 + 2) * 3"),
            binary(
                Expr::Unary {
                    op: UnaryOp::Negate,
                    expr: Box::new(binary(number("1"), BinaryOp::Add, number("2"))),
                },
                BinaryOp::Multiply,
                number("3")
            )
        );
        assert_eq!(
            expression("!!false"),
            Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(Expr::Bool(false))
                }),
            }
        );
        assert_eq!(
            expression("-xs[0]"),
            Expr::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(Expr::Index {
                    expr: Box::new(identifier("xs")),
                    index: Box::new(number("0"))
                }),
            }
        );
    }

    #[test]
    fn supports_every_binary_operator() {
        for (symbol, op) in [
            ("+", BinaryOp::Add),
            ("-", BinaryOp::Subtract),
            ("*", BinaryOp::Multiply),
            ("/", BinaryOp::Divide),
            ("%", BinaryOp::Remainder),
            ("==", BinaryOp::Equal),
            ("!=", BinaryOp::NotEqual),
            ("<", BinaryOp::LessThan),
            ("<=", BinaryOp::LessThanOrEqual),
            (">", BinaryOp::GreaterThan),
            (">=", BinaryOp::GreaterThanOrEqual),
            ("&&", BinaryOp::And),
            ("||", BinaryOp::Or),
        ] {
            assert_eq!(
                expression(&format!("a {symbol} b")),
                binary(identifier("a"), op, identifier("b"))
            );
        }
    }

    #[test]
    fn reports_missing_semicolon_at_the_offending_token() {
        let source = "fn main() -> number { return 1 }";
        let position = source.find('}').unwrap();

        assert_eq!(
            parse(source).unwrap_err(),
            ParseError::UnexpectedToken {
                expected: ";",
                found: "}",
                span: Span::new(position, position + 1),
            }
        );
    }

    #[test]
    fn reports_missing_closing_brace_at_eof() {
        let source = "fn main() -> void {";

        assert_eq!(
            parse(source).unwrap_err(),
            ParseError::UnexpectedToken {
                expected: "}",
                found: "end of input",
                span: Span::new(source.len(), source.len()),
            }
        );
    }

    #[test]
    fn rejects_invalid_assignment_targets() {
        for target in ["1", "a + b", "call()", "[1]", "(a)", "-a", "!a"] {
            let source = format!("fn main() -> void {{ {target} = 1; }}");

            assert_eq!(
                parse(&source).unwrap_err(),
                ParseError::InvalidAssignmentTarget {
                    span: Span::new(
                        "fn main() -> void { ".len(),
                        "fn main() -> void { ".len() + target.len()
                    ),
                }
            );
        }
    }

    #[test]
    fn rejects_malformed_constructs() {
        for source in [
            "",
            "let x: number = 1;",
            "fn main() {}",
            "fn main(1: number) -> void {}",
            "fn main(x: unknown) -> void {}",
            "fn main(x: bool[]) -> void {}",
            "fn main() -> string[] {}",
            "fn main() -> void { let x = 1; }",
            "fn main() -> void { let x: number; }",
            "fn main() -> void { return +1; }",
            "fn main() -> void { if true {} else if false {} }",
            "fn main() -> void { while {} }",
            "fn main() -> void { call(1,); }",
            "fn main(x: number,) -> void {}",
            "fn main() -> void { [1,]; }",
            "fn main() -> void { xs[]; }",
            "fn main() -> void { (1 + 2; }",
            "fn main() -> void { call(1; }",
            "fn main() -> void { [1, 2; }",
            "fn main() -> void { xs[0; }",
            "fn main() -> void {} trailing",
            "fn main() -> void { a = b = 1; }",
            "fn main() -> void { @; }",
            "fn main() -> void { a[0][1]; }",
            "fn main() -> void { f()(1); }",
        ] {
            assert!(parse(source).is_err(), "unexpectedly accepted: {source}");
        }
    }

    #[test]
    fn handles_token_slices_without_eof_and_empty_slices() {
        let source = "fn main() -> void {}";
        let mut tokens = Lexer::new(source).tokenize();

        tokens.pop();

        assert!(Parser::new(&tokens).parse_program().is_ok());

        tokens.pop();

        assert_eq!(
            Parser::new(&tokens).parse_program().unwrap_err(),
            ParseError::UnexpectedToken {
                expected: "}",
                found: "end of input",
                span: Span::new(source.len() - 1, source.len() - 1),
            }
        );
        assert_eq!(
            Parser::new(&[]).parse_program().unwrap_err(),
            ParseError::UnexpectedToken {
                expected: "fn",
                found: "end of input",
                span: Span::new(0, 0),
            }
        );
    }

    #[test]
    fn rejects_tokens_after_eof() {
        let mut tokens = Lexer::new("fn main() -> void {}").tokenize();

        tokens.push(Token::new(TokenKind::Fn, Span::new(20, 22)));

        assert_eq!(
            Parser::new(&tokens).parse_program().unwrap_err(),
            ParseError::UnexpectedToken {
                expected: "end of input",
                found: "fn",
                span: Span::new(20, 22),
            }
        );
    }

    #[test]
    fn truncated_programs_never_panic() {
        let tokens = Lexer::new("fn compute(xs: number[], x: number) -> number { let y: number = xs[x] + call(1, 2); if y > 0 { return y; } else { return -y; } }").tokenize();

        for end in 0..tokens.len() - 1 {
            assert!(
                Parser::new(&tokens[..end]).parse_program().is_err(),
                "accepted truncated input at token {end}"
            );
        }
    }
}
