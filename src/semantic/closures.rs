use super::*;

#[derive(Clone)]
pub(super) struct CallableBinding {
    id: CallableId,
    params: Vec<Type>,
    return_type: Type,
}

#[derive(Clone, Copy)]
pub(super) enum BindingUse {
    Read,
    Write,
}

#[derive(Clone)]
pub(super) enum LexicalBinding {
    Value(VariableBinding),
    Callable(CallableBinding),
}

impl SemanticAnalyzer {
    pub(super) fn lookup_binding(&self, name: &str) -> Option<&LexicalBinding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    pub(super) fn check_value_binding(
        &self,
        name: &str,
        span: Span,
        usage: BindingUse,
    ) -> Result<(), SemanticError> {
        match self.lookup_binding(name) {
            Some(LexicalBinding::Callable(_)) => Err(SemanticError::InvalidClosure {
                reason: match usage {
                    BindingUse::Write => "a local callable binding cannot be reassigned",
                    BindingUse::Read => {
                        "a local callable binding cannot be used as a value; invoke it directly or use a named function"
                    }
                },
                span,
            }),
            Some(LexicalBinding::Value(binding)) if binding.owner != self.owner => {
                Err(SemanticError::InvalidClosure {
                    reason: "capturing outer bindings is not supported in this subset",
                    span,
                })
            }
            _ => Ok(()),
        }
    }

    pub(super) fn declare_callable(
        &mut self,
        closure: &TypedClosure,
        span: Span,
    ) -> Result<(), SemanticError> {
        let scope = self
            .scopes
            .last_mut()
            .expect("callables require a lexical scope");
        if scope.contains_key(&closure.name) {
            return Err(SemanticError::DuplicateVariable {
                name: closure.name.clone(),
                span,
            });
        }
        scope.insert(
            closure.name.clone(),
            LexicalBinding::Callable(CallableBinding {
                id: closure.id,
                params: closure.params.iter().map(|param| param.ty).collect(),
                return_type: closure.return_type,
            }),
        );
        Ok(())
    }

    pub(super) fn analyze_lexical_call(
        &mut self,
        callable: &CallableBinding,
        args: &[CallArgument],
        span: Span,
        context: ReturnContext,
    ) -> Result<(TypedExpr, Flow), SemanticError> {
        if let Some(arg) = args.iter().find(|arg| arg.name.is_some()) {
            return Err(SemanticError::NamedArgumentsNotSupported {
                name: "<closure>".into(),
                span: arg.span,
            });
        }
        if args.len() != callable.params.len() {
            return Err(SemanticError::ArgumentCountMismatch {
                name: "<closure>".into(),
                expected: callable.params.len(),
                found: args.len(),
                span,
            });
        }
        // Nested declarations preserve earlier external dependencies in every
        // intervening helper, but not dependencies on that helper's own children.
        for (owner, dependencies) in &mut self.dependencies {
            if callable.id.0 < owner.0 && !dependencies.contains(&callable.id) {
                dependencies.push(callable.id);
            }
        }
        let mut arguments = Vec::with_capacity(args.len());
        let mut flow = Flow::FALLTHROUGH;
        for (arg, expected) in args.iter().zip(&callable.params) {
            let (value, next) = self.analyze_expr(&arg.value, arg.span, context)?;
            self.expect_type(*expected, value.ty, arg.span)?;
            flow = flow.then(next);
            arguments.push(value);
        }
        Ok((
            TypedExpr {
                kind: TypedExprKind::LexicalCall {
                    callable: callable.id,
                    args: arguments,
                    span,
                },
                ty: Self::completion_result(callable.return_type.into(), flow),
            },
            flow,
        ))
    }

    pub(super) fn analyze_closure(
        &mut self,
        name: &str,
        name_span: Span,
        lambda: &Lambda,
    ) -> Result<TypedClosure, SemanticError> {
        let identity = std::ptr::from_ref(lambda).addr();
        let id = self.callable_ids[&identity];
        let owner = self.owner.expect("closure declarations have an owner");
        let next_variable = self.next_variable_id;
        let next_loop = self.next_loop_id;
        let loops = std::mem::take(&mut self.loops);
        self.owner = Some(BindingOwner::Closure(id));
        self.dependencies.push((id, Vec::new()));

        let result = (|| {
            // Named dependency scheduling supplies concrete signatures before
            // inference. Cache only the result: every visit still checks the body
            // in its current lexical context and reconstructs dependencies.
            let return_type = match self.closure_results.get(&identity).copied() {
                Some(ty) => ty,
                None => self.infer_closure_result(lambda)?,
            };
            let (params, body, _, _) =
                self.analyze_closure_body(lambda, ReturnContext::Known(return_type))?;
            self.closure_results.insert(identity, return_type);
            Ok((params, body, return_type))
        })();

        let (_, mut dependencies) = self
            .dependencies
            .pop()
            .expect("closure dependency context is active");
        dependencies.sort_unstable_by_key(|id| id.0);
        self.owner = Some(owner);
        self.next_variable_id = next_variable;
        self.next_loop_id = next_loop;
        self.loops = loops;
        let (params, body, return_type) = result?;
        Ok(TypedClosure {
            id,
            owner,
            name: name.into(),
            name_span,
            params,
            return_type,
            body,
            captures: Vec::new(),
            dependencies,
            span: lambda.span,
            body_span: lambda.body_span,
        })
    }

    fn infer_closure_result(&mut self, lambda: &Lambda) -> Result<Type, SemanticError> {
        let (_, _, result, flow) = self.analyze_closure_body(lambda, ReturnContext::Infer)?;
        let returned = flow.return_type().map_err(|(expected, found)| {
            SemanticError::IncompatibleInferredReturns {
                expected,
                found,
                span: lambda.body_span,
            }
        })?;
        let has_value_return = inference::closure_has_value_return(lambda);
        let no_tail = matches!(&lambda.body, LambdaBody::Block(block) if block.tail.is_none());
        if no_tail && flow.can_fall_through() && returned.is_some_and(|ty| ty != Type::Void) {
            return Err(SemanticError::MissingReturn {
                name: "<closure>".into(),
                span: lambda.body_span,
            });
        }
        let result = if let Some(returned) = returned {
            self.join_results(returned.into(), result, lambda.body_span)?
        } else {
            result
        };
        match result {
            ExprResult::Value(ty) => Ok(ty),
            ExprResult::Void if !has_value_return => Ok(Type::Void),
            ExprResult::Never if no_tail && !has_value_return => Ok(Type::Void),
            _ => Err(SemanticError::CannotInferClosureResult {
                span: lambda.body_span,
            }),
        }
    }

    fn analyze_closure_body(
        &mut self,
        lambda: &Lambda,
        context: ReturnContext,
    ) -> Result<(Vec<TypedParam>, TypedClosureBody, ExprResult, Flow), SemanticError> {
        self.next_variable_id = 0;
        self.next_loop_id = 0;
        self.scopes.push(HashMap::new());
        let result = (|| {
            let mut params = Vec::with_capacity(lambda.params.len());
            for param in &lambda.params {
                self.check_variable_type(&param.name, param.ty, param.span)?;
                if param.default.is_some() {
                    return Err(SemanticError::InvalidClosure {
                        reason: "closure parameters cannot have defaults",
                        span: param.span,
                    });
                }
                let id = self.declare_variable(
                    &param.name,
                    param.ty,
                    BindingMutability::Mutable,
                    param.span,
                )?;
                params.push(TypedParam {
                    id,
                    name: param.name.clone(),
                    ty: param.ty,
                    span: param.span,
                });
            }
            let (body, ty, flow) = match &lambda.body {
                LambdaBody::Expression(expr) => {
                    let (expr, flow) = self.analyze_expr(expr, lambda.body_span, context)?;
                    if let ReturnContext::Known(expected) = context {
                        self.expect_type(expected, expr.ty, lambda.body_span)?;
                    }
                    let ty = expr.ty;
                    (TypedClosureBody::Expression(Box::new(expr)), ty, flow)
                }
                LambdaBody::Block(block) => {
                    let (body, ty, flow) =
                        self.analyze_value_block(block, context, ExprContext::Value)?;
                    if let (ReturnContext::Known(expected), Some(tail)) = (context, &body.tail) {
                        self.expect_type(expected, tail.ty, lambda.body_span)?;
                    }
                    (TypedClosureBody::Block(body), ty, flow)
                }
            };
            Ok((params, body, ty, flow))
        })();
        self.scopes.pop();
        self.loops.clear();
        result
    }
}
