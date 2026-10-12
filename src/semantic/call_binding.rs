use super::*;

impl SemanticAnalyzer {
    pub(super) fn analyze_bound_call(
        &mut self,
        name: &str,
        args: &[CallArgument],
        span: Span,
        return_type: Type,
    ) -> Result<(TypedExpr, Flow), SemanticError> {
        let signature = &self.functions[name];

        if !matches!(signature.reference, FunctionRef::User(_)) {
            return Err(SemanticError::NamedArgumentsNotSupported {
                name: name.into(),
                span: args
                    .iter()
                    .find(|arg| arg.name.is_some())
                    .expect("non-user bound calls require a named argument")
                    .span,
            });
        }

        let mut written_to_formal = Vec::with_capacity(args.len());
        let mut supplied = vec![false; signature.params.len()];
        let mut named = false;
        let mut positional = 0;

        for arg in args {
            let formal = if let Some(argument) = &arg.name {
                named = true;
                signature
                    .param_names
                    .iter()
                    .position(|formal| formal == argument)
                    .ok_or_else(|| SemanticError::UnknownArgument {
                        name: name.into(),
                        argument: argument.clone(),
                        span: arg.span,
                    })?
            } else {
                if named {
                    return Err(SemanticError::PositionalAfterNamed {
                        name: name.into(),
                        span: arg.span,
                    });
                }

                if positional >= supplied.len() {
                    return Err(SemanticError::TooManyPositionalArguments {
                        name: name.into(),
                        span: arg.span,
                    });
                }

                let formal = positional;
                positional += 1;
                formal
            };

            if supplied[formal] {
                return Err(SemanticError::DuplicateArgument {
                    name: name.into(),
                    argument: signature.param_names[formal].clone(),
                    span: arg.span,
                });
            }

            supplied[formal] = true;
            written_to_formal.push(formal);
        }

        if let Some(formal) = supplied
            .iter()
            .enumerate()
            .position(|(formal, supplied)| !supplied && signature.defaults[formal].is_none())
        {
            return Err(SemanticError::MissingArgument {
                name: name.into(),
                argument: signature.param_names[formal].clone(),
                span,
            });
        }

        let reference = signature.reference;
        let result_type = signature.return_type;
        let formal_count = signature.params.len();
        let mut bindings = Vec::with_capacity(formal_count);
        let mut mapping = vec![None; formal_count];
        let mut flow = Flow::FALLTHROUGH;

        for (argument, (arg, formal)) in args.iter().zip(written_to_formal).enumerate() {
            let expected = self.functions[name].params[formal];
            let id = VariableId(self.next_variable_id);
            self.next_variable_id += 1;
            let (value, value_flow) = self.analyze_expr(&arg.value, arg.span, return_type)?;

            self.expect_type(expected, value.ty, arg.span)?;
            flow = flow.then(value_flow);
            bindings.push(TypedCallBinding {
                id,
                ty: expected,
                value,
                origin: CallBindingOrigin::Explicit { argument },
                span: arg.span,
            });
            mapping[formal] = Some(id);
        }

        for formal in 0..formal_count {
            if supplied[formal] {
                continue;
            }

            let signature = &self.functions[name];
            let default = signature.defaults[formal]
                .as_ref()
                .expect("structural binding checked every required formal");
            let value = default.instantiate(&mapping);
            let id = VariableId(self.next_variable_id);
            self.next_variable_id += 1;
            bindings.push(TypedCallBinding {
                id,
                ty: signature.params[formal],
                value,
                origin: CallBindingOrigin::Default { formal },
                span: default.span,
            });
            mapping[formal] = Some(id);
        }

        let signature = &self.functions[name];
        let formal_to_binding: Vec<_> = mapping
            .into_iter()
            .map(|id| id.expect("every formal has an explicit or default binding"))
            .collect();
        let final_args = formal_to_binding
            .iter()
            .enumerate()
            .map(|(formal, &id)| TypedExpr {
                kind: TypedExprKind::Identifier {
                    id,
                    name: signature.param_names[formal].clone(),
                },
                ty: signature.params[formal].into(),
            })
            .collect();
        let call = TypedExpr {
            kind: TypedExprKind::Call {
                func: reference,
                name: name.into(),
                args: final_args,
            },
            ty: result_type.into(),
        };

        Ok((
            TypedExpr {
                kind: TypedExprKind::NormalizedCall(TypedNormalizedCall {
                    bindings,
                    formal_to_binding,
                    call: Box::new(call),
                }),
                ty: Self::completion_result(result_type.into(), flow),
            },
            flow,
        ))
    }
}
