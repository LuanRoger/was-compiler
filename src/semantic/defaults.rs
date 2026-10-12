use super::*;
use crate::number_literal::{self, LiteralError};

pub(super) struct CheckedDefault {
    pub(super) value: TypedExpr,
    pub(super) span: Span,
}

impl SemanticAnalyzer {
    pub(super) fn validate_defaults(&mut self, function: &Function) -> Result<(), SemanticError> {
        // Declaration lookup must not inherit any caller/body bindings. Formal IDs
        // are template identities, independent of the next body's allocator.
        let scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        let next_variable_id = std::mem::replace(&mut self.next_variable_id, 0);
        let result = self.check_function_defaults(function);

        self.scopes = scopes;
        self.next_variable_id = next_variable_id;

        let defaults = result?;
        self.functions
            .get_mut(&function.name)
            .expect("function signatures precede default validation")
            .defaults = defaults;

        Ok(())
    }

    fn check_function_defaults(
        &mut self,
        function: &Function,
    ) -> Result<Vec<Option<CheckedDefault>>, SemanticError> {
        let mut defaults = Vec::with_capacity(function.params.len());
        let mut saw_default = false;

        for param in &function.params {
            self.check_variable_type(&param.name, param.ty, param.span)?;

            let default = if let Some(default) = &param.default {
                saw_default = true;
                self.check_default_expression(&default.value, default.span)?;
                let (value, _) = self.analyze_expr(
                    &default.value,
                    default.span,
                    ReturnContext::Known(Type::Void),
                )?;

                self.expect_type(param.ty, value.ty, default.span)?;

                Some(CheckedDefault {
                    value,
                    span: default.span,
                })
            } else {
                if saw_default {
                    return Err(SemanticError::NonTrailingDefault {
                        name: param.name.clone(),
                        span: param.span,
                    });
                }

                None
            };

            // Insert only after validation: self/later references are not in scope.
            self.declare_variable(
                &param.name,
                param.ty,
                BindingMutability::ReadOnly,
                param.span,
            )?;
            defaults.push(default);
        }

        Ok(defaults)
    }

    fn check_default_expression(&self, expr: &Expr, span: Span) -> Result<(), SemanticError> {
        let invalid = |reason| SemanticError::InvalidDefaultExpression { reason, span };

        match expr {
            Expr::Number(literal) => {
                number_literal::parse(literal).map_err(|error| match error {
                    LiteralError::Invalid => SemanticError::InvalidDefaultNumberLiteral {
                        literal: literal.clone(),
                        span,
                    },
                    LiteralError::OutOfRange => SemanticError::DefaultNumberLiteralOutOfRange {
                        literal: literal.clone(),
                        span,
                    },
                })?;
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

                self.check_default_expression(expr, span)?;
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

                self.check_default_expression(left, span)?;
                self.check_default_expression(right, span)?;
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
                    self.check_default_expression(&arg.value, span)?;
                }
            }
            Expr::Array(_) => return Err(invalid("array construction is not allowed")),
            Expr::Range { .. } => return Err(invalid("range construction is not allowed")),
            Expr::Index { .. } => return Err(invalid("indexing is not allowed")),
            Expr::Block(_) => return Err(invalid("blocks are not allowed")),
            Expr::If { .. } => return Err(invalid("if expressions are not allowed")),
            Expr::Loop { .. } => return Err(invalid("loop expressions are not allowed")),
        }

        Ok(())
    }
}

impl CheckedDefault {
    pub(super) fn instantiate(&self, mapping: &[Option<VariableId>]) -> TypedExpr {
        remap_default(&self.value, mapping)
    }
}

fn remap_default(value: &TypedExpr, mapping: &[Option<VariableId>]) -> TypedExpr {
    let kind = match &value.kind {
        TypedExprKind::Number(value) => TypedExprKind::Number(value.clone()),
        TypedExprKind::String(value) => TypedExprKind::String(value.clone()),
        TypedExprKind::Bool(value) => TypedExprKind::Bool(*value),
        TypedExprKind::Identifier { id, name } => TypedExprKind::Identifier {
            id: mapping[id.0].expect("earlier formals are bound before default instantiation"),
            name: name.clone(),
        },
        TypedExprKind::Unary { op, expr } => TypedExprKind::Unary {
            op: *op,
            expr: Box::new(remap_default(expr, mapping)),
        },
        TypedExprKind::Binary { left, op, right } => TypedExprKind::Binary {
            left: Box::new(remap_default(left, mapping)),
            op: *op,
            right: Box::new(remap_default(right, mapping)),
        },
        TypedExprKind::Call {
            func: FunctionRef::Builtin(BuiltinFunction::Len),
            name,
            args,
        } => TypedExprKind::Call {
            func: FunctionRef::Builtin(BuiltinFunction::Len),
            name: name.clone(),
            args: args.iter().map(|arg| remap_default(arg, mapping)).collect(),
        },
        TypedExprKind::Call { .. }
        | TypedExprKind::NormalizedCall(_)
        | TypedExprKind::Range { .. }
        | TypedExprKind::Loop { .. }
        | TypedExprKind::Block(_)
        | TypedExprKind::If { .. }
        | TypedExprKind::Array(_)
        | TypedExprKind::Index { .. } => {
            unreachable!("private default templates pass the whitelist before expression checking")
        }
    };

    TypedExpr { kind, ty: value.ty }
}
