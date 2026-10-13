use super::*;
use crate::number_literal::{self, LiteralError};

#[derive(Clone, Copy)]
pub(super) enum PureExpressionContext {
    Default,
    Contract,
}

impl PureExpressionContext {
    fn invalid(self, reason: &'static str, span: Span) -> SemanticError {
        match self {
            Self::Default => SemanticError::InvalidDefaultExpression { reason, span },
            Self::Contract => SemanticError::InvalidContractExpression { reason, span },
        }
    }

    fn check_number(self, literal: &str, span: Span) -> Result<(), SemanticError> {
        number_literal::parse(literal).map(|_| ()).map_err(|error| {
            let literal = literal.to_owned();
            match (self, error) {
                (Self::Default, LiteralError::Invalid) => {
                    SemanticError::InvalidDefaultNumberLiteral { literal, span }
                }
                (Self::Default, LiteralError::OutOfRange) => {
                    SemanticError::DefaultNumberLiteralOutOfRange { literal, span }
                }
                (Self::Contract, LiteralError::Invalid) => {
                    SemanticError::InvalidContractNumberLiteral { literal, span }
                }
                (Self::Contract, LiteralError::OutOfRange) => {
                    SemanticError::ContractNumberLiteralOutOfRange { literal, span }
                }
            }
        })
    }
}

impl SemanticAnalyzer {
    pub(super) fn check_pure_expression(
        &self,
        expr: &Expr,
        span: Span,
        context: PureExpressionContext,
    ) -> Result<(), SemanticError> {
        let invalid = |reason| context.invalid(reason, span);

        match expr {
            Expr::Number(literal) => {
                context.check_number(literal, span)?;
            }
            Expr::String(_) | Expr::Bool(_) => {}
            Expr::Identifier(name) => {
                if self.lookup_variable(name).is_none() {
                    return Err(SemanticError::UndefinedVariable {
                        name: name.clone(),
                        span,
                    });
                }
            }
            Expr::Unary { op, expr } => {
                match op {
                    UnaryOp::Negate | UnaryOp::Not => {}
                }

                self.check_pure_expression(expr, span, context)?;
            }
            Expr::Binary { left, op, right } => {
                match op {
                    BinaryOp::In => return Err(invalid("range membership is not allowed")),
                    BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::Remainder
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::LessThan
                    | BinaryOp::GreaterThan
                    | BinaryOp::LessThanOrEqual
                    | BinaryOp::GreaterThanOrEqual
                    | BinaryOp::And
                    | BinaryOp::Or => {}
                }

                self.check_pure_expression(left, span, context)?;
                self.check_pure_expression(right, span, context)?;
            }
            Expr::Call { func, args } => {
                if self.lookup_variable(func).is_some() {
                    return Err(SemanticError::NotCallable {
                        name: func.clone(),
                        span,
                    });
                }

                if !matches!(
                    self.functions
                        .get(func)
                        .map(|signature| signature.reference),
                    Some(FunctionRef::Builtin(BuiltinFunction::Len))
                ) || args.iter().any(|arg| arg.name.is_some())
                {
                    return Err(invalid("only positional builtin len calls are allowed"));
                }

                for arg in args {
                    self.check_pure_expression(&arg.value, span, context)?;
                }
            }
            Expr::Array(_) => return Err(invalid("array construction is not allowed")),
            Expr::Range { .. } => return Err(invalid("range construction is not allowed")),
            Expr::Index { .. } => return Err(invalid("indexing is not allowed")),
            Expr::Block(_) => return Err(invalid("blocks are not allowed")),
            Expr::Match { .. } => return Err(invalid("match expressions are not allowed")),
            Expr::If { .. } => return Err(invalid("if expressions are not allowed")),
            Expr::Loop { .. } => return Err(invalid("loop expressions are not allowed")),
        }

        Ok(())
    }
}
