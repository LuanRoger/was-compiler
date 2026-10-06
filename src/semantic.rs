use std::collections::{HashMap, hash_map::Entry};

use thiserror::Error;

use crate::{
    ast::{BinaryOp, Block, Expr, Function, Program, Statement, StatementKind, Type, UnaryOp},
    tokens::Span,
};

#[derive(Debug, Error, PartialEq)]
pub enum SemanticError {
    #[error("Function '{name}' is already declared at {span:?}")]
    DuplicateFunction { name: String, span: Span },

    #[error("Variable '{name}' is already declared in this scope at {span:?}")]
    DuplicateVariable { name: String, span: Span },

    #[error("Undefined variable '{name}' at {span:?}")]
    UndefinedVariable { name: String, span: Span },

    #[error("Undefined function '{name}' at {span:?}")]
    UndefinedFunction { name: String, span: Span },

    #[error("Variable '{name}' cannot be called as a function at {span:?}")]
    NotCallable { name: String, span: Span },

    #[error("'{name}' cannot have type void at {span:?}")]
    InvalidVoidType { name: String, span: Span },

    #[error("Type mismatch: expected {expected:?}, found {found:?} at {span:?}")]
    TypeMismatch {
        expected: Type,
        found: Type,
        span: Span,
    },

    #[error("Function '{name}' expects {expected} arguments, found {found} at {span:?}")]
    ArgumentCountMismatch {
        name: String,
        expected: usize,
        found: usize,
        span: Span,
    },

    #[error("Operator '{operator}' cannot be applied to {left:?} and {right:?} at {span:?}")]
    InvalidBinaryOperands {
        operator: &'static str,
        left: Type,
        right: Type,
        span: Span,
    },

    #[error("Assignment target must be a variable or array element at {span:?}")]
    InvalidAssignmentTarget { span: Span },

    #[error("A void function cannot return a value at {span:?}")]
    UnexpectedReturnValue { span: Span },

    #[error("Function '{name}' does not return a value on every path at {span:?}")]
    MissingReturn { name: String, span: Span },
}

struct FunctionSignature {
    params: Vec<Type>,
    return_type: Type,
}

#[derive(Default)]
pub struct SemanticAnalyzer {
    functions: HashMap<String, FunctionSignature>,
    scopes: Vec<HashMap<String, Type>>,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn analyze(&mut self, program: &Program) -> Result<(), SemanticError> {
        self.functions.clear();
        self.scopes.clear();

        self.functions.insert(
            "len".into(),
            FunctionSignature {
                params: vec![Type::NumberArray],
                return_type: Type::Number,
            },
        );

        let result = self.analyze_program(program);

        self.scopes.clear();

        result
    }

    fn analyze_program(&mut self, program: &Program) -> Result<(), SemanticError> {
        for function in &program.functions {
            let signature = FunctionSignature {
                params: function.params.iter().map(|param| param.ty).collect(),
                return_type: function.return_type,
            };

            if self
                .functions
                .insert(function.name.clone(), signature)
                .is_some()
            {
                return Err(SemanticError::DuplicateFunction {
                    name: function.name.clone(),
                    span: function.span,
                });
            }

            for param in &function.params {
                self.check_variable_type(&param.name, param.ty, param.span)?;
            }
        }

        for function in &program.functions {
            self.analyze_function(function)?;
        }

        Ok(())
    }

    fn analyze_function(&mut self, function: &Function) -> Result<(), SemanticError> {
        self.scopes.push(HashMap::new());

        let result = self.analyze_function_body(function);

        self.scopes.pop();

        result
    }

    fn analyze_function_body(&mut self, function: &Function) -> Result<(), SemanticError> {
        for param in &function.params {
            self.declare_variable(&param.name, param.ty, param.span)?;
        }

        let always_returns = self.analyze_block(&function.body, function.return_type)?;

        if function.return_type != Type::Void && !always_returns {
            return Err(SemanticError::MissingReturn {
                name: function.name.clone(),
                span: function.span,
            });
        }

        Ok(())
    }

    fn analyze_block(&mut self, block: &Block, return_type: Type) -> Result<bool, SemanticError> {
        let mut always_returns = false;

        for statement in block {
            let returns = self.analyze_statement(statement, return_type)?;

            always_returns |= returns;
        }

        Ok(always_returns)
    }

    fn analyze_scoped_block(
        &mut self,
        block: &Block,
        return_type: Type,
    ) -> Result<bool, SemanticError> {
        self.scopes.push(HashMap::new());

        let result = self.analyze_block(block, return_type);

        self.scopes.pop();

        result
    }

    fn analyze_statement(
        &mut self,
        statement: &Statement,
        return_type: Type,
    ) -> Result<bool, SemanticError> {
        let span = statement.span;

        match &statement.kind {
            StatementKind::Let { name, ty, value } => {
                self.check_variable_type(name, *ty, span)?;

                let found = self.analyze_expr(value, span)?;

                self.expect_type(*ty, found, span)?;
                self.declare_variable(name, *ty, span)?;
            }

            StatementKind::Assignment { target, value } => {
                let expected = self.analyze_assignment_target(target, span)?;
                let found = self.analyze_expr(value, span)?;

                self.expect_type(expected, found, span)?;
            }

            StatementKind::If { cond, then, else_ } => {
                let condition_type = self.analyze_expr(cond, span)?;

                self.expect_type(Type::Bool, condition_type, span)?;

                let then_returns = self.analyze_scoped_block(then, return_type)?;
                let else_returns = match else_ {
                    Some(block) => self.analyze_scoped_block(block, return_type)?,
                    None => false,
                };

                return Ok(then_returns && else_returns);
            }

            StatementKind::While { cond, body } => {
                let condition_type = self.analyze_expr(cond, span)?;

                self.expect_type(Type::Bool, condition_type, span)?;
                self.analyze_scoped_block(body, return_type)?;
            }

            StatementKind::Return(value) => {
                match value {
                    Some(_) if return_type == Type::Void => {
                        return Err(SemanticError::UnexpectedReturnValue { span });
                    }

                    Some(expr) => {
                        let found = self.analyze_expr(expr, span)?;

                        self.expect_type(return_type, found, span)?;
                    }

                    None => self.expect_type(return_type, Type::Void, span)?,
                }

                return Ok(true);
            }

            StatementKind::Expression(expr) => {
                self.analyze_expr(expr, span)?;
            }
        }

        Ok(false)
    }

    fn analyze_assignment_target(&self, target: &Expr, span: Span) -> Result<Type, SemanticError> {
        match target {
            Expr::Identifier(_) => self.analyze_expr(target, span),
            Expr::Index { expr, .. } if matches!(**expr, Expr::Identifier(_)) => {
                self.analyze_expr(target, span)
            }

            _ => Err(SemanticError::InvalidAssignmentTarget { span }),
        }
    }

    fn analyze_expr(&self, expr: &Expr, span: Span) -> Result<Type, SemanticError> {
        match expr {
            Expr::Number(_) => Ok(Type::Number),
            Expr::String(_) => Ok(Type::String),
            Expr::Bool(_) => Ok(Type::Bool),

            Expr::Identifier(name) => {
                self.lookup_variable(name)
                    .ok_or_else(|| SemanticError::UndefinedVariable {
                        name: name.clone(),
                        span,
                    })
            }

            Expr::Unary { op, expr } => {
                let found = self.analyze_expr(expr, span)?;
                let expected = match op {
                    UnaryOp::Not => Type::Bool,
                    UnaryOp::Negate => Type::Number,
                };

                self.expect_type(expected, found, span)?;

                Ok(expected)
            }

            Expr::Binary { left, op, right } => {
                let left = self.analyze_expr(left, span)?;
                let right = self.analyze_expr(right, span)?;

                self.analyze_binary(op, left, right, span)
            }

            Expr::Call { func, args } => self.analyze_call(func, args, span),

            Expr::Array(elements) => {
                for element in elements {
                    let found = self.analyze_expr(element, span)?;

                    self.expect_type(Type::Number, found, span)?;
                }

                Ok(Type::NumberArray)
            }

            Expr::Index { expr, index } => {
                let array_type = self.analyze_expr(expr, span)?;

                self.expect_type(Type::NumberArray, array_type, span)?;

                let index_type = self.analyze_expr(index, span)?;

                self.expect_type(Type::Number, index_type, span)?;

                Ok(Type::Number)
            }
        }
    }

    fn analyze_call(&self, name: &str, args: &[Expr], span: Span) -> Result<Type, SemanticError> {
        if self.lookup_variable(name).is_some() {
            return Err(SemanticError::NotCallable {
                name: name.into(),
                span,
            });
        }

        let signature =
            self.functions
                .get(name)
                .ok_or_else(|| SemanticError::UndefinedFunction {
                    name: name.into(),
                    span,
                })?;

        if signature.params.len() != args.len() {
            return Err(SemanticError::ArgumentCountMismatch {
                name: name.into(),
                expected: signature.params.len(),
                found: args.len(),
                span,
            });
        }

        for (arg, expected) in args.iter().zip(&signature.params) {
            let found = self.analyze_expr(arg, span)?;

            self.expect_type(*expected, found, span)?;
        }

        Ok(signature.return_type)
    }

    fn analyze_binary(
        &self,
        op: &BinaryOp,
        left: Type,
        right: Type,
        span: Span,
    ) -> Result<Type, SemanticError> {
        let (operator, operand_type, result_type) = match op {
            BinaryOp::Add => ("+", Type::Number, Type::Number),
            BinaryOp::Subtract => ("-", Type::Number, Type::Number),
            BinaryOp::Multiply => ("*", Type::Number, Type::Number),
            BinaryOp::Divide => ("/", Type::Number, Type::Number),
            BinaryOp::Remainder => ("%", Type::Number, Type::Number),

            BinaryOp::LessThan => ("<", Type::Number, Type::Bool),
            BinaryOp::LessThanOrEqual => ("<=", Type::Number, Type::Bool),
            BinaryOp::GreaterThan => (">", Type::Number, Type::Bool),
            BinaryOp::GreaterThanOrEqual => (">=", Type::Number, Type::Bool),

            BinaryOp::And => ("&&", Type::Bool, Type::Bool),
            BinaryOp::Or => ("||", Type::Bool, Type::Bool),

            BinaryOp::Equal | BinaryOp::NotEqual => {
                let operator = if *op == BinaryOp::Equal { "==" } else { "!=" };

                if left == right && matches!(left, Type::Number | Type::Bool | Type::String) {
                    return Ok(Type::Bool);
                }

                return Err(SemanticError::InvalidBinaryOperands {
                    operator,
                    left,
                    right,
                    span,
                });
            }
        };

        if left != operand_type || right != operand_type {
            return Err(SemanticError::InvalidBinaryOperands {
                operator,
                left,
                right,
                span,
            });
        }

        Ok(result_type)
    }

    fn lookup_variable(&self, name: &str) -> Option<Type> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn declare_variable(&mut self, name: &str, ty: Type, span: Span) -> Result<(), SemanticError> {
        let scope = self
            .scopes
            .last_mut()
            .expect("declarations require a function scope");

        match scope.entry(name.into()) {
            Entry::Occupied(_) => Err(SemanticError::DuplicateVariable {
                name: name.into(),
                span,
            }),

            Entry::Vacant(entry) => {
                entry.insert(ty);

                Ok(())
            }
        }
    }

    fn check_variable_type(&self, name: &str, ty: Type, span: Span) -> Result<(), SemanticError> {
        if ty == Type::Void {
            return Err(SemanticError::InvalidVoidType {
                name: name.into(),
                span,
            });
        }

        Ok(())
    }

    fn expect_type(&self, expected: Type, found: Type, span: Span) -> Result<(), SemanticError> {
        if expected != found {
            return Err(SemanticError::TypeMismatch {
                expected,
                found,
                span,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "semantic_tests.rs"]
mod tests;
