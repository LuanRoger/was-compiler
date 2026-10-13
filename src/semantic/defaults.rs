use super::pure_expression::PureExpressionContext;
use super::*;

pub(super) struct CheckedDefault {
    pub(super) value: TypedExpr,
    pub(super) span: Span,
}

impl SemanticAnalyzer {
    pub(super) fn validate_declaration_expressions(
        &mut self,
        function: &Function,
    ) -> Result<(), SemanticError> {
        // Declaration lookup must not inherit any caller/body bindings. Formal IDs
        // are template identities, independent of the next body's allocator.
        let scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        let next_variable_id = std::mem::replace(&mut self.next_variable_id, 0);
        let result = self.check_declaration_expressions(function);

        self.scopes = scopes;
        self.next_variable_id = next_variable_id;

        let (defaults, contracts) = result?;
        let signature = self
            .functions
            .get_mut(&function.name)
            .expect("function signatures precede declaration validation");
        signature.defaults = defaults;
        signature.contracts = contracts;

        Ok(())
    }

    fn check_declaration_expressions(
        &mut self,
        function: &Function,
    ) -> Result<(Vec<Option<CheckedDefault>>, Vec<TypedContract>), SemanticError> {
        let mut defaults = Vec::with_capacity(function.params.len());
        let mut saw_default = false;

        for param in &function.params {
            self.check_variable_type(&param.name, param.ty, param.span)?;

            let default = if let Some(default) = &param.default {
                saw_default = true;
                self.check_pure_expression(
                    &default.value,
                    default.span,
                    PureExpressionContext::Default,
                )?;
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

        let mut contracts = Vec::with_capacity(function.contracts.len());

        // The whitelist cannot allocate bindings. These formal-order identities
        // therefore match the parameters allocated by final body analysis.
        for contract in &function.contracts {
            self.check_pure_expression(
                &contract.value,
                contract.span,
                PureExpressionContext::Contract,
            )?;
            let (value, _) = self.analyze_expr(
                &contract.value,
                contract.span,
                ReturnContext::Known(Type::Void),
            )?;
            self.expect_type(Type::Bool, value.ty, contract.span)?;
            contracts.push(TypedContract {
                value,
                span: contract.span,
            });
        }

        Ok((defaults, contracts))
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
        | TypedExprKind::Match { .. }
        | TypedExprKind::Array(_)
        | TypedExprKind::Index { .. } => {
            unreachable!("private default templates pass the whitelist before expression checking")
        }
    };

    TypedExpr { kind, ty: value.ty }
}
