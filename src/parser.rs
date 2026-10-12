use thiserror::Error;

use crate::{
    ast::{
        BinaryOp, Block, CallArgument, DefaultExpression, Expr, Function, Param, Program,
        Statement, StatementKind, Type, UnaryOp, ValueBlock,
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
        let default = if self.consume(TokenKind::Equal) {
            let start = self.current_span().start();
            let value = self.parse_expression()?;

            Some(DefaultExpression {
                value,
                span: self.span_since(start),
            })
        } else {
            None
        };

        Ok(Param {
            name,
            ty,
            default,
            span: self.span_since(start),
        })
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        let ty = match self.peek().map(|token| &token.kind) {
            Some(TokenKind::Number) => Type::Number,
            Some(TokenKind::Bool) => Type::Bool,
            Some(TokenKind::String) => Type::String,
            Some(TokenKind::Range) => Type::Range,
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
            Some(TokenKind::For) => self.parse_for()?,
            Some(TokenKind::Loop) => {
                let body = self.parse_loop_body()?;

                self.consume(TokenKind::Semicolon);

                StatementKind::Loop { body }
            }
            Some(TokenKind::Break) => {
                self.advance();

                let kind = if self.check(&TokenKind::Semicolon) {
                    StatementKind::Break
                } else {
                    StatementKind::BreakValue(self.parse_expression()?)
                };

                self.expect(TokenKind::Semicolon)?;

                kind
            }
            Some(TokenKind::Continue) => {
                self.advance();
                self.expect(TokenKind::Semicolon)?;

                StatementKind::Continue
            }
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
        let ty = if self.consume(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(TokenKind::Equal)?;

        let value = self.parse_expression()?;

        self.expect(TokenKind::Semicolon)?;

        Ok(StatementKind::Let { name, ty, value })
    }

    fn parse_if(&mut self) -> Result<StatementKind, ParseError> {
        // A bare statement-if ends at its last arm; a following minus starts a
        // new statement. Parenthesize the whole if to continue it as an operand.
        let expr = self.parse_if_expression()?;

        if self.consume(TokenKind::Semicolon) {
            Ok(StatementKind::Expression(expr))
        } else {
            Ok(self.discard_if(expr))
        }
    }

    fn parse_if_expression(&mut self) -> Result<Expr, ParseError> {
        self.expect(TokenKind::If)?;

        let cond = Box::new(self.parse_expression()?);
        let then = self.parse_value_block()?;
        let else_ = if self.consume(TokenKind::Else) {
            if self.check(&TokenKind::If) {
                let start = self.current_span().start();
                let tail = self.parse_if_expression()?;

                Some(ValueBlock {
                    statements: Vec::new(),
                    tail: Some(Box::new(tail)),
                    span: self.span_since(start),
                })
            } else {
                Some(self.parse_value_block()?)
            }
        } else {
            None
        };

        Ok(Expr::If { cond, then, else_ })
    }

    fn discard_if(&self, expr: Expr) -> StatementKind {
        let Expr::If { cond, then, else_ } = expr else {
            unreachable!("discard_if is called only with an if expression");
        };

        StatementKind::If {
            cond,
            then: self.discard_block(then),
            else_: else_.map(|block| self.discard_block(block)),
        }
    }

    fn discard_block(&self, block: ValueBlock) -> Block {
        if block.tail.is_none() {
            return block.statements;
        }

        let tail_span = self.discarded_tail_span(&block);
        let first = self
            .tokens
            .partition_point(|token| token.span.start() < tail_span.start());
        let first_kind = self.tokens.get(first).map(|token| &token.kind);
        let mut statements = block.statements;

        if let Some(tail) = block.tail {
            let kind = match *tail {
                expr @ Expr::If { .. } if first_kind == Some(&TokenKind::If) => {
                    self.discard_if(expr)
                }
                Expr::Loop { body } if first_kind == Some(&TokenKind::Loop) => {
                    StatementKind::Loop { body }
                }
                expr => StatementKind::Expression(expr),
            };

            statements.push(Statement {
                kind,
                span: tail_span,
            });
        }

        statements
    }

    fn discarded_tail_span(&self, block: &ValueBlock) -> Span {
        // Raw expressions have no spans. Recover the discarded tail's token range
        // without changing ValueBlock's public shape or old statement-if spans.
        let opening = self
            .tokens
            .partition_point(|token| token.span.start() < block.span.start());
        let braced = self
            .tokens
            .get(opening)
            .is_some_and(|token| token.kind == TokenKind::LeftBrace);
        let first = match block.statements.last() {
            Some(statement) => self
                .tokens
                .partition_point(|token| token.span.start() < statement.span.end()),
            None => opening + usize::from(braced),
        };
        let start = self
            .tokens
            .get(first)
            .map_or(block.span.start(), |token| token.span.start());
        let end = if braced {
            let after = self
                .tokens
                .partition_point(|token| token.span.start() < block.span.end());

            self.tokens
                .get(after.saturating_sub(2))
                .map_or(start, |token| token.span.end())
        } else {
            // Synthetic else-if blocks cover the nested if rather than braces.
            block.span.end()
        };

        Span::new(start, end)
    }

    fn parse_value_block(&mut self) -> Result<ValueBlock, ParseError> {
        let start = self.expect(TokenKind::LeftBrace)?.span.start();
        let mut statements = Vec::new();
        let mut tail = None;

        while !self.check(&TokenKind::RightBrace) {
            if self.at_end() {
                return Err(self.unexpected("}"));
            }

            if matches!(
                self.peek().map(|token| &token.kind),
                Some(
                    TokenKind::Let
                        | TokenKind::While
                        | TokenKind::For
                        | TokenKind::Break
                        | TokenKind::Continue
                        | TokenKind::Return
                )
            ) {
                statements.push(self.parse_statement()?);
                continue;
            }

            let statement_start = self.current_span().start();
            let starts_with_if = self.check(&TokenKind::If);
            let starts_with_loop = self.check(&TokenKind::Loop);
            let starts_with_identifier = matches!(
                self.peek().map(|token| &token.kind),
                Some(TokenKind::Identifier(_))
            );
            let expr = if starts_with_if {
                self.parse_if_expression()?
            } else if starts_with_loop {
                Expr::Loop {
                    body: self.parse_loop_body()?,
                }
            } else {
                self.parse_expression()?
            };

            if self.check(&TokenKind::Equal) || self.compound_assignment_operator().is_some() {
                let kind = self.finish_expression_statement(
                    expr,
                    starts_with_identifier,
                    statement_start,
                )?;

                statements.push(Statement {
                    kind,
                    span: self.span_since(statement_start),
                });
            } else if self.consume(TokenKind::Semicolon) {
                statements.push(Statement {
                    kind: if starts_with_loop {
                        let Expr::Loop { body } = expr else {
                            unreachable!("a bare loop was parsed above");
                        };

                        StatementKind::Loop { body }
                    } else {
                        StatementKind::Expression(expr)
                    },
                    span: self.span_since(statement_start),
                });
            } else if self.check(&TokenKind::RightBrace) {
                tail = Some(Box::new(expr));
                break;
            } else if starts_with_if {
                statements.push(Statement {
                    kind: self.discard_if(expr),
                    span: self.span_since(statement_start),
                });
            } else if starts_with_loop {
                let Expr::Loop { body } = expr else {
                    unreachable!("a bare loop was parsed above");
                };

                statements.push(Statement {
                    kind: StatementKind::Loop { body },
                    span: self.span_since(statement_start),
                });
            } else {
                return Err(self.unexpected(";"));
            }
        }

        self.expect(TokenKind::RightBrace)?;

        Ok(ValueBlock {
            statements,
            tail,
            span: self.span_since(start),
        })
    }

    fn parse_loop_body(&mut self) -> Result<Block, ParseError> {
        self.expect(TokenKind::Loop)?;

        self.parse_block()
    }

    fn parse_while(&mut self) -> Result<StatementKind, ParseError> {
        self.expect(TokenKind::While)?;

        let cond = Box::new(self.parse_expression()?);
        let body = self.parse_block()?;

        Ok(StatementKind::While { cond, body })
    }

    fn parse_for(&mut self) -> Result<StatementKind, ParseError> {
        self.expect(TokenKind::For)?;

        let name = self.parse_identifier()?;

        self.expect(TokenKind::In)?;

        let iterable = self.parse_expression()?;
        let body = self.parse_block()?;

        Ok(StatementKind::For {
            name,
            iterable,
            body,
        })
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

        self.finish_expression_statement(expr, starts_with_identifier, start)
    }

    fn finish_expression_statement(
        &mut self,
        expr: Expr,
        starts_with_identifier: bool,
        start: usize,
    ) -> Result<StatementKind, ParseError> {
        let compound_op = self.compound_assignment_operator();

        let kind = if self.check(&TokenKind::Equal) || compound_op.is_some() {
            if !starts_with_identifier || !matches!(expr, Expr::Identifier(_) | Expr::Index { .. })
            {
                return Err(ParseError::InvalidAssignmentTarget {
                    span: self.span_since(start),
                });
            }

            self.advance();

            let value = self.parse_expression()?;

            match compound_op {
                Some(op) => StatementKind::CompoundAssignment {
                    target: expr,
                    op,
                    value,
                },
                None => StatementKind::Assignment {
                    target: expr,
                    value,
                },
            }
        } else {
            StatementKind::Expression(expr)
        };

        self.expect(TokenKind::Semicolon)?;

        Ok(kind)
    }

    fn compound_assignment_operator(&self) -> Option<BinaryOp> {
        Some(match &self.peek()?.kind {
            TokenKind::PlusEqual => BinaryOp::Add,
            TokenKind::MinusEqual => BinaryOp::Subtract,
            TokenKind::StarEqual => BinaryOp::Multiply,
            TokenKind::SlashEqual => BinaryOp::Divide,
            TokenKind::PercentEqual => BinaryOp::Remainder,
            _ => return None,
        })
    }

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(1)
    }

    fn parse_binary(&mut self, min_precedence: u8) -> Result<Expr, ParseError> {
        let mut left = if min_precedence <= 5 {
            self.parse_range()?
        } else {
            self.parse_unary()?
        };

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

    fn parse_range(&mut self) -> Result<Expr, ParseError> {
        let start = self.parse_binary(6)?;
        let inclusive = match self.peek().map(|token| &token.kind) {
            Some(TokenKind::DotDot) => false,
            Some(TokenKind::DotDotEqual) => true,
            _ => return Ok(start),
        };

        self.advance();

        let end = self.parse_binary(6)?;
        let step = if matches!(self.peek().map(|token| &token.kind),
            Some(TokenKind::Identifier(name)) if name == "by")
        {
            self.advance();

            Some(Box::new(self.parse_binary(6)?))
        } else {
            None
        };

        if self.check(&TokenKind::DotDot) || self.check(&TokenKind::DotDotEqual) {
            return Err(self.unexpected("non-chained range"));
        }

        Ok(Expr::Range {
            start: Box::new(start),
            end: Box::new(end),
            step,
            inclusive,
        })
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
            TokenKind::In => (BinaryOp::In, 4),

            TokenKind::Plus => (BinaryOp::Add, 6),
            TokenKind::Minus => (BinaryOp::Subtract, 6),

            TokenKind::Star => (BinaryOp::Multiply, 7),
            TokenKind::Slash => (BinaryOp::Divide, 7),
            TokenKind::Percent => (BinaryOp::Remainder, 7),

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
                    let args = self.parse_call_arguments()?;

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

            Some(TokenKind::LeftBrace) => return Ok(Expr::Block(self.parse_value_block()?)),
            Some(TokenKind::If) => return self.parse_if_expression(),
            Some(TokenKind::Loop) => {
                return Ok(Expr::Loop {
                    body: self.parse_loop_body()?,
                });
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

    fn parse_call_arguments(&mut self) -> Result<Vec<CallArgument>, ParseError> {
        let mut args = Vec::new();

        if !self.check(&TokenKind::RightParen) {
            loop {
                let start = self.current_span().start();
                let named = matches!(
                    self.peek().map(|token| &token.kind),
                    Some(TokenKind::Identifier(_))
                ) && self
                    .tokens
                    .get(self.current + 1)
                    .is_some_and(|token| token.kind == TokenKind::Colon);
                let name = if named {
                    let name = self.parse_identifier()?;
                    self.expect(TokenKind::Colon)?;
                    Some(name)
                } else {
                    None
                };
                let value = self.parse_expression()?;

                args.push(CallArgument {
                    name,
                    value,
                    span: self.span_since(start),
                });

                if !self.consume(TokenKind::Comma) {
                    break;
                }
            }
        }

        self.expect(TokenKind::RightParen)?;
        Ok(args)
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
        TokenKind::For => "for",
        TokenKind::Loop => "loop",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::Return => "return",
        TokenKind::In => "in",

        TokenKind::True => "true",
        TokenKind::False => "false",

        TokenKind::Number => "number",
        TokenKind::Bool => "bool",
        TokenKind::String => "string",
        TokenKind::Range => "range",
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

        TokenKind::PlusEqual => "+=",
        TokenKind::MinusEqual => "-=",
        TokenKind::StarEqual => "*=",
        TokenKind::SlashEqual => "/=",
        TokenKind::PercentEqual => "%=",

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

        TokenKind::DotDot => "..",
        TokenKind::DotDotEqual => "..=",
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

    fn negate(expr: Expr) -> Expr {
        Expr::Unary {
            op: UnaryOp::Negate,
            expr: Box::new(expr),
        }
    }

    fn assert_expression_statement(statement: &Statement, expected: &Expr) {
        let StatementKind::Expression(actual) = &statement.kind else {
            panic!("expected expression statement: {statement:?}");
        };

        assert_eq!(actual, expected);
    }

    fn assert_if_arithmetic(expr: &Expr, expected_op: BinaryOp, expected_right: &Expr) {
        let Expr::Binary { left, op, right } = expr else {
            panic!("expected whole-if arithmetic: {expr:?}");
        };
        let Expr::If {
            cond,
            then,
            else_: Some(_),
        } = &**left
        else {
            panic!("expected if/else as the left operand: {left:?}");
        };

        assert_eq!(**cond, Expr::Bool(true));
        assert_eq!(then.tail.as_deref(), Some(&number("1")));
        assert_eq!(*op, expected_op);
        assert_eq!(&**right, expected_right);
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
                ty: Some(Type::NumberArray),
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
    fn parses_all_compound_assignments_with_both_targets_and_original_spans() {
        for (operator, op) in [
            ("+=", BinaryOp::Add),
            ("-=", BinaryOp::Subtract),
            ("*=", BinaryOp::Multiply),
            ("/=", BinaryOp::Divide),
            ("%=", BinaryOp::Remainder),
        ] {
            for (target_text, target) in [
                ("value", identifier("value")),
                (
                    "xs[index()]",
                    Expr::Index {
                        expr: Box::new(identifier("xs")),
                        index: Box::new(Expr::Call {
                            func: "index".into(),
                            args: vec![],
                        }),
                    },
                ),
            ] {
                let statement = format!("{target_text}/*é*/{operator}/*λ*/2 + 3 * 4;");
                let source = format!("fn main() -> void {{ /*🙂*/{statement} }}");
                let program = parse(&source).unwrap();
                let body = &program.functions[0].body;
                let start = source.find(target_text).unwrap();

                assert_eq!(body.len(), 1);
                assert_eq!(body[0].span, Span::new(start, start + statement.len()));
                assert_eq!(
                    body[0].kind,
                    StatementKind::CompoundAssignment {
                        target,
                        op,
                        value: binary(
                            number("2"),
                            BinaryOp::Add,
                            binary(number("3"), BinaryOp::Multiply, number("4")),
                        ),
                    }
                );
            }
        }
    }

    #[test]
    fn compound_assignments_reuse_invalid_assignment_target_errors() {
        for operator in ["+=", "-=", "*=", "/=", "%="] {
            for target in ["1", "a + b", "call()", "[1]", "(a)", "(xs[0])", "-a", "!a"] {
                let source = format!("fn main() -> void {{ {target} {operator} missing; }}");
                let start = "fn main() -> void { ".len();

                assert_eq!(
                    parse(&source).unwrap_err(),
                    ParseError::InvalidAssignmentTarget {
                        span: Span::new(start, start + target.len()),
                    }
                );
            }
        }
    }

    #[test]
    fn compound_assignments_report_missing_values_and_semicolons() {
        for operator in ["+=", "-=", "*=", "/=", "%="] {
            for (tail, expected, found) in [(";", "expression", ";"), ("1 }", ";", "}")] {
                let source = format!("fn main() -> void {{ value {operator} {tail}");
                let position = source.rfind(found).unwrap();

                assert_eq!(
                    parse(&source).unwrap_err(),
                    ParseError::UnexpectedToken {
                        expected,
                        found,
                        span: Span::new(position, position + 1),
                    }
                );
            }
        }
    }

    #[test]
    fn rejects_compound_assignments_in_expressions_and_malformed_places() {
        for operator in ["+=", "-=", "*=", "/=", "%="] {
            for body in [
                format!("let value = x {operator} 1;"),
                format!("let value {operator} 1;"),
                format!("return x {operator} 1;"),
                format!("call(x {operator} 1);"),
                format!("[x {operator} 1];"),
                format!("xs[x {operator} 1];"),
                format!("if x {operator} 1 {{}}"),
                format!("while x {operator} 1 {{}}"),
                format!("x {operator} y {operator} 1;"),
                format!("x = y {operator} 1;"),
                format!("x {operator} y = 1;"),
                format!("(x {operator} 1);"),
                format!("xs[] {operator} 1;"),
                format!("xs[0][1] {operator} 1;"),
                format!("make()[0] {operator} 1;"),
                format!("[1][0] {operator} 1;"),
                format!("x {operator}= 1;"),
                format!("x {}/*é*/= 1;", &operator[..1]),
            ] {
                let source = format!("fn main() -> void {{ {body} }}");

                assert!(parse(&source).is_err(), "unexpectedly accepted: {source}");
            }
        }
    }

    #[test]
    fn truncated_compound_assignments_never_panic() {
        for operator in ["+=", "-=", "*=", "/=", "%="] {
            let source = format!("fn test() -> void {{ xs[index()] {operator} value() + 1; }}");
            let tokens = Lexer::new(&source).tokenize();

            for end in 0..tokens.len() - 1 {
                assert!(
                    Parser::new(&tokens[..end]).parse_program().is_err(),
                    "accepted truncated {operator} input at token {end}"
                );
            }
        }
    }

    #[test]
    fn parses_mixed_inferred_and_annotated_declarations_with_original_spans() {
        let source = "fn main() -> void { let/* note */n = 1; let flag: bool = true; let text = \"é\"; let values = []; }";
        let program = parse(source).unwrap();
        let statements = &program.functions[0].body;
        let expected = [
            (
                "let/* note */n = 1;",
                StatementKind::Let {
                    name: "n".into(),
                    ty: None,
                    value: number("1"),
                },
            ),
            (
                "let flag: bool = true;",
                StatementKind::Let {
                    name: "flag".into(),
                    ty: Some(Type::Bool),
                    value: Expr::Bool(true),
                },
            ),
            (
                "let text = \"é\";",
                StatementKind::Let {
                    name: "text".into(),
                    ty: None,
                    value: Expr::String("é".into()),
                },
            ),
            (
                "let values = [];",
                StatementKind::Let {
                    name: "values".into(),
                    ty: None,
                    value: Expr::Array(vec![]),
                },
            ),
        ];

        assert_eq!(statements.len(), expected.len());

        for (statement, (fragment, kind)) in statements.iter().zip(expected) {
            let start = source.find(fragment).unwrap();

            assert_eq!(statement.kind, kind);
            assert_eq!(statement.span, Span::new(start, start + fragment.len()));
        }
    }

    #[test]
    fn incomplete_local_declarations_report_the_offending_token() {
        for (statement, expected, found) in [
            ("let value;", "=", ";"),
            ("let value: number;", "=", ";"),
            ("let value = ;", "expression", ";"),
            ("let value: = 1;", "type", "="),
            ("let value = 1", ";", "}"),
        ] {
            let source = format!("fn main() -> void {{ {statement} }}");
            let start = source.find(found).unwrap();

            assert_eq!(
                parse(&source).unwrap_err(),
                ParseError::UnexpectedToken {
                    expected,
                    found,
                    span: Span::new(start, start + found.len()),
                }
            );
        }
    }

    #[test]
    fn parses_bare_loop_transfers_with_original_spans() {
        let source = "fn main() -> void { /* é */ while true { if false { break /* λ */; } else { continue; } } break; continue; }";
        let program = parse(source).unwrap();
        let statements = &program.functions[0].body;
        let StatementKind::While { body, .. } = &statements[0].kind else {
            panic!("expected while");
        };
        let StatementKind::If {
            then,
            else_: Some(otherwise),
            ..
        } = &body[0].kind
        else {
            panic!("expected if/else");
        };

        for (statement, text, kind) in [
            (&then[0], "break /* λ */;", StatementKind::Break),
            (&otherwise[0], "continue;", StatementKind::Continue),
            (&statements[1], "break;", StatementKind::Break),
        ] {
            let start = source.find(text).unwrap();

            assert_eq!(statement.kind, kind);
            assert_eq!(statement.span, Span::new(start, start + text.len()));
        }

        let start = source.rfind("continue;").unwrap();

        assert_eq!(statements[2].kind, StatementKind::Continue);
        assert_eq!(
            statements[2].span,
            Span::new(start, start + "continue;".len())
        );
    }

    #[test]
    fn rejects_continue_values_labels_and_missing_transfer_semicolons() {
        for (statement, found, text) in [
            ("continue 1;", "number literal", "1"),
            ("continue label;", "identifier", "label"),
            ("break 1", "}", "}"),
            ("continue", "}", "}"),
        ] {
            let source = format!("fn main() -> void {{ while true {{ {statement} }} }}");
            let statement_start = source.find(statement).unwrap();
            let start = statement_start + source[statement_start..].find(text).unwrap();
            let width = text.len();

            assert_eq!(
                parse(&source),
                Err(ParseError::UnexpectedToken {
                    expected: ";",
                    found,
                    span: Span::new(start, start + width),
                }),
                "{source}"
            );
        }
    }

    #[test]
    fn loop_transfer_keywords_are_not_expressions_or_names() {
        for source in [
            "fn break() -> void {}",
            "fn continue() -> void {}",
            "fn loop() -> void {}",
            "fn main(loop: number) -> void {}",
            "fn main() -> void { let loop = 1; }",
            "fn main() -> void { break }",
            "fn main(break: number) -> void {}",
            "fn main() -> void { let continue = 1; }",
            "fn main() -> number { return break; }",
            "fn main() -> void { let x = continue; }",
            "fn main() -> void { while break {} }",
            "fn main() -> void { use_value(continue); }",
            "fn main() -> void { while true { label: break; } }",
            "fn main() -> void { while true { break: label; } }",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }

        for keyword in ["break", "continue"] {
            let source = format!("fn main() -> void {{ while true {{ {keyword}; }} }}");

            for end in 0..source.len() {
                let _ = parse(&source[..end]);
            }
        }
    }

    #[test]
    fn parses_break_values_with_expressions_and_original_byte_spans() {
        for value in [
            "1",
            "true",
            "label",
            "(1)",
            "[1, 2]",
            "if true { 1 } else { 2 }",
            "loop { break 1; }",
        ] {
            let text = format!("break /* λ */ {value};");
            let source = format!("fn test() -> void {{ /* é */ {text} }}");
            let program = parse(&source).unwrap();
            let statement = &program.functions[0].body[0];
            let start = source.find(&text).unwrap();

            let tokens = Lexer::new(&source).tokenize();
            let value_start = start + "break /* λ */ ".len();
            let first = tokens.partition_point(|token| token.span.start() < value_start);
            let expected = Parser::new(&tokens[first..]).parse_expression().unwrap();

            assert_eq!(statement.kind, StatementKind::BreakValue(expected));
            assert_eq!(statement.span, Span::new(start, start + text.len()));
        }
    }

    #[test]
    fn bare_loop_statements_end_at_the_brace_and_allow_an_optional_semicolon() {
        for separator in ["", " ", "/* é */", "\n"] {
            for suffix in ["", ";"] {
                let head = format!("loop {{ break; }}{suffix}");
                let source =
                    format!("fn test() -> void {{ {head}{separator}--1; (loop {{ break 2; }}); }}");
                let program = parse(&source).unwrap();
                let body = &program.functions[0].body;
                let start = source.find("loop").unwrap();

                assert_eq!(body.len(), 3);
                assert!(matches!(body[0].kind, StatementKind::Loop { .. }));
                assert_eq!(body[0].span, Span::new(start, start + head.len()));
                assert_expression_statement(&body[1], &negate(negate(number("1"))));
                assert!(matches!(
                    body[2].kind,
                    StatementKind::Expression(Expr::Loop { .. })
                ));
            }
        }
    }

    #[test]
    fn value_blocks_classify_final_nonfinal_and_parenthesized_loops_without_newlines() {
        let Expr::Block(block) =
            expression("{ loop { break; }-1; loop { break; }; loop { break true; } }")
        else {
            panic!("expected block")
        };

        assert_eq!(block.statements.len(), 3);
        assert!(matches!(
            block.statements[0].kind,
            StatementKind::Loop { .. }
        ));
        assert_expression_statement(&block.statements[1], &negate(number("1")));
        assert!(matches!(
            block.statements[2].kind,
            StatementKind::Loop { .. }
        ));
        assert!(matches!(block.tail.as_deref(), Some(Expr::Loop { .. })));

        let Expr::Block(block) = expression("{ (loop { break 1; }) - 1 }") else {
            panic!("expected block")
        };

        assert!(matches!(
            block.tail.as_deref(),
            Some(Expr::Binary {
                op: BinaryOp::Subtract,
                ..
            })
        ));
        assert!(parse("fn test() -> void { loop {} + 1; }").is_err());
        assert!(parse("fn test() -> void { let x = { loop {} + 1 }; }").is_err());

        let program =
            parse("fn test() -> void { if true { loop { break; } } else { (loop { break 1; }) } }")
                .unwrap();
        let StatementKind::If {
            then,
            else_: Some(else_),
            ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected if")
        };

        assert!(matches!(then[0].kind, StatementKind::Loop { .. }));
        assert!(matches!(
            else_[0].kind,
            StatementKind::Expression(Expr::Loop { .. })
        ));
    }

    #[test]
    fn loop_expressions_accept_ordinary_operand_continuation_and_strict_bodies() {
        for value in [
            "loop { break 1; } - 2 * 3",
            "-loop { break 1; }",
            "take(loop { break [1]; })",
        ] {
            let _ = expression(value);
        }

        for source in [
            "fn test() -> void { loop }",
            "fn test() -> void { loop true {} }",
            "fn test() -> void { loop { 1 } }",
            "fn test() -> void { let x = loop { 1 }; }",
            "fn test() -> void { let x = loop { break 1 }; }",
            "fn test() -> void { loop { break: label; } }",
            "fn test() -> void { loop { continue 1; } }",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }

        let source = "fn test() -> number { return loop { break if true { loop { break 1; } } else { 2 }; }; }";

        for end in 0..source.len() {
            let _ = parse(&source[..end]);
        }
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
                    (number("1"), Span::new(5, 6)),
                    (
                        Expr::Call {
                            func: "other".into(),
                            args: vec![]
                        },
                        Span::new(8, 15)
                    ),
                    (
                        Expr::Index {
                            expr: Box::new(identifier("xs")),
                            index: Box::new(binary(number("2"), BinaryOp::Add, number("3")))
                        },
                        Span::new(17, 26)
                    ),
                    (
                        Expr::Array(vec![Expr::Bool(true), Expr::Bool(false)]),
                        Span::new(28, 41)
                    ),
                    (Expr::String("hello".into()), Span::new(43, 50))
                ]
                .into_iter()
                .map(|(value, span)| CallArgument {
                    name: None,
                    value,
                    span
                })
                .collect(),
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
            "fn main(x) -> void {}",
            "fn main() -> void { let x; }",
            "fn main() -> void { let x = ; }",
            "fn main() -> void { let x: = 1; }",
            "fn main() -> void { let x = 1 }",
            "fn main() -> void { let x number = 1; }",
            "fn main() -> void { let x: number; }",
            "fn main() -> void { return +1; }",
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
    fn value_blocks_preserve_tails_statements_and_utf8_byte_spans() {
        let source = "fn test() -> void { let x = { let text = \"é\"; { 2 } }; }";
        let program = parse(source).unwrap();
        let StatementKind::Let {
            value: Expr::Block(block),
            ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected a block initializer");
        };
        let start = source.find("{ let text").unwrap();
        let end = source.find(" }; }").unwrap() + 2;

        assert_eq!(block.span, Span::new(start, end));
        assert_eq!(block.statements.len(), 1);
        let Some(tail) = &block.tail else {
            panic!("expected tail")
        };
        let Expr::Block(nested) = &**tail else {
            panic!("expected nested block")
        };
        let start = source.find("{ 2 }").unwrap();

        assert_eq!(nested.span, Span::new(start, start + 5));
        assert_eq!(nested.tail.as_deref(), Some(&number("2")));
        assert!(matches!(
            expression("{ 2; }"),
            Expr::Block(ValueBlock { tail: None, .. })
        ));
        assert!(matches!(
            expression("{}"),
            Expr::Block(ValueBlock { tail: None, .. })
        ));
    }

    #[test]
    fn final_if_is_a_tail_but_nonfinal_if_is_a_statement() {
        let Expr::Block(block) = expression("{ if true { 1 } else { 2 } }") else {
            panic!("expected block")
        };

        assert!(block.statements.is_empty());
        assert!(matches!(
            block.tail.as_deref(),
            Some(Expr::If { else_: Some(_), .. })
        ));

        let Expr::Block(block) = expression("{ if true { 1 } 2 }") else {
            panic!("expected block")
        };

        assert!(matches!(
            block.statements[0].kind,
            StatementKind::If { else_: None, .. }
        ));
        assert_eq!(block.tail.as_deref(), Some(&number("2")));
        let Expr::Block(block) = expression("{ if true {} }") else {
            panic!("expected block")
        };

        assert!(matches!(
            block.tail.as_deref(),
            Some(Expr::If { else_: None, .. })
        ));
    }

    #[test]
    fn statement_if_discards_tails_and_else_if_retains_statement_shapes() {
        let program = parse("fn test() -> void { if true { 1 } else if false { 2 } }").unwrap();
        let StatementKind::If {
            then,
            else_: Some(else_),
            ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected statement if")
        };

        assert_eq!(then.len(), 1);
        assert!(matches!(
            then[0].kind,
            StatementKind::Expression(Expr::Number(_))
        ));
        assert!(matches!(
            else_[0].kind,
            StatementKind::If { else_: None, .. }
        ));

        let Expr::If {
            else_: Some(else_), ..
        } = expression("if true { 1 } else if false { 2 } else { 3 }")
        else {
            panic!("expected expression if")
        };

        assert!(else_.statements.is_empty());
        assert!(matches!(
            else_.tail.as_deref(),
            Some(Expr::If { else_: Some(_), .. })
        ));
        assert!(matches!(
            parse("fn test() -> void { if true {} else {}; }")
                .unwrap()
                .functions[0]
                .body[0]
                .kind,
            StatementKind::Expression(Expr::If { .. })
        ));
    }

    #[test]
    fn discarded_branch_tails_keep_token_spans_and_parenthesized_if_context() {
        let source = "fn test() -> void { if true { if false { return; } } else { let x = 1; /* comment */ x + 2 } }";
        let program = parse(source).unwrap();
        let StatementKind::If {
            then,
            else_: Some(else_),
            ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected if")
        };
        let start = source.find("if false").unwrap();

        assert_eq!(
            then[0].span,
            Span::new(start, start + "if false { return; }".len())
        );
        let start = source.find("x + 2").unwrap();

        assert_eq!(else_[1].span, Span::new(start, start + 5));
        let program = parse("fn test() -> void { if true { (if false {}) } }").unwrap();
        let StatementKind::If { then, .. } = &program.functions[0].body[0].kind else {
            panic!("expected if")
        };

        assert!(matches!(
            then[0].kind,
            StatementKind::Expression(Expr::If { else_: None, .. })
        ));
    }

    #[test]
    fn value_block_adjacency_requires_semicolons_without_newline_heuristics() {
        for body in [
            "{ 1 2 }",
            "{ 1\n2 }",
            "{ {} {} }",
            "{ {}\n1 }",
            "{ call() call() }",
            "{ (if true {} else {}) 1 }",
            "{ (if true {} else {})\n1 }",
        ] {
            let source = format!("fn test() -> void {{ let x = {body}; }}");

            assert!(
                matches!(
                    parse(&source),
                    Err(ParseError::UnexpectedToken { expected: ";", .. })
                ),
                "{source}"
            );
        }

        for source in [
            "fn test() -> number { 1 }",
            "fn test() -> void { {} }",
            "fn test() -> void { while true { 1 } }",
            "fn test() -> void { while true { {} } }",
        ] {
            assert!(
                matches!(
                    parse(source),
                    Err(ParseError::UnexpectedToken {
                        expected: ";",
                        found: "}",
                        ..
                    })
                ),
                "{source}"
            );
        }

        assert!(parse("fn test() -> void { { 1 }; while true { { 2 }; } }").is_ok());
        assert!(parse("fn test() -> void { (if true { 1 } else { 2 }) + 3; }").is_ok());
    }

    #[test]
    fn bare_if_preserves_following_unary_statements_and_byte_spans() {
        for else_ in [
            "",
            " else {}",
            " else if false {}",
            " else if false {} else {}",
        ] {
            let if_source = format!("if true {{}}{else_}");

            for (unary_source, expected) in [
                ("-1;", negate(number("1"))),
                ("--1;", negate(negate(number("1")))),
                ("- /* é */ -1;", negate(negate(number("1")))),
            ] {
                for trivia in ["", " ", "\n", "\r\n", "/* é */", "// é\r\n"] {
                    for (opening, closing) in [
                        ("", ""),
                        ("while true { ", " }"),
                        ("if false { ", " }"),
                        ("if false {} else { ", " }"),
                    ] {
                        let source = format!(
                            "fn main() -> void {{ /* π */ {opening}{if_source}{trivia}{unary_source}{closing} }}"
                        );
                        let program = parse(&source).unwrap();
                        let body = &program.functions[0].body;
                        let statements = match &body[0].kind {
                            StatementKind::While { body, .. } => body,
                            StatementKind::If { then, .. } if !opening.is_empty() => {
                                if let StatementKind::If {
                                    else_: Some(else_), ..
                                } = &body[0].kind
                                {
                                    else_
                                } else {
                                    then
                                }
                            }
                            _ => body,
                        };

                        assert_eq!(statements.len(), 2, "{source}");
                        let StatementKind::If {
                            cond,
                            then,
                            else_: parsed_else,
                        } = &statements[0].kind
                        else {
                            panic!("expected bare statement-if: {source}");
                        };

                        assert_eq!(**cond, Expr::Bool(true), "{source}");
                        assert!(then.is_empty(), "{source}");
                        assert_eq!(parsed_else.is_some(), !else_.is_empty(), "{source}");
                        assert_expression_statement(&statements[1], &expected);
                        let if_start = source.find(&if_source).unwrap();
                        let unary_start = source.find(unary_source).unwrap();

                        assert_eq!(
                            statements[0].span,
                            Span::new(if_start, if_start + if_source.len()),
                            "{source}"
                        );
                        assert_eq!(
                            statements[1].span,
                            Span::new(unary_start, unary_start + unary_source.len()),
                            "{source}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn value_block_bare_if_stops_before_unary_statements_and_tails() {
        for else_ in [
            "",
            " else {}",
            " else if false {}",
            " else if false {} else {}",
        ] {
            let if_source = format!("if true {{}}{else_}");

            for (unary_source, expected) in [
                ("-1", negate(number("1"))),
                ("--1", negate(negate(number("1")))),
                ("- /* é */ -1", negate(negate(number("1")))),
            ] {
                for trivia in ["", " ", "\n", "\r\n", "/* é */", "// é\r\n"] {
                    for semicolon in ["", ";"] {
                        let source = format!(
                            "{{ /* π */ {if_source}{trivia}{unary_source}{semicolon} /* end */ }}"
                        );
                        let Expr::Block(block) = expression(&source) else {
                            panic!("expected value block");
                        };
                        let if_start = source.find(&if_source).unwrap();

                        assert_eq!(block.span, Span::new(0, source.len()), "{source}");
                        assert_eq!(
                            block.statements[0].span,
                            Span::new(if_start, if_start + if_source.len()),
                            "{source}"
                        );
                        assert!(
                            matches!(block.statements[0].kind, StatementKind::If { .. }),
                            "{source}"
                        );

                        if semicolon.is_empty() {
                            assert_eq!(block.statements.len(), 1, "{source}");
                            assert_eq!(block.tail.as_deref(), Some(&expected), "{source}");
                        } else {
                            assert_eq!(block.statements.len(), 2, "{source}");
                            assert!(block.tail.is_none(), "{source}");
                            assert_expression_statement(&block.statements[1], &expected);
                            let unary_start = source.find(unary_source).unwrap();

                            assert_eq!(
                                block.statements[1].span,
                                Span::new(unary_start, unary_start + unary_source.len() + 1),
                                "{source}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn discarded_nested_unary_tails_keep_their_shapes_and_token_spans() {
        for else_ in ["", " else {}"] {
            for (unary_source, expected) in [
                ("-1", negate(number("1"))),
                ("--1", negate(negate(number("1")))),
            ] {
                let nested_if = format!("if false {{}}{else_}");
                let source = format!(
                    "fn main() -> void {{ if true {{ {nested_if} /* é */ {unary_source} /* end */ }} }}"
                );
                let program = parse(&source).unwrap();
                let StatementKind::If { then, .. } = &program.functions[0].body[0].kind else {
                    panic!("expected outer statement-if");
                };
                let if_start = source.find(&nested_if).unwrap();
                let unary_start = source.find(unary_source).unwrap();

                assert_eq!(then.len(), 2, "{source}");
                assert!(matches!(then[0].kind, StatementKind::If { .. }));
                assert_eq!(
                    then[0].span,
                    Span::new(if_start, if_start + nested_if.len())
                );
                assert_eq!(then[1].kind, StatementKind::Expression(expected));
                assert_eq!(
                    then[1].span,
                    Span::new(unary_start, unary_start + unary_source.len())
                );
            }
        }
    }

    #[test]
    fn whole_if_binary_continuation_in_statement_positions_requires_parentheses() {
        for (operator, op) in [
            ("+", BinaryOp::Add),
            ("-", BinaryOp::Subtract),
            ("*", BinaryOp::Multiply),
            ("/", BinaryOp::Divide),
            ("%", BinaryOp::Remainder),
        ] {
            let if_source = "if true { 1 } else { 2 }";
            let source = format!("fn main() -> void {{ ({if_source}) {operator} 3; }}");
            let program = parse(&source).unwrap();

            assert_eq!(program.functions[0].body.len(), 1);
            let StatementKind::Expression(expr) = &program.functions[0].body[0].kind else {
                panic!("expected expression statement");
            };

            assert_if_arithmetic(expr, op, &number("3"));

            for semicolon in ["", ";"] {
                let Expr::Block(block) =
                    expression(&format!("{{ ({if_source}) {operator} 3{semicolon} }}"))
                else {
                    panic!("expected block");
                };

                if semicolon.is_empty() {
                    assert!(block.statements.is_empty());
                    assert_if_arithmetic(block.tail.as_deref().unwrap(), op, &number("3"));
                } else {
                    assert!(block.tail.is_none());
                    assert_eq!(block.statements.len(), 1);
                    let StatementKind::Expression(expr) = &block.statements[0].kind else {
                        panic!("expected expression statement");
                    };

                    assert_if_arithmetic(expr, op, &number("3"));
                }
            }

            if operator != "-" {
                for source in [
                    format!("fn main() -> void {{ {if_source} {operator} 3; }}"),
                    format!("fn main() -> void {{ let x = {{ {if_source} {operator} 3 }}; }}"),
                ] {
                    let start = source.find(operator).unwrap();
                    assert_eq!(
                        parse(&source),
                        Err(ParseError::UnexpectedToken {
                            expected: "expression",
                            found: operator,
                            span: Span::new(start, start + operator.len()),
                        }),
                        "{source}"
                    );
                }
            }
        }
    }

    #[test]
    fn ordinary_value_contexts_keep_unparenthesized_if_arithmetic() {
        let if_source = "if true { 1 } else if false { 2 } else { 3 }";

        for (operator, op) in [
            ("+", BinaryOp::Add),
            ("-", BinaryOp::Subtract),
            ("*", BinaryOp::Multiply),
            ("/", BinaryOp::Divide),
            ("%", BinaryOp::Remainder),
        ] {
            let value = format!("{if_source} {operator} --4");
            let expected = binary(expression(if_source), op, negate(negate(number("4"))));

            assert_eq!(expression(&value), expected);
            let source = format!(
                "fn main() -> number {{ let x = {value}; x = {value}; x += {value}; call({value}); return {value}; }}"
            );
            let program = parse(&source).unwrap();
            let body = &program.functions[0].body;

            assert_eq!(body.len(), 5);
            let StatementKind::Let { value, .. } = &body[0].kind else {
                panic!("expected initializer");
            };

            assert_if_arithmetic(value, op, &negate(negate(number("4"))));
            let StatementKind::Assignment { value, .. } = &body[1].kind else {
                panic!("expected assignment");
            };

            assert_if_arithmetic(value, op, &negate(negate(number("4"))));
            let StatementKind::CompoundAssignment { value, .. } = &body[2].kind else {
                panic!("expected compound assignment");
            };

            assert_if_arithmetic(value, op, &negate(negate(number("4"))));
            let StatementKind::Expression(Expr::Call { func, args }) = &body[3].kind else {
                panic!("expected call");
            };

            assert_eq!(func, "call");
            assert_eq!(args.len(), 1);
            assert_if_arithmetic(&args[0].value, op, &negate(negate(number("4"))));
            let StatementKind::Return(Some(value)) = &body[4].kind else {
                panic!("expected return value");
            };

            assert_if_arithmetic(value, op, &negate(negate(number("4"))));
        }
    }

    #[test]
    fn legacy_if_followed_by_unary_statements_preserves_head_wasm_bytes() {
        // Literal bytes captured by separately compiling these sources with an
        // isolated archive of HEAD feb869075147, not the working-tree compiler.
        let prefix = [
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
            0x02, 0x01, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x08, 0x01, 0x04, 0x6d, 0x61, 0x69,
            0x6e, 0x00, 0x00,
        ];
        let cases: [(&str, &[u8]); 6] = [
            (
                "fn main() -> void { if true {} -1; }",
                &[
                    0x0a, 0x14, 0x01, 0x12, 0x00, 0x41, 0x01, 0x04, 0x40, 0x0b, 0x44, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x1a, 0x0b,
                ],
            ),
            (
                "fn main() -> void { if true {} --1; }",
                &[
                    0x0a, 0x15, 0x01, 0x13, 0x00, 0x41, 0x01, 0x04, 0x40, 0x0b, 0x44, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x9a, 0x1a, 0x0b,
                ],
            ),
            (
                "fn main() -> void { if true {} else {} -1; }",
                &[
                    0x0a, 0x15, 0x01, 0x13, 0x00, 0x41, 0x01, 0x04, 0x40, 0x05, 0x0b, 0x44, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x1a, 0x0b,
                ],
            ),
            (
                "fn main() -> void { if true {} else {} --1; }",
                &[
                    0x0a, 0x16, 0x01, 0x14, 0x00, 0x41, 0x01, 0x04, 0x40, 0x05, 0x0b, 0x44, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x9a, 0x1a, 0x0b,
                ],
            ),
            (
                "fn main() -> void { if true { if false {} -1; } --1; }",
                &[
                    0x0a, 0x25, 0x01, 0x23, 0x00, 0x41, 0x01, 0x04, 0x40, 0x41, 0x00, 0x04, 0x40,
                    0x0b, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x1a, 0x0b,
                    0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x9a, 0x1a, 0x0b,
                ],
            ),
            (
                "fn main() -> void { if true { if false {} else {} --1; } else { if false {} -1; } -1; }",
                &[
                    0x0a, 0x37, 0x01, 0x35, 0x00, 0x41, 0x01, 0x04, 0x40, 0x41, 0x00, 0x04, 0x40,
                    0x05, 0x0b, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, 0x9a, 0x9a,
                    0x1a, 0x05, 0x41, 0x00, 0x04, 0x40, 0x0b, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0xf0, 0x3f, 0x9a, 0x1a, 0x0b, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0xf0, 0x3f, 0x9a, 0x1a, 0x0b,
                ],
            ),
        ];

        for (source, code_section) in cases {
            let expected = [prefix.as_slice(), code_section].concat();

            for source in [source.to_owned(), source.replace(' ', " /* é */\n")] {
                let actual = crate::Compiler::new().compile_source(&source).unwrap();

                wasmparser::Validator::new().validate_all(&actual).unwrap();
                assert_eq!(actual, expected, "{source}");
            }
        }
    }

    #[test]
    fn truncated_value_blocks_and_if_expressions_never_panic() {
        let source = "fn test() -> number { let x = { if true { 1 } else if false { return 2; } else { let y = 3; y } }; return x; }";
        let tokens = Lexer::new(source).tokenize();

        for end in 0..tokens.len() - 1 {
            assert!(
                Parser::new(&tokens[..end]).parse_program().is_err(),
                "token {end}"
            );
        }

        for source in [
            "fn test() -> void { let x = { 1",
            "fn test() -> void { let x = if true { 1 } else",
            "fn test() -> void { let x = if true { 1 } else if",
            "fn test() -> void { let x = { let a = 1;",
        ] {
            assert!(parse(source).is_err());
        }
    }

    #[test]
    fn truncated_programs_never_panic() {
        for declaration in ["let y: number", "let y"] {
            let source = format!(
                "fn compute(xs: number[], x: number) -> number {{ {declaration} = xs[x] + call(1, 2); if y > 0 {{ return y; }} else {{ return -y; }} }}"
            );
            let tokens = Lexer::new(&source).tokenize();

            for end in 0..tokens.len() - 1 {
                assert!(
                    Parser::new(&tokens[..end]).parse_program().is_err(),
                    "accepted truncated input at token {end} for {declaration}"
                );
            }
        }
    }
}
