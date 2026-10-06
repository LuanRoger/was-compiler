use std::collections::{HashMap, hash_map::Entry};

use thiserror::Error;

use crate::{
    ast::{BinaryOp, Block, Expr, Function, Program, Statement, StatementKind, Type, UnaryOp},
    host::HostFunctions,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, FunctionId, FunctionRef, TypedBlock, TypedExpr, TypedExprKind,
        TypedFunction, TypedParam, TypedProgram, TypedStatement, TypedStatementKind, VariableId,
    },
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
    reference: FunctionRef,
    params: Vec<Type>,
    return_type: Type,
}

#[derive(Clone, Copy)]
struct VariableBinding {
    id: VariableId,
    ty: Type,
}

#[derive(Default)]
pub struct SemanticAnalyzer {
    host_functions: HostFunctions,
    functions: HashMap<String, FunctionSignature>,
    scopes: Vec<HashMap<String, VariableBinding>>,
    next_variable_id: usize,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_host_functions(host_functions: &HostFunctions) -> Self {
        Self {
            host_functions: host_functions.clone(),
            ..Self::default()
        }
    }

    pub fn analyze(&mut self, program: &Program) -> Result<TypedProgram, SemanticError> {
        self.functions.clear();
        self.scopes.clear();
        self.next_variable_id = 0;

        self.functions.insert(
            "len".into(),
            FunctionSignature {
                reference: FunctionRef::Builtin(BuiltinFunction::Len),
                params: vec![Type::NumberArray],
                return_type: Type::Number,
            },
        );

        for function in self.host_functions.functions() {
            self.functions.insert(
                function.name.clone(),
                FunctionSignature {
                    reference: FunctionRef::Host(function.id),
                    params: function.params.clone(),
                    return_type: function.return_type,
                },
            );
        }

        let result = self.analyze_program(program);

        self.scopes.clear();

        result
    }

    fn analyze_program(&mut self, program: &Program) -> Result<TypedProgram, SemanticError> {
        for (index, function) in program.functions.iter().enumerate() {
            let signature = FunctionSignature {
                reference: FunctionRef::User(FunctionId(index)),
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

        let mut functions = Vec::with_capacity(program.functions.len());

        for (index, function) in program.functions.iter().enumerate() {
            functions.push(self.analyze_function(function, FunctionId(index))?);
        }

        Ok(TypedProgram {
            host_functions: self.host_functions.functions().to_vec(),
            functions,
        })
    }

    fn analyze_function(
        &mut self,
        function: &Function,
        id: FunctionId,
    ) -> Result<TypedFunction, SemanticError> {
        self.next_variable_id = 0;

        self.scopes.push(HashMap::new());

        let result = self.analyze_function_body(function, id);

        self.scopes.pop();

        result
    }

    fn analyze_function_body(
        &mut self,
        function: &Function,
        id: FunctionId,
    ) -> Result<TypedFunction, SemanticError> {
        let mut params = Vec::with_capacity(function.params.len());

        for param in &function.params {
            let id = self.declare_variable(&param.name, param.ty, param.span)?;

            params.push(TypedParam {
                id,
                name: param.name.clone(),
                ty: param.ty,
                span: param.span,
            });
        }

        let (body, always_returns) = self.analyze_block(&function.body, function.return_type)?;

        if function.return_type != Type::Void && !always_returns {
            return Err(SemanticError::MissingReturn {
                name: function.name.clone(),
                span: function.span,
            });
        }

        Ok(TypedFunction {
            id,
            name: function.name.clone(),
            params,
            return_type: function.return_type,
            body,
            span: function.span,
        })
    }

    fn analyze_block(
        &mut self,
        block: &Block,
        return_type: Type,
    ) -> Result<(TypedBlock, bool), SemanticError> {
        let mut statements = Vec::with_capacity(block.len());
        let mut always_returns = false;

        for statement in block {
            let (statement, returns) = self.analyze_statement(statement, return_type)?;

            statements.push(statement);

            always_returns |= returns;
        }

        Ok((statements, always_returns))
    }

    fn analyze_scoped_block(
        &mut self,
        block: &Block,
        return_type: Type,
    ) -> Result<(TypedBlock, bool), SemanticError> {
        self.scopes.push(HashMap::new());

        let result = self.analyze_block(block, return_type);

        self.scopes.pop();

        result
    }

    fn analyze_statement(
        &mut self,
        statement: &Statement,
        return_type: Type,
    ) -> Result<(TypedStatement, bool), SemanticError> {
        let span = statement.span;
        let mut always_returns = false;

        let kind = match &statement.kind {
            StatementKind::Let { name, ty, value } => {
                self.check_variable_type(name, *ty, span)?;

                let value = self.analyze_expr(value, span)?;

                self.expect_type(*ty, value.ty, span)?;

                let id = self.declare_variable(name, *ty, span)?;

                TypedStatementKind::Let {
                    id,
                    name: name.clone(),
                    ty: *ty,
                    value,
                }
            }

            StatementKind::Assignment { target, value } => {
                let target = self.analyze_assignment_target(target, span)?;
                let value = self.analyze_expr(value, span)?;

                self.expect_type(target.ty, value.ty, span)?;

                TypedStatementKind::Assignment { target, value }
            }

            StatementKind::If { cond, then, else_ } => {
                let cond = Box::new(self.analyze_expr(cond, span)?);

                self.expect_type(Type::Bool, cond.ty, span)?;

                let (then, then_returns) = self.analyze_scoped_block(then, return_type)?;
                let (else_, else_returns) = match else_ {
                    Some(block) => {
                        let (block, returns) = self.analyze_scoped_block(block, return_type)?;

                        (Some(block), returns)
                    }

                    None => (None, false),
                };

                always_returns = then_returns && else_returns;

                TypedStatementKind::If { cond, then, else_ }
            }

            StatementKind::While { cond, body } => {
                let cond = Box::new(self.analyze_expr(cond, span)?);

                self.expect_type(Type::Bool, cond.ty, span)?;

                let (body, _) = self.analyze_scoped_block(body, return_type)?;

                TypedStatementKind::While { cond, body }
            }

            StatementKind::Return(value) => {
                let value = match value {
                    Some(_) if return_type == Type::Void => {
                        return Err(SemanticError::UnexpectedReturnValue { span });
                    }

                    Some(expr) => {
                        let expr = self.analyze_expr(expr, span)?;

                        self.expect_type(return_type, expr.ty, span)?;

                        Some(expr)
                    }

                    None => {
                        self.expect_type(return_type, Type::Void, span)?;

                        None
                    }
                };

                always_returns = true;

                TypedStatementKind::Return(value)
            }

            StatementKind::Expression(expr) => {
                TypedStatementKind::Expression(self.analyze_expr(expr, span)?)
            }
        };

        Ok((TypedStatement { kind, span }, always_returns))
    }

    fn analyze_assignment_target(
        &self,
        target: &Expr,
        span: Span,
    ) -> Result<TypedExpr, SemanticError> {
        match target {
            Expr::Identifier(_) => self.analyze_expr(target, span),
            Expr::Index { expr, .. } if matches!(**expr, Expr::Identifier(_)) => {
                self.analyze_expr(target, span)
            }

            _ => Err(SemanticError::InvalidAssignmentTarget { span }),
        }
    }

    fn analyze_expr(&self, expr: &Expr, span: Span) -> Result<TypedExpr, SemanticError> {
        let (kind, ty) = match expr {
            Expr::Number(value) => (TypedExprKind::Number(value.clone()), Type::Number),
            Expr::String(value) => (TypedExprKind::String(value.clone()), Type::String),
            Expr::Bool(value) => (TypedExprKind::Bool(*value), Type::Bool),

            Expr::Identifier(name) => {
                let binding =
                    self.lookup_variable(name)
                        .ok_or_else(|| SemanticError::UndefinedVariable {
                            name: name.clone(),
                            span,
                        })?;

                (
                    TypedExprKind::Identifier {
                        id: binding.id,
                        name: name.clone(),
                    },
                    binding.ty,
                )
            }

            Expr::Unary { op, expr } => {
                let expr = Box::new(self.analyze_expr(expr, span)?);
                let ty = match op {
                    UnaryOp::Not => Type::Bool,
                    UnaryOp::Negate => Type::Number,
                };

                self.expect_type(ty, expr.ty, span)?;

                (TypedExprKind::Unary { op: *op, expr }, ty)
            }

            Expr::Binary { left, op, right } => {
                let left = Box::new(self.analyze_expr(left, span)?);
                let right = Box::new(self.analyze_expr(right, span)?);
                let ty = self.analyze_binary(op, left.ty, right.ty, span)?;

                (
                    TypedExprKind::Binary {
                        left,
                        op: *op,
                        right,
                    },
                    ty,
                )
            }

            Expr::Call { func, args } => return self.analyze_call(func, args, span),

            Expr::Array(elements) => {
                let mut values = Vec::with_capacity(elements.len());

                for element in elements {
                    let value = self.analyze_expr(element, span)?;

                    self.expect_type(Type::Number, value.ty, span)?;

                    values.push(value);
                }

                (TypedExprKind::Array(values), Type::NumberArray)
            }

            Expr::Index { expr, index } => {
                let expr = Box::new(self.analyze_expr(expr, span)?);

                self.expect_type(Type::NumberArray, expr.ty, span)?;

                let index = Box::new(self.analyze_expr(index, span)?);

                self.expect_type(Type::Number, index.ty, span)?;

                (TypedExprKind::Index { expr, index }, Type::Number)
            }
        };

        Ok(TypedExpr { kind, ty })
    }

    fn analyze_call(
        &self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<TypedExpr, SemanticError> {
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

        let mut arguments = Vec::with_capacity(args.len());

        for (arg, expected) in args.iter().zip(&signature.params) {
            let arg = self.analyze_expr(arg, span)?;

            self.expect_type(*expected, arg.ty, span)?;

            arguments.push(arg);
        }

        Ok(TypedExpr {
            kind: TypedExprKind::Call {
                func: signature.reference,
                name: name.into(),
                args: arguments,
            },
            ty: signature.return_type,
        })
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

    fn lookup_variable(&self, name: &str) -> Option<VariableBinding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn declare_variable(
        &mut self,
        name: &str,
        ty: Type,
        span: Span,
    ) -> Result<VariableId, SemanticError> {
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
                let id = VariableId(self.next_variable_id);

                self.next_variable_id += 1;

                entry.insert(VariableBinding { id, ty });

                Ok(id)
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

#[cfg(test)]
#[path = "typed_ast_tests.rs"]
mod typed_tests;
