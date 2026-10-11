use std::collections::{HashMap, hash_map::Entry};

use thiserror::Error;

use crate::{
    ast::{
        BinaryOp, Block, Expr, Function, Program, Statement, StatementKind, Type, UnaryOp,
        ValueBlock,
    },
    flow::Flow,
    host::HostFunctions,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, ExprResult, FunctionId, FunctionRef, LoopId, TypedBlock, TypedExpr,
        TypedExprKind, TypedFunction, TypedParam, TypedProgram, TypedStatement, TypedStatementKind,
        TypedValueBlock, VariableId,
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

    #[error("An if expression requires an else branch at {span:?}")]
    MissingElse { span: Span },

    #[error("Cannot infer a concrete type for divergent initializer '{name}' at {span:?}")]
    CannotInferNeverType { name: String, span: Span },

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

    #[error("Compound assignment requires an arithmetic operator, found {op:?} at {span:?}")]
    InvalidCompoundAssignmentOperator { op: BinaryOp, span: Span },

    #[error("Break is only allowed inside a loop at {span:?}")]
    BreakOutsideLoop { span: Span },

    #[error("A break value is not allowed in a statement loop at {span:?}")]
    BreakValueNotAllowed { span: Span },

    #[error("A value loop requires a break value at {span:?}")]
    MissingBreakValue { span: Span },

    #[error("A break value cannot have type void at {span:?}")]
    InvalidBreakValue { span: Span },

    #[error("Continue is only allowed inside a loop at {span:?}")]
    ContinueOutsideLoop { span: Span },

    #[error("A void function cannot return a value at {span:?}")]
    UnexpectedReturnValue { span: Span },

    #[error("Function '{name}' does not return a value on every path at {span:?}")]
    MissingReturn { name: String, span: Span },
}

impl SemanticError {
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::DuplicateFunction { span, .. }
            | Self::DuplicateVariable { span, .. }
            | Self::UndefinedVariable { span, .. }
            | Self::UndefinedFunction { span, .. }
            | Self::NotCallable { span, .. }
            | Self::InvalidVoidType { span, .. }
            | Self::MissingElse { span }
            | Self::CannotInferNeverType { span, .. }
            | Self::TypeMismatch { span, .. }
            | Self::ArgumentCountMismatch { span, .. }
            | Self::InvalidBinaryOperands { span, .. }
            | Self::InvalidAssignmentTarget { span }
            | Self::InvalidCompoundAssignmentOperator { span, .. }
            | Self::BreakOutsideLoop { span }
            | Self::BreakValueNotAllowed { span }
            | Self::MissingBreakValue { span }
            | Self::InvalidBreakValue { span }
            | Self::ContinueOutsideLoop { span }
            | Self::UnexpectedReturnValue { span }
            | Self::MissingReturn { span, .. } => Some(*span),
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopKind {
    Statement,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LoopContext {
    id: LoopId,
    kind: LoopKind,
}

#[derive(Default)]
pub struct SemanticAnalyzer {
    host_functions: HostFunctions,
    functions: HashMap<String, FunctionSignature>,
    scopes: Vec<HashMap<String, VariableBinding>>,
    next_variable_id: usize,
    loops: Vec<LoopContext>,
    next_loop_id: usize,
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
        self.loops.clear();
        self.next_loop_id = 0;

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
        self.loops.clear();

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
        self.next_loop_id = 0;
        self.loops.clear();

        self.scopes.push(HashMap::new());

        let result = self.analyze_function_body(function, id);

        self.scopes.pop();
        self.loops.clear();

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

        let (body, flow) = self.analyze_block(&function.body, function.return_type)?;

        if function.return_type != Type::Void && !flow.returns_or_diverges() {
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
    ) -> Result<(TypedBlock, Flow), SemanticError> {
        let mut statements = Vec::with_capacity(block.len());
        let mut flow = Flow::FALLTHROUGH;

        for statement in block {
            let (statement, next) = self.analyze_statement(statement, return_type)?;

            statements.push(statement);

            flow = flow.then(next);
        }

        Ok((statements, flow))
    }

    fn analyze_scoped_block(
        &mut self,
        block: &Block,
        return_type: Type,
    ) -> Result<(TypedBlock, Flow), SemanticError> {
        self.scopes.push(HashMap::new());

        let result = self.analyze_block(block, return_type);

        self.scopes.pop();

        result
    }

    fn analyze_statement(
        &mut self,
        statement: &Statement,
        return_type: Type,
    ) -> Result<(TypedStatement, Flow), SemanticError> {
        let span = statement.span;
        let mut flow = Flow::FALLTHROUGH;

        let kind = match &statement.kind {
            StatementKind::Let { name, ty, value } => {
                if let Some(ty) = ty {
                    self.check_variable_type(name, *ty, span)?;
                }

                let (value, value_flow) = self.analyze_expr(value, span, return_type)?;

                flow = value_flow;

                let ty = match ty {
                    Some(ty) => {
                        self.expect_type(*ty, value.ty, span)?;

                        *ty
                    }

                    None => match value.ty {
                        ExprResult::Value(ty) => ty,
                        ExprResult::Void => {
                            return Err(SemanticError::InvalidVoidType {
                                name: name.clone(),
                                span,
                            });
                        }
                        ExprResult::Never => {
                            return Err(SemanticError::CannotInferNeverType {
                                name: name.clone(),
                                span,
                            });
                        }
                    },
                };

                let id = self.declare_variable(name, ty, span)?;

                TypedStatementKind::Let {
                    id,
                    name: name.clone(),
                    ty,
                    value,
                }
            }

            StatementKind::Assignment { target, value } => {
                let (target, target_flow) =
                    self.analyze_assignment_target(target, span, return_type)?;
                let (value, value_flow) = self.analyze_expr(value, span, return_type)?;

                // An indexed place may diverge while computing its index, but its
                // storage type is still number and its RHS must still be checked.
                let target_type = match target.ty {
                    ExprResult::Value(ty) => ty,
                    ExprResult::Never => Type::Number,
                    ExprResult::Void => unreachable!("checked places have a storage type"),
                };

                self.expect_type(target_type, value.ty, span)?;
                flow = target_flow.then(value_flow);

                TypedStatementKind::Assignment { target, value }
            }

            StatementKind::CompoundAssignment { target, op, value } => {
                let (target, target_flow) =
                    self.analyze_assignment_target(target, span, return_type)?;

                self.expect_type(Type::Number, target.ty, span)?;

                if !matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Remainder
                ) {
                    return Err(SemanticError::InvalidCompoundAssignmentOperator { op: *op, span });
                }

                let (value, value_flow) = self.analyze_expr(value, span, return_type)?;

                self.expect_type(Type::Number, value.ty, span)?;
                flow = target_flow.then(value_flow);

                TypedStatementKind::CompoundAssignment {
                    target,
                    op: *op,
                    value,
                }
            }

            StatementKind::If { cond, then, else_ } => {
                let (cond, cond_flow) = self.analyze_expr(cond, span, return_type)?;
                let cond = Box::new(cond);

                self.expect_type(Type::Bool, cond.ty, span)?;

                let (then, then_flow) = self.analyze_scoped_block(then, return_type)?;
                let (else_, else_flow) = match else_ {
                    Some(block) => {
                        let (block, flow) = self.analyze_scoped_block(block, return_type)?;

                        (Some(block), flow)
                    }

                    None => (None, Flow::FALLTHROUGH),
                };

                flow = cond_flow.then(then_flow.join(else_flow));

                TypedStatementKind::If { cond, then, else_ }
            }

            StatementKind::While { cond, body } => {
                let id = LoopId(self.next_loop_id);

                self.next_loop_id += 1;
                self.loops.push(LoopContext {
                    id,
                    kind: LoopKind::Statement,
                });

                let result = (|| {
                    let (cond, cond_flow) = self.analyze_expr(cond, span, return_type)?;

                    self.expect_type(Type::Bool, cond.ty, span)?;

                    let (body, body_flow) = self.analyze_scoped_block(body, return_type)?;

                    Ok((Box::new(cond), body, cond_flow, body_flow))
                })();

                self.loops.pop();

                let (cond, body, cond_flow, body_flow) = result?;

                flow = if cond_flow.can_fall_through() {
                    cond_flow.then(body_flow).while_loop()
                } else {
                    cond_flow.infinite_loop()
                };

                TypedStatementKind::While { id, cond, body }
            }

            StatementKind::Loop { body } => {
                let (id, body, body_flow) =
                    self.analyze_loop_body(body, return_type, LoopKind::Statement)?;

                flow = body_flow.infinite_loop();

                TypedStatementKind::Loop { id, body }
            }

            StatementKind::Break => {
                let context = self
                    .loops
                    .last()
                    .copied()
                    .ok_or(SemanticError::BreakOutsideLoop { span })?;

                if context.kind == LoopKind::Value {
                    return Err(SemanticError::MissingBreakValue { span });
                }

                flow = Flow::BREAK;

                TypedStatementKind::Break { target: context.id }
            }

            StatementKind::BreakValue(value) => {
                let context = self
                    .loops
                    .last()
                    .copied()
                    .ok_or(SemanticError::BreakOutsideLoop { span })?;

                if context.kind == LoopKind::Statement {
                    return Err(SemanticError::BreakValueNotAllowed { span });
                }

                let (value, value_flow) = self.analyze_expr(value, span, return_type)?;

                flow = match value.ty {
                    ExprResult::Value(ty) => value_flow.then(Flow::value_break(ty)),
                    ExprResult::Never => value_flow,
                    ExprResult::Void => return Err(SemanticError::InvalidBreakValue { span }),
                };

                TypedStatementKind::BreakValue {
                    target: context.id,
                    value,
                }
            }

            StatementKind::Continue => {
                let target = self
                    .loops
                    .last()
                    .copied()
                    .ok_or(SemanticError::ContinueOutsideLoop { span })?;

                flow = Flow::CONTINUE;

                TypedStatementKind::Continue { target: target.id }
            }

            StatementKind::Return(value) => {
                let value = match value {
                    Some(_) if return_type == Type::Void => {
                        return Err(SemanticError::UnexpectedReturnValue { span });
                    }

                    Some(expr) => {
                        let (expr, expr_flow) = self.analyze_expr(expr, span, return_type)?;

                        self.expect_type(return_type, expr.ty, span)?;
                        flow = expr_flow;

                        Some(expr)
                    }

                    None => {
                        self.expect_type(return_type, Type::Void.into(), span)?;

                        None
                    }
                };

                flow = flow.then(Flow::RETURN);

                TypedStatementKind::Return(value)
            }

            StatementKind::Expression(expr) => {
                let (expr, expr_flow) = self.analyze_expr(expr, span, return_type)?;

                flow = expr_flow;

                TypedStatementKind::Expression(expr)
            }
        };

        Ok((TypedStatement { kind, span }, flow))
    }

    fn analyze_loop_body(
        &mut self,
        body: &Block,
        return_type: Type,
        kind: LoopKind,
    ) -> Result<(LoopId, TypedBlock, Flow), SemanticError> {
        let id = LoopId(self.next_loop_id);

        self.next_loop_id += 1;
        self.loops.push(LoopContext { id, kind });

        let result = self.analyze_scoped_block(body, return_type);

        self.loops.pop();

        let (body, flow) = result?;

        Ok((id, body, flow))
    }

    fn analyze_assignment_target(
        &mut self,
        target: &Expr,
        span: Span,
        return_type: Type,
    ) -> Result<(TypedExpr, Flow), SemanticError> {
        match target {
            Expr::Identifier(_) => self.analyze_expr(target, span, return_type),
            Expr::Index { expr, .. } if matches!(**expr, Expr::Identifier(_)) => {
                self.analyze_expr(target, span, return_type)
            }

            _ => Err(SemanticError::InvalidAssignmentTarget { span }),
        }
    }

    fn analyze_value_block(
        &mut self,
        block: &ValueBlock,
        return_type: Type,
    ) -> Result<(TypedValueBlock, ExprResult, Flow), SemanticError> {
        self.scopes.push(HashMap::new());

        let result = (|| {
            let (statements, mut flow) = self.analyze_block(&block.statements, return_type)?;
            let (tail, ty) = match &block.tail {
                Some(expr) => {
                    let (expr, tail_flow) = self.analyze_expr(expr, block.span, return_type)?;

                    flow = flow.then(tail_flow);

                    let ty = expr.ty;

                    (Some(Box::new(expr)), ty)
                }
                None => (None, ExprResult::Void),
            };
            let ty = Self::completion_result(ty, flow);

            Ok((
                TypedValueBlock {
                    statements,
                    tail,
                    span: block.span,
                },
                ty,
                flow,
            ))
        })();

        self.scopes.pop();

        result
    }

    fn completion_result(ty: ExprResult, flow: Flow) -> ExprResult {
        if flow.can_fall_through() {
            ty
        } else {
            ExprResult::Never
        }
    }

    fn join_results(
        &self,
        then: ExprResult,
        else_: ExprResult,
        span: Span,
    ) -> Result<ExprResult, SemanticError> {
        match (then, else_) {
            (ExprResult::Never, ty) | (ty, ExprResult::Never) => Ok(ty),
            (left, right) if left == right => Ok(left),
            (left, right) => {
                let expected = left.value_type().unwrap_or(Type::Void);

                self.expect_type(expected, right, span)?;

                Ok(left)
            }
        }
    }

    fn analyze_expr(
        &mut self,
        expr: &Expr,
        span: Span,
        return_type: Type,
    ) -> Result<(TypedExpr, Flow), SemanticError> {
        let mut flow = Flow::FALLTHROUGH;
        let (kind, ty) = match expr {
            Expr::Number(value) => (TypedExprKind::Number(value.clone()), Type::Number.into()),
            Expr::String(value) => (TypedExprKind::String(value.clone()), Type::String.into()),
            Expr::Bool(value) => (TypedExprKind::Bool(*value), Type::Bool.into()),

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
                    binding.ty.into(),
                )
            }

            Expr::Loop { body } => {
                let (id, body, body_flow) =
                    self.analyze_loop_body(body, return_type, LoopKind::Value)?;
                let ty = body_flow.break_type().map_err(|(expected, found)| {
                    SemanticError::TypeMismatch {
                        expected,
                        found,
                        span,
                    }
                })?;

                flow = body_flow.infinite_loop();

                (
                    TypedExprKind::Loop { id, body },
                    ty.map_or(ExprResult::Never, ExprResult::Value),
                )
            }

            Expr::Block(block) => {
                let (block, ty, block_flow) = self.analyze_value_block(block, return_type)?;

                flow = block_flow;

                (TypedExprKind::Block(block), ty)
            }

            Expr::If { cond, then, else_ } => {
                let (cond, cond_flow) = self.analyze_expr(cond, span, return_type)?;

                self.expect_type(Type::Bool, cond.ty, span)?;

                let (then, then_ty, then_flow) = self.analyze_value_block(then, return_type)?;
                let else_ = else_.as_ref().ok_or(SemanticError::MissingElse { span })?;
                let (else_, else_ty, else_flow) = self.analyze_value_block(else_, return_type)?;
                let ty = self.join_results(then_ty, else_ty, span)?;

                flow = cond_flow.then(then_flow.join(else_flow));

                (
                    TypedExprKind::If {
                        cond: Box::new(cond),
                        then,
                        else_: Some(else_),
                    },
                    ty,
                )
            }

            Expr::Unary { op, expr } => {
                let (expr, expr_flow) = self.analyze_expr(expr, span, return_type)?;
                let ty = match op {
                    UnaryOp::Not => Type::Bool,
                    UnaryOp::Negate => Type::Number,
                };

                self.expect_type(ty, expr.ty, span)?;
                flow = expr_flow;

                (
                    TypedExprKind::Unary {
                        op: *op,
                        expr: Box::new(expr),
                    },
                    ty.into(),
                )
            }

            Expr::Binary { left, op, right } => {
                let (left, left_flow) = self.analyze_expr(left, span, return_type)?;
                let (right, right_flow) = self.analyze_expr(right, span, return_type)?;
                let ty = self.analyze_binary(op, left.ty, right.ty, span)?;

                flow = if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    left_flow.then(Flow::FALLTHROUGH.join(right_flow.without_fallthrough()))
                } else {
                    left_flow.then(right_flow)
                };

                (
                    TypedExprKind::Binary {
                        left: Box::new(left),
                        op: *op,
                        right: Box::new(right),
                    },
                    ty.into(),
                )
            }

            Expr::Range {
                start,
                end,
                step,
                inclusive,
            } => {
                let (start, start_flow) = self.analyze_expr(start, span, return_type)?;

                self.expect_type(Type::Number, start.ty, span)?;

                let (end, end_flow) = self.analyze_expr(end, span, return_type)?;

                self.expect_type(Type::Number, end.ty, span)?;
                flow = start_flow.then(end_flow);

                let step = match step {
                    Some(step) => {
                        let (step, step_flow) = self.analyze_expr(step, span, return_type)?;

                        self.expect_type(Type::Number, step.ty, span)?;
                        flow = flow.then(step_flow);

                        Some(Box::new(step))
                    }
                    None => None,
                };

                (
                    TypedExprKind::Range {
                        start: Box::new(start),
                        end: Box::new(end),
                        step,
                        inclusive: *inclusive,
                    },
                    Type::Range.into(),
                )
            }

            Expr::Call { func, args } => return self.analyze_call(func, args, span, return_type),

            Expr::Array(elements) => {
                let mut values = Vec::with_capacity(elements.len());

                for element in elements {
                    let (value, value_flow) = self.analyze_expr(element, span, return_type)?;

                    self.expect_type(Type::Number, value.ty, span)?;
                    flow = flow.then(value_flow);
                    values.push(value);
                }

                (TypedExprKind::Array(values), Type::NumberArray.into())
            }

            Expr::Index { expr, index } => {
                let (expr, expr_flow) = self.analyze_expr(expr, span, return_type)?;

                self.expect_type(Type::NumberArray, expr.ty, span)?;

                let (index, index_flow) = self.analyze_expr(index, span, return_type)?;

                self.expect_type(Type::Number, index.ty, span)?;
                flow = expr_flow.then(index_flow);

                (
                    TypedExprKind::Index {
                        expr: Box::new(expr),
                        index: Box::new(index),
                    },
                    Type::Number.into(),
                )
            }
        };
        let ty = Self::completion_result(ty, flow);

        Ok((TypedExpr { kind, ty }, flow))
    }

    fn analyze_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        return_type: Type,
    ) -> Result<(TypedExpr, Flow), SemanticError> {
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

        let reference = signature.reference;
        let result_type = signature.return_type;
        let mut arguments = Vec::with_capacity(args.len());
        let mut flow = Flow::FALLTHROUGH;

        for (index, arg) in args.iter().enumerate() {
            // Expression analysis may introduce scoped bindings, so release the
            // signature borrow before analyzing each argument.
            let expected = self.functions[name].params[index];
            let (arg, arg_flow) = self.analyze_expr(arg, span, return_type)?;

            self.expect_type(expected, arg.ty, span)?;
            flow = flow.then(arg_flow);
            arguments.push(arg);
        }

        Ok((
            TypedExpr {
                kind: TypedExprKind::Call {
                    func: reference,
                    name: name.into(),
                    args: arguments,
                },
                ty: Self::completion_result(result_type.into(), flow),
            },
            flow,
        ))
    }

    fn analyze_binary(
        &self,
        op: &BinaryOp,
        left: ExprResult,
        right: ExprResult,
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

            BinaryOp::In => {
                if Self::accepts_operand(left, Type::Number)
                    && Self::accepts_operand(right, Type::Range)
                {
                    return Ok(Type::Bool);
                }

                return Err(SemanticError::InvalidBinaryOperands {
                    operator: "in",
                    left: Self::diagnostic_operand(left, Type::Number),
                    right: Self::diagnostic_operand(right, Type::Range),
                    span,
                });
            }

            BinaryOp::And => ("&&", Type::Bool, Type::Bool),
            BinaryOp::Or => ("||", Type::Bool, Type::Bool),

            BinaryOp::Equal | BinaryOp::NotEqual => {
                let operator = if *op == BinaryOp::Equal { "==" } else { "!=" };

                let operand_type = left
                    .value_type()
                    .or(right.value_type())
                    .unwrap_or(Type::Number);

                if matches!(
                    operand_type,
                    Type::Number | Type::Bool | Type::String | Type::NumberArray
                ) && Self::accepts_operand(left, operand_type)
                    && Self::accepts_operand(right, operand_type)
                {
                    return Ok(Type::Bool);
                }

                return Err(SemanticError::InvalidBinaryOperands {
                    operator,
                    left: Self::diagnostic_operand(left, operand_type),
                    right: Self::diagnostic_operand(right, operand_type),
                    span,
                });
            }
        };

        if !Self::accepts_operand(left, operand_type) || !Self::accepts_operand(right, operand_type)
        {
            return Err(SemanticError::InvalidBinaryOperands {
                operator,
                left: Self::diagnostic_operand(left, operand_type),
                right: Self::diagnostic_operand(right, operand_type),
                span,
            });
        }

        Ok(result_type)
    }

    fn accepts_operand(result: ExprResult, expected: Type) -> bool {
        result == ExprResult::Never || result == expected
    }

    fn diagnostic_operand(result: ExprResult, expected: Type) -> Type {
        match result {
            ExprResult::Value(ty) => ty,
            ExprResult::Void => Type::Void,
            ExprResult::Never => expected,
        }
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

    fn expect_type(
        &self,
        expected: Type,
        found: ExprResult,
        span: Span,
    ) -> Result<(), SemanticError> {
        let found = match found {
            ExprResult::Never => return Ok(()),
            ExprResult::Void => Type::Void,
            ExprResult::Value(ty) => ty,
        };

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
