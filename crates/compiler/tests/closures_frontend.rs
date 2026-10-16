use was_compiler::compiler::CompileError;
use was_compiler::semantic::SemanticError;
use was_compiler::typed_ast::TypedStatementKind;
use was_compiler::{Compiler, Type};

fn closure(
    program: &was_compiler::typed_ast::TypedProgram,
    function: usize,
    statement: usize,
) -> &was_compiler::typed_ast::TypedClosure {
    let TypedStatementKind::Closure(closure) = &program.functions[function].body[statement].kind
    else {
        panic!("expected closure");
    };
    closure
}

#[test]
fn deeply_nested_context_free_declarations_preserve_results_owners_and_dependencies() {
    use std::time::{Duration, Instant};
    use was_compiler::typed_ast::{BindingOwner, CallableId, FunctionId, TypedClosureBody};

    let depth = 16;
    let mut body = "base()".to_owned();
    for level in (0..depth).rev() {
        body = format!("{{ let nested{level} = () => {body}; nested{level}() }}");
    }
    let source =
        format!("fn main() {{ let base = () => 7; let root = () => {body}; return root(); }}");
    let start = Instant::now();
    let program = Compiler::new().analyze(&source).unwrap();
    let elapsed = start.elapsed();
    eprintln!("nested closure analysis depth {depth}: {elapsed:?}");
    // Catch repeated whole-body inference with ample headroom for ordinary checks.
    assert!(
        elapsed < Duration::from_secs(2),
        "bounded nested analysis took {elapsed:?}"
    );
    assert_eq!(program.functions[0].return_type, Type::Number);

    let mut current = closure(&program, 0, 1);
    let mut owner = BindingOwner::Function(FunctionId(0));
    for index in 1..=depth + 1 {
        assert_eq!(current.id, CallableId(index));
        assert_eq!(current.owner, owner);
        assert_eq!(current.return_type, Type::Number);
        assert!(current.captures.is_empty());
        assert_eq!(current.dependencies, vec![CallableId(0)]);
        if index <= depth {
            let TypedClosureBody::Block(block) = &current.body else {
                panic!("nested block");
            };
            let TypedStatementKind::Closure(nested) = &block.statements[0].kind else {
                panic!("nested closure");
            };
            owner = BindingOwner::Closure(current.id);
            current = nested;
        }
    }
}

#[test]
fn nested_inference_compound_checks_preserve_captures_and_repeated_analysis() {
    let compiler = Compiler::new();
    let mut body = "{ let x = 1; x += 2; x *= 3; let capture = () => x; capture() }".to_owned();
    for level in (0..12).rev() {
        // Repeated checks revisit distinct source compounds with colliding
        // owner-local IDs; none of these ancestor bindings is captured.
        body = format!(
            "{{ let x = 0; x += 1; let nested{level} = () => {body}; x *= 2; nested{level}() }}"
        );
    }
    let source =
        format!("fn main() {{ let x = 0; x += 7; let root = () => {body}; return root(); }}");
    let expected_program = compiler.analyze(&source).unwrap();
    assert_eq!(expected_program.functions[0].return_type, Type::Number);

    // A capture found before nested inference must survive every helper visit.
    let earlier = source.replacen("let x = 0;", "let x = 0; let capture = () => x;", 1);
    let earlier_program = compiler.analyze(&earlier).unwrap();
    assert_eq!(closure(&earlier_program, 0, 1).captures.len(), 1);
    for _ in 0..3 {
        assert_eq!(compiler.analyze(&source).unwrap(), expected_program);
        assert_eq!(compiler.analyze(&earlier).unwrap(), earlier_program);
    }
}

#[test]
fn repeated_closure_checks_preserve_initial_error_order_and_initializer_visibility() {
    use was_compiler::tokens::Span;
    use was_compiler::typed_ast::{CallableId, TypedClosureBody, TypedExprKind};

    let source = "fn main() { let f = () => { return 1; return true; missing; }; }";
    let start = source.find("missing;").unwrap();
    assert_eq!(
        semantic_error(source),
        SemanticError::UndefinedVariable {
            name: "missing".into(),
            span: Span::new(start, start + "missing;".len()),
        }
    );

    let compiler = Compiler::new();
    for (value, ty) in [("7", Type::Number), ("true", Type::Bool)] {
        let source = format!(
            "fn first() {{ let f = () => later(); let root = () => {{ let f = () => f(); f() }}; return root(); }} fn later() {{ return {value}; }}"
        );
        let program = compiler.analyze(&source).unwrap();
        assert_eq!(program.functions[0].return_type, ty);
        let root = closure(&program, 0, 1);
        assert_eq!(root.return_type, ty);
        assert_eq!(root.dependencies, vec![CallableId(0)]);
        let TypedClosureBody::Block(body) = &root.body else {
            panic!("block");
        };
        let TypedStatementKind::Closure(inner) = &body.statements[0].kind else {
            panic!("closure");
        };
        assert_eq!(inner.return_type, ty);
        assert_eq!(inner.dependencies, vec![CallableId(0)]);
        let TypedClosureBody::Expression(call) = &inner.body else {
            panic!("expression");
        };
        assert!(matches!(
            call.kind,
            TypedExprKind::LexicalCall {
                callable: CallableId(0),
                ..
            }
        ));
    }
}

#[test]
fn expression_and_block_results_cover_every_concrete_type_and_void() {
    for (body, ty) in [
        ("1", Type::Number),
        ("true", Type::Bool),
        ("\"é\"", Type::String),
        (
            "[1, 2]",
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
        ("1..3", Type::Range),
        ("{}", Type::Void),
        ("{ loop {} }", Type::Void),
        ("{ return; }", Type::Void),
        ("{ if true { return 1; } 2 }", Type::Number),
        ("{ if true { return true; } false }", Type::Bool),
        (
            "loop { break [1]; }",
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
    ] {
        let source = format!("fn main() {{ let f = () => {body}; }}");
        let program = Compiler::new()
            .analyze(&source)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(closure(&program, 0, 0).return_type, ty, "{source}");
        assert!(closure(&program, 0, 0).captures.is_empty());
    }
}

#[test]
fn inferable_results_require_exact_reachable_joins_and_complete_value_paths() {
    for body in ["loop {}", "{ return loop {}; }"] {
        let source = format!("fn main() {{ let f = () => {body}; }}");
        assert!(
            matches!(
                semantic_error(&source),
                SemanticError::CannotInferClosureResult { .. }
            ),
            "{source}"
        );
    }
    assert!(
        matches!(semantic_error("fn main() { let f = () => { if true { return 1; } let x = 2; }; }"), SemanticError::MissingReturn { name, .. } if name == "<closure>")
    );
    assert!(matches!(
        semantic_error("fn main() { let f = () => { if true { return 1; } false }; }"),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        }
    ));
    assert!(matches!(
        semantic_error(
            "fn main() { let f = () => { if true { return 1; } else { return true; } }; }"
        ),
        SemanticError::IncompatibleInferredReturns { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = () => { return 1; false }; }"),
        SemanticError::TypeMismatch { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = () => { return; return 1; }; }"),
        SemanticError::CannotInferClosureResult { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = () => { return 1; missing; }; }"),
        SemanticError::UndefinedVariable { .. }
    ));
}

#[test]
fn lambdas_and_callable_bindings_are_not_values() {
    for body in [
        "() => missing;",
        "let x = [() => missing];",
        "return () => missing;",
    ] {
        let source = format!("fn main() -> number {{ {body} }}");
        assert!(
            matches!(
                semantic_error(&source),
                SemanticError::InvalidClosure {
                    reason: "a closure must initialize a local callable binding",
                    ..
                }
            ),
            "{source}"
        );
    }
    for use_ in [
        "let alias = f;",
        "return f;",
        "[f];",
        "f == f;",
        "let x = f[0];",
        "named(f);",
    ] {
        let source = format!(
            "fn named(x: number) -> void {{}} fn main() -> void {{ let f = () => 1; {use_} }}"
        );
        assert!(
            matches!(
                semantic_error(&source),
                SemanticError::InvalidClosure {
                    reason: "a local callable binding cannot be used as a value; invoke it directly or use a named function",
                    ..
                }
            ),
            "{source}"
        );
    }
    for use_ in ["f = 2;", "f += 2;"] {
        let source = format!("fn main() {{ let f = () => 1; {use_} }}");
        assert!(matches!(
            semantic_error(&source),
            SemanticError::InvalidClosure {
                reason: "a local callable binding cannot be reassigned",
                ..
            }
        ));
    }
    assert!(matches!(
        semantic_error("fn main() { let f: number = () => missing; }"),
        SemanticError::InvalidClosure {
            reason: "a local callable binding cannot have a value type annotation",
            ..
        }
    ));
}

#[test]
fn immediate_creator_values_are_captured_even_when_skipped_or_nested() {
    for source in [
        "fn main(x: number) { let f = () => x; }",
        "fn main() { let x = 1; let f = () => { x = 2; }; }",
        "fn main() { let x = [1]; let f = () => { x[0] = 2; }; }",
        "fn main() { let x = true; let f = () => false && x; }",
        "fn main() { let x = 1; let f = () => { return 1; x; }; }",
        "fn main() { let f = (x: number) => { let g = () => x; g() }; }",
    ] {
        Compiler::new().analyze(source).unwrap();
    }
}

#[test]
fn captured_compounds_keep_numeric_types_and_original_binding_provenance() {
    use was_compiler::typed_ast::{
        BindingOwner, FunctionId, TypedClosureBody, TypedExprKind, VariableId,
    };

    for op in ["+=", "-=", "*=", "/=", "%="] {
        for (initial, target, ty) in [
            ("8", "x", Type::Number),
            (
                "[8]",
                "x[0]",
                Type::Array(was_compiler::ArrayElementType::Number),
            ),
        ] {
            let source = format!(
                "fn main() {{ let x = {initial}; {target} {op} 2; let update = () => {{ {target} {op} 3; }}; {target} {op} 4; update(); }}"
            );
            let program = Compiler::new().analyze(&source).unwrap();
            let update = closure(&program, 0, 2);
            assert_eq!(update.captures.len(), 1);
            assert_eq!(update.captures[0].ty, ty);
            let TypedClosureBody::Block(body) = &update.body else {
                panic!("block");
            };
            let TypedStatementKind::CompoundAssignment { target, .. } = &body.statements[0].kind
            else {
                panic!("compound");
            };
            let base = match &target.kind {
                TypedExprKind::Index { expr, .. } => expr.as_ref(),
                _ => target,
            };
            assert!(matches!(
                base.kind,
                TypedExprKind::Captured {
                    owner: BindingOwner::Function(FunctionId(0)),
                    id: VariableId(0),
                    ..
                }
            ));
        }
    }
    for source in [
        "fn main(x: number) { let update = () => { x += 2; }; update(); }",
        "fn main(x: number) { let update = () => { return; x %= 2; }; }",
        "fn main() { let x = 1; let update = () => false && { x *= 2; true }; }",
        "fn main() { let x = 1; let update = () => { let x = x; x += 2; x }; }",
    ] {
        Compiler::new().analyze(source).unwrap();
    }
}

#[test]
fn captured_compounds_check_invalid_types_targets_and_skipped_children() {
    for source in [
        "fn main(x: bool) { let f = () => { x += 2; }; }",
        "fn main(x: string) { let f = () => { x *= 2; }; }",
        "fn main(x: range) { let f = () => { x %= 2; }; }",
        "fn main(x: number[]) { let f = () => { x -= 2; }; }",
        "fn main(x: number) { let f = () => { x /= true; }; }",
        "fn main(x: number[]) { let f = () => { x[0] += true; }; }",
        "fn main(x: number[]) { let f = () => { x[false] += 1; }; }",
        "fn main(x: number) { let f = () => { return; x += true; }; }",
        "fn main(x: number) { let f = () => false && { x += true; true }; }",
    ] {
        assert!(
            matches!(semantic_error(source), SemanticError::TypeMismatch { .. }),
            "{source}"
        );
    }
    for source in [
        "fn main() { for x in [1] { let f = () => { x += 2; }; } }",
        "fn main() { for x in [1] { let f = () => 1; x += 2; } }",
        "fn main() { let f = () => 1; f += 2; }",
    ] {
        assert!(Compiler::new().analyze(source).is_err(), "{source}");
    }
}

#[test]
fn nested_iteration_convergence_preserves_original_capture_and_dependency_provenance() {
    use was_compiler::OptimizationLevel;
    use was_compiler::typed_ast::{
        BindingOwner, CallableId, CaptureMutability, FunctionId, TypedCapture, TypedClosureBody,
        TypedExprKind, VariableId,
    };

    let source = r#"
        fn main() {
            let total = 0;
            let xs = [3, 4];
            for r in 1..3 {
                let base = () => r;
                let relay = () => base();
                let root = () => {
                    for a in xs {
                        let middle = () => {
                            let leaf = () => {
                                r;
                                a;
                                total += relay();
                                xs[0] += { xs = [a]; r };
                                total + a
                            };
                            leaf()
                        };
                        middle();
                    }
                    total
                };
                root();
            }
        }
    "#;
    let mut compiler = Compiler::new();
    let mut baseline = None;
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        let program = compiler.analyze(source).unwrap();
        let TypedStatementKind::For { variable, body, .. } = &program.functions[0].body[2].kind
        else {
            panic!("range for");
        };
        assert_eq!(*variable, VariableId(2));
        let [base, relay, root, _] = body.as_slice() else {
            panic!("range body");
        };
        let (
            TypedStatementKind::Closure(base),
            TypedStatementKind::Closure(relay),
            TypedStatementKind::Closure(root),
        ) = (&base.kind, &relay.kind, &root.kind)
        else {
            panic!("range callables");
        };
        let function = BindingOwner::Function(FunctionId(0));
        let range_capture = TypedCapture {
            owner: function,
            variable: VariableId(2),
            ty: Type::Number,
            mutability: CaptureMutability::ReadOnly,
        };
        let original_captures = vec![
            TypedCapture {
                owner: function,
                variable: VariableId(0),
                ty: Type::Number,
                mutability: CaptureMutability::Mutable,
            },
            TypedCapture {
                owner: function,
                variable: VariableId(1),
                ty: Type::Array(was_compiler::ArrayElementType::Number),
                mutability: CaptureMutability::Mutable,
            },
            range_capture.clone(),
        ];
        assert_eq!(base.id, CallableId(0));
        assert_eq!(base.owner, function);
        assert_eq!(base.captures, vec![range_capture]);
        assert!(base.dependencies.is_empty());
        assert_eq!(relay.id, CallableId(1));
        assert_eq!(relay.owner, function);
        assert!(relay.captures.is_empty());
        assert_eq!(relay.dependencies, vec![CallableId(0)]);
        assert_eq!(root.id, CallableId(2));
        assert_eq!(root.owner, function);
        assert_eq!(root.captures, original_captures);
        assert_eq!(root.dependencies, vec![CallableId(1)]);
        let TypedClosureBody::Block(root_body) = &root.body else {
            panic!("root block");
        };
        let TypedStatementKind::For { variable, body, .. } = &root_body.statements[0].kind else {
            panic!("array for");
        };
        assert_eq!(*variable, VariableId(0));
        let TypedStatementKind::Closure(middle) = &body[0].kind else {
            panic!("middle");
        };
        let TypedClosureBody::Block(middle_body) = &middle.body else {
            panic!("middle block");
        };
        let TypedStatementKind::Closure(leaf) = &middle_body.statements[0].kind else {
            panic!("leaf");
        };
        let mut nested_captures = original_captures;
        nested_captures.push(TypedCapture {
            owner: BindingOwner::Closure(CallableId(2)),
            variable: VariableId(0),
            ty: Type::Number,
            mutability: CaptureMutability::ReadOnly,
        });
        for (helper, id, owner) in [
            (middle, CallableId(3), BindingOwner::Closure(CallableId(2))),
            (leaf, CallableId(4), BindingOwner::Closure(CallableId(3))),
        ] {
            assert_eq!(helper.id, id);
            assert_eq!(helper.owner, owner);
            assert_eq!(helper.captures, nested_captures);
            assert_eq!(helper.dependencies, vec![CallableId(1)]);
            assert_eq!(helper.return_type, Type::Number);
        }
        let TypedClosureBody::Block(leaf_body) = &leaf.body else {
            panic!("leaf block");
        };
        for (statement, owner, id, name) in [
            (&leaf_body.statements[0], function, VariableId(2), "r"),
            (
                &leaf_body.statements[1],
                BindingOwner::Closure(CallableId(2)),
                VariableId(0),
                "a",
            ),
        ] {
            let TypedStatementKind::Expression(expr) = &statement.kind else {
                panic!("capture read");
            };
            assert_eq!(
                expr.kind,
                TypedExprKind::Captured {
                    owner,
                    id,
                    name: name.into()
                }
            );
        }
        for (statement, id, name) in [
            (&leaf_body.statements[2], VariableId(0), "total"),
            (&leaf_body.statements[3], VariableId(1), "xs"),
        ] {
            let TypedStatementKind::CompoundAssignment { target, .. } = &statement.kind else {
                panic!("compound place");
            };
            let base = match &target.kind {
                TypedExprKind::Index { expr, .. } => expr.as_ref(),
                _ => target,
            };
            assert_eq!(
                base.kind,
                TypedExprKind::Captured {
                    owner: function,
                    id,
                    name: name.into()
                }
            );
        }
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        if let Some((expected_program, expected_bytes)) = &baseline {
            assert_eq!(&program, expected_program, "{level:?}");
            assert_eq!(&bytes, expected_bytes, "{level:?}");
        } else {
            baseline = Some((program, bytes));
        }
    }
}

#[test]
fn nested_iteration_convergence_dependency_only_helpers_do_not_recapture_shadowed_values() {
    use was_compiler::OptimizationLevel;
    use was_compiler::typed_ast::{
        BindingOwner, CallableId, CaptureMutability, FunctionId, TypedCapture, TypedClosureBody,
        VariableId,
    };

    let source = r#"
        fn main() {
            let total = 0;
            for r in 2..=3 {
                for a in [5, 7] {
                    let base = () => { total += r + a; total };
                    let relay = () => base();
                    let outer = () => {
                        let r = 100;
                        let a = 200;
                        let middle = () => {
                            let leaf = () => relay();
                            leaf() + leaf()
                        };
                        middle()
                    };
                    total += 1;
                    outer();
                }
            }
            return total;
        }
    "#;
    let mut compiler = Compiler::new();
    let expected = compiler.analyze(source).unwrap();
    let expected_bytes = compiler.compile(&expected).unwrap();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        let program = compiler.analyze(source).unwrap();
        assert_eq!(program, expected, "{level:?}");
        assert_eq!(
            compiler.compile(&program).unwrap(),
            expected_bytes,
            "{level:?}"
        );
        let TypedStatementKind::For { body, .. } = &program.functions[0].body[1].kind else {
            panic!("range for");
        };
        let TypedStatementKind::For { body, .. } = &body[0].kind else {
            panic!("array for");
        };
        let TypedStatementKind::Closure(base) = &body[0].kind else {
            panic!("base");
        };
        assert_eq!(base.id, CallableId(0));
        assert_eq!(base.owner, BindingOwner::Function(FunctionId(0)));
        assert_eq!(base.return_type, Type::Number);
        assert!(base.dependencies.is_empty());
        assert_eq!(
            base.captures,
            vec![
                TypedCapture {
                    owner: BindingOwner::Function(FunctionId(0)),
                    variable: VariableId(0),
                    ty: Type::Number,
                    mutability: CaptureMutability::Mutable
                },
                TypedCapture {
                    owner: BindingOwner::Function(FunctionId(0)),
                    variable: VariableId(1),
                    ty: Type::Number,
                    mutability: CaptureMutability::ReadOnly
                },
                TypedCapture {
                    owner: BindingOwner::Function(FunctionId(0)),
                    variable: VariableId(2),
                    ty: Type::Number,
                    mutability: CaptureMutability::ReadOnly
                },
            ]
        );
        let TypedStatementKind::Closure(relay) = &body[1].kind else {
            panic!("relay");
        };
        assert_eq!(relay.id, CallableId(1));
        assert!(relay.captures.is_empty());
        assert_eq!(relay.dependencies, vec![CallableId(0)]);
        let TypedStatementKind::Closure(outer) = &body[2].kind else {
            panic!("outer");
        };
        let TypedClosureBody::Block(outer_body) = &outer.body else {
            panic!("outer block");
        };
        let TypedStatementKind::Closure(middle) = &outer_body.statements[2].kind else {
            panic!("middle");
        };
        let TypedClosureBody::Block(middle_body) = &middle.body else {
            panic!("middle block");
        };
        let TypedStatementKind::Closure(leaf) = &middle_body.statements[0].kind else {
            panic!("leaf");
        };
        for (helper, id, owner) in [
            (outer, CallableId(2), BindingOwner::Function(FunctionId(0))),
            (middle, CallableId(3), BindingOwner::Closure(CallableId(2))),
            (leaf, CallableId(4), BindingOwner::Closure(CallableId(3))),
        ] {
            assert_eq!(helper.id, id);
            assert_eq!(helper.owner, owner);
            assert_eq!(helper.return_type, Type::Number);
            assert!(helper.captures.is_empty());
            assert_eq!(helper.dependencies, vec![CallableId(1)]);
        }
    }
}

#[test]
fn nested_iteration_convergence_rejects_skipped_and_unreachable_read_only_writes() {
    use was_compiler::OptimizationLevel;

    for (outer, inner) in [("1..3", "[4, 5]"), ("[4, 5]", "1..3")] {
        for target in ["outer", "inner"] {
            for operator in ["=", "+=", "-=", "*=", "/=", "%="] {
                let write = format!("{target} {operator} 9;");
                for body in [
                    write.clone(),
                    format!("return; {write}"),
                    format!("false && {{ {write} true }};"),
                    format!("if false {{ {write} }}"),
                ] {
                    let source = format!(
                        "fn main() {{ for outer in {outer} {{
                            let read = () => outer;
                            let relay = () => read();
                            let root = () => {{ for inner in {inner} {{
                                let middle = () => {{
                                    let leaf = () => {{ relay(); outer; inner; {body} }};
                                }};
                            }} }};
                        }} }}"
                    );
                    for level in [
                        OptimizationLevel::None,
                        OptimizationLevel::Basic,
                        OptimizationLevel::Standard,
                        OptimizationLevel::Aggressive,
                    ] {
                        let mut compiler = Compiler::new();
                        compiler.set_optimization_level(level);
                        for error in [
                            compiler.analyze(&source).unwrap_err(),
                            compiler.compile_source(&source).unwrap_err(),
                        ] {
                            let CompileError::Semantic(error) = error else {
                                panic!("expected semantic error: {source}");
                            };
                            assert!(
                                matches!(&error, SemanticError::ReadOnlyVariable { name, .. } if name == target),
                                "{level:?}: {error:?}: {source}"
                            );
                            let span = error.span().unwrap();
                            assert_eq!(&source[span.start()..span.end()], write, "{level:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn iteration_captures_preserve_original_read_only_number_metadata() {
    use was_compiler::typed_ast::{BindingOwner, CaptureMutability, FunctionId, TypedCapture};

    for iterable in ["1..2", "[1]", "2..1", "[]"] {
        let source = format!("fn main() {{ for x in {iterable} {{ let f = () => x; f(); }} }}");
        let program = Compiler::new().analyze(&source).unwrap();
        let TypedStatementKind::For { variable, body, .. } = &program.functions[0].body[0].kind
        else {
            panic!("for");
        };
        let TypedStatementKind::Closure(c) = &body[0].kind else {
            panic!("closure");
        };
        assert_eq!(
            c.captures,
            vec![TypedCapture {
                owner: BindingOwner::Function(FunctionId(0)),
                variable: *variable,
                ty: Type::Number,
                mutability: CaptureMutability::ReadOnly,
            }]
        );
        assert_eq!(c.return_type, Type::Number);
    }
}

#[test]
fn iteration_capture_writes_are_rejected_without_weakening_shadowing_or_transfer_boundaries() {
    for iterable in ["0..2", "[1, 2]"] {
        for write in [
            "x = 3;", "x += 3;", "x -= 3;", "x *= 3;", "x /= 3;", "x %= 3;",
        ] {
            for body in [
                write.to_string(),
                format!("return; {write}"),
                format!("false && {{ {write} true }};"),
            ] {
                let source = format!(
                    "fn main() {{ for x in {iterable} {{ let f = () => {{ {body} }}; }} }}"
                );
                let error = semantic_error(&source);
                assert!(
                    matches!(&error, SemanticError::ReadOnlyVariable { name, .. } if name == "x"),
                    "{error:?}"
                );
                let span = error.span().unwrap();
                assert_eq!(&source[span.start()..span.end()], write);
            }
        }
        Compiler::new().compile_source(&format!("fn main() {{ for x in {iterable} {{ let read = () => x; let update = () => {{ let x = x; x += 1; x }}; read(); update(); }} }}")).unwrap();
        for transfer in ["break;", "continue;"] {
            let source = format!(
                "fn main() {{ for x in {iterable} {{ let f = () => {{ x; {transfer} }}; }} }}"
            );
            assert!(Compiler::new().analyze(&source).is_err(), "{source}");
        }
    }
}

#[test]
fn nested_captures_forward_original_bindings_without_capturing_creator_owned_locals() {
    use was_compiler::typed_ast::{
        BindingOwner, CallableId, CaptureMutability, FunctionId, TypedCapture, TypedClosureBody,
        TypedExprKind, VariableId,
    };

    let source = "fn main(x: number) { let outer = (y: bool) => { let middle = () => { let inner = () => { y; x += 1; x }; inner() }; middle() }; }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let outer = closure(&program, 0, 0);
    let TypedClosureBody::Block(body) = &outer.body else {
        panic!("block")
    };
    let TypedStatementKind::Closure(middle) = &body.statements[0].kind else {
        panic!("closure")
    };
    let TypedClosureBody::Block(body) = &middle.body else {
        panic!("block")
    };
    let TypedStatementKind::Closure(inner) = &body.statements[0].kind else {
        panic!("closure")
    };
    let original = TypedCapture {
        owner: BindingOwner::Function(FunctionId(0)),
        variable: VariableId(0),
        ty: Type::Number,
        mutability: CaptureMutability::Mutable,
    };
    let creator_parameter = TypedCapture {
        owner: BindingOwner::Closure(CallableId(0)),
        variable: VariableId(0),
        ty: Type::Bool,
        mutability: CaptureMutability::Mutable,
    };
    assert_eq!(outer.captures, vec![original.clone()]);
    assert_eq!(
        middle.captures,
        vec![original.clone(), creator_parameter.clone()]
    );
    assert_eq!(inner.captures, vec![original, creator_parameter]);
    let TypedClosureBody::Block(body) = &inner.body else {
        panic!("block")
    };
    assert!(matches!(
        body.tail.as_ref().unwrap().kind,
        TypedExprKind::Captured {
            owner: BindingOwner::Function(FunctionId(0)),
            id: VariableId(0),
            ..
        }
    ));
    assert_eq!(compiler.analyze(source).unwrap(), program);
}

#[test]
fn capturing_callable_dependencies_forward_live_context_without_value_reads() {
    use was_compiler::typed_ast::{CallableId, TypedClosureBody};

    let source = "fn main(x: number) { let base = () => x; let free = () => 2; let root = () => { let middle = () => { let inner = () => free() + base() + free() + base(); inner() }; middle() }; let relay = () => root(); relay(); }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let root = closure(&program, 0, 2);
    let TypedClosureBody::Block(body) = &root.body else {
        panic!("block")
    };
    let TypedStatementKind::Closure(middle) = &body.statements[0].kind else {
        panic!("closure")
    };
    let TypedClosureBody::Block(body) = &middle.body else {
        panic!("block")
    };
    let TypedStatementKind::Closure(inner) = &body.statements[0].kind else {
        panic!("closure")
    };
    for helper in [root, middle, inner] {
        assert!(helper.captures.is_empty());
        assert_eq!(helper.dependencies, vec![CallableId(0), CallableId(1)]);
        assert_eq!(helper.return_type, Type::Number);
    }
    let relay = closure(&program, 0, 3);
    assert!(relay.captures.is_empty());
    assert_eq!(relay.dependencies, vec![CallableId(2)]);
    assert_eq!(compiler.analyze(source).unwrap(), program);
}

#[test]
fn nested_capture_types_shadowing_and_skipped_places_preserve_original_owners() {
    use was_compiler::typed_ast::{
        BindingOwner, CallableId, CaptureMutability, FunctionId, TypedClosureBody, TypedExprKind,
        VariableId,
    };

    for (value, replacement, ty) in [
        ("1", "2", Type::Number),
        ("true", "false", Type::Bool),
        ("\"first\"", "\"second\"", Type::String),
        (
            "[1]",
            "[2]",
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
        ("1..2", "3..4", Type::Range),
    ] {
        let source = format!(
            "fn main() {{ let x = {value}; let outer = () => {{ let x = x; let middle = () => {{ let inner = () => {{ if false {{ x = {replacement}; }} x }}; inner() }}; middle() }}; }}"
        );
        let program = Compiler::new().analyze(&source).unwrap();
        let outer = closure(&program, 0, 1);
        assert_eq!(outer.captures.len(), 1);
        assert_eq!(
            outer.captures[0].owner,
            BindingOwner::Function(FunctionId(0))
        );
        let TypedClosureBody::Block(body) = &outer.body else {
            panic!("block")
        };
        let TypedStatementKind::Let { value, .. } = &body.statements[0].kind else {
            panic!("let")
        };
        assert!(matches!(
            value.kind,
            TypedExprKind::Captured {
                owner: BindingOwner::Function(FunctionId(0)),
                id: VariableId(0),
                ..
            }
        ));
        let TypedStatementKind::Closure(middle) = &body.statements[1].kind else {
            panic!("closure")
        };
        let TypedClosureBody::Block(body) = &middle.body else {
            panic!("block")
        };
        let TypedStatementKind::Closure(inner) = &body.statements[0].kind else {
            panic!("closure")
        };
        for helper in [middle, inner] {
            assert_eq!(helper.captures.len(), 1);
            let capture = &helper.captures[0];
            assert_eq!(capture.owner, BindingOwner::Closure(CallableId(0)));
            assert_eq!(capture.variable, VariableId(0));
            assert_eq!(capture.ty, ty);
            assert_eq!(capture.mutability, CaptureMutability::Mutable);
            assert_eq!(helper.return_type, ty);
        }
    }
    for source in [
        "fn main(x: number) { let outer = () => { let inner = () => { x = true; }; }; }",
        "fn main(x: number[]) { let outer = () => { let inner = () => { x[0] += false; }; }; }",
    ] {
        assert!(matches!(
            semantic_error(source),
            SemanticError::TypeMismatch { .. }
        ));
    }
    assert!(matches!(
        semantic_error("fn main() { let outer = () => { let inner = () => missing; }; let missing = 1; }"),
        SemanticError::UndefinedVariable { name, .. } if name == "missing"
    ));
}

#[test]
fn nested_capturing_dependencies_stop_at_their_creator_and_keep_initializer_visibility() {
    use was_compiler::typed_ast::{CallableId, TypedClosureBody, TypedExprKind};

    let source = "fn main(x: number) { let outer = () => { let base = () => x; let middle = () => { let base = () => base(); base() }; middle() }; }";
    let program = Compiler::new().analyze(source).unwrap();
    let outer = closure(&program, 0, 0);
    assert_eq!(outer.captures.len(), 1);
    assert!(outer.dependencies.is_empty());
    let TypedClosureBody::Block(body) = &outer.body else {
        panic!("block")
    };
    let TypedStatementKind::Closure(base) = &body.statements[0].kind else {
        panic!("closure")
    };
    assert_eq!(base.captures, outer.captures);
    let TypedStatementKind::Closure(middle) = &body.statements[1].kind else {
        panic!("closure")
    };
    assert!(middle.captures.is_empty());
    assert_eq!(middle.dependencies, vec![CallableId(1)]);
    let TypedClosureBody::Block(body) = &middle.body else {
        panic!("block")
    };
    let TypedStatementKind::Closure(inner) = &body.statements[0].kind else {
        panic!("closure")
    };
    assert!(inner.captures.is_empty());
    assert_eq!(inner.dependencies, vec![CallableId(1)]);
    let TypedClosureBody::Expression(call) = &inner.body else {
        panic!("expression")
    };
    assert!(matches!(
        call.kind,
        TypedExprKind::LexicalCall {
            callable: CallableId(1),
            ..
        }
    ));
}

#[test]
fn captured_locals_retain_concrete_types_and_indexed_places_use_original_array_binding() {
    use was_compiler::typed_ast::{TypedClosureBody, TypedExprKind, VariableId};
    for (value, replacement, ty) in [
        ("1", "2", Type::Number),
        ("true", "false", Type::Bool),
        ("\"first\"", "\"second\"", Type::String),
        (
            "[1]",
            "[2]",
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
        ("1..2", "3..4", Type::Range),
    ] {
        let source = format!(
            "fn main() {{ let x = {value}; let read = () => x; let write = () => {{ x = {replacement}; x }}; x = {replacement}; read(); write(); }}"
        );
        let program = Compiler::new().analyze(&source).unwrap();
        for index in [1, 2] {
            let callable = closure(&program, 0, index);
            assert_eq!(callable.captures.len(), 1);
            assert_eq!(callable.captures[0].variable, VariableId(0));
            assert_eq!(callable.captures[0].ty, ty);
            assert_eq!(callable.return_type, ty);
        }
    }
    let program = Compiler::new()
        .analyze("fn main() { let a = [1]; let f = () => { a[0] = a[0] + 1; a[0] }; }")
        .unwrap();
    let f = closure(&program, 0, 1);
    assert_eq!(f.captures.len(), 1);
    let TypedClosureBody::Block(body) = &f.body else {
        panic!("block")
    };
    let TypedStatementKind::Assignment { target, .. } = &body.statements[0].kind else {
        panic!("assignment")
    };
    let TypedExprKind::Index { expr, .. } = &target.kind else {
        panic!("index")
    };
    assert!(matches!(
        expr.kind,
        TypedExprKind::Captured {
            id: VariableId(0),
            ..
        }
    ));
}

#[test]
fn capture_shadowing_initializer_visibility_and_assignment_types_remain_lexical() {
    use was_compiler::typed_ast::{
        BindingOwner, CallableId, TypedClosureBody, TypedExprKind, VariableId,
    };
    let program = Compiler::new().analyze("fn main(x: number) { let f = () => { let x = x; x = 3; let g = () => x; g() }; let x2 = 4; }").unwrap();
    let f = closure(&program, 0, 0);
    let TypedClosureBody::Block(body) = &f.body else {
        panic!("block")
    };
    let TypedStatementKind::Let { id, value, .. } = &body.statements[0].kind else {
        panic!("let")
    };
    assert_eq!(*id, VariableId(0));
    assert!(matches!(
        value.kind,
        TypedExprKind::Captured {
            id: VariableId(0),
            ..
        }
    ));
    let TypedStatementKind::Closure(g) = &body.statements[2].kind else {
        panic!("closure")
    };
    assert_eq!(g.captures.len(), 1);
    assert_eq!(g.captures[0].owner, BindingOwner::Closure(CallableId(0)));
    assert_eq!(g.captures[0].variable, VariableId(0));
    assert!(f.dependencies.is_empty());
    for source in [
        "fn main() { let f = () => missing; let missing = 1; }",
        "fn main() { let f = () => { let x = x; x }; }",
    ] {
        assert!(matches!(
            semantic_error(source),
            SemanticError::UndefinedVariable { .. }
        ));
    }
    for source in [
        "fn main(x: number) { let f = () => { x = true; }; }",
        "fn main(x: number) { let f = () => x; x = true; }",
        "fn main(x: number[]) { let f = () => { x[0] = true; }; }",
    ] {
        assert!(matches!(
            semantic_error(source),
            SemanticError::TypeMismatch { .. }
        ));
    }
}

#[test]
fn captures_preserve_all_concrete_types_original_identity_and_declaration_order() {
    use was_compiler::typed_ast::{
        BindingOwner, CaptureMutability, FunctionId, TypedClosureBody, TypedExprKind, VariableId,
    };
    let program = Compiler::new().analyze(
        "fn main(n: number, b: bool, s: string, a: number[], r: range) { let local = 7; let f = (own: number) => { r; a; s; b; local = own; n }; n = 2; }"
    ).unwrap();
    let f = closure(&program, 0, 1);
    assert_eq!(
        f.captures
            .iter()
            .map(|capture| (
                capture.owner,
                capture.variable,
                capture.ty,
                capture.mutability
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                BindingOwner::Function(FunctionId(0)),
                VariableId(0),
                Type::Number,
                CaptureMutability::Mutable
            ),
            (
                BindingOwner::Function(FunctionId(0)),
                VariableId(1),
                Type::Bool,
                CaptureMutability::Mutable
            ),
            (
                BindingOwner::Function(FunctionId(0)),
                VariableId(2),
                Type::String,
                CaptureMutability::Mutable
            ),
            (
                BindingOwner::Function(FunctionId(0)),
                VariableId(3),
                Type::Array(was_compiler::ArrayElementType::Number),
                CaptureMutability::Mutable
            ),
            (
                BindingOwner::Function(FunctionId(0)),
                VariableId(4),
                Type::Range,
                CaptureMutability::Mutable
            ),
            (
                BindingOwner::Function(FunctionId(0)),
                VariableId(5),
                Type::Number,
                CaptureMutability::Mutable
            ),
        ]
    );
    let TypedClosureBody::Block(body) = &f.body else {
        panic!("block")
    };
    assert!(
        matches!(&body.tail.as_ref().unwrap().kind, TypedExprKind::Captured {
        owner: BindingOwner::Function(FunctionId(0)), id: VariableId(0), name
    } if name == "n")
    );
    let TypedStatementKind::Assignment { target, value } = &body.statements[4].kind else {
        panic!("assignment")
    };
    assert!(matches!(
        target.kind,
        TypedExprKind::Captured {
            id: VariableId(5),
            ..
        }
    ));
    assert!(matches!(
        value.kind,
        TypedExprKind::Identifier {
            id: VariableId(0),
            ..
        }
    ));
    assert!(
        matches!(&program.functions[0].body[2].kind, TypedStatementKind::Assignment { target, .. }
        if matches!(target.kind, TypedExprKind::Identifier { id: VariableId(0), .. }))
    );
}

#[test]
fn closures_have_independent_loop_and_variable_id_contexts() {
    use was_compiler::typed_ast::{LoopId, TypedClosureBody, VariableId};
    for (transfer, is_break) in [("break;", true), ("continue;", false)] {
        let source = format!("fn main() {{ loop {{ let f = () => {{ {transfer} }}; break; }} }}");
        let error = semantic_error(&source);
        assert!(if is_break {
            matches!(error, SemanticError::BreakOutsideLoop { .. })
        } else {
            matches!(error, SemanticError::ContinueOutsideLoop { .. })
        });
    }
    let program = Compiler::new().analyze("fn main(x: number) { let f = (y: number) => { let z = y; loop { break; } z }; let n = 1; loop { break; } }").unwrap();
    let f = closure(&program, 0, 0);
    assert_eq!(f.params[0].id, VariableId(0));
    let TypedClosureBody::Block(body) = &f.body else {
        panic!("block");
    };
    assert!(matches!(
        body.statements[0].kind,
        TypedStatementKind::Let {
            id: VariableId(1),
            ..
        }
    ));
    assert!(matches!(
        body.statements[1].kind,
        TypedStatementKind::Loop { id: LoopId(0), .. }
    ));
    assert!(matches!(
        program.functions[0].body[1].kind,
        TypedStatementKind::Let {
            id: VariableId(1),
            ..
        }
    ));
    assert!(matches!(
        program.functions[0].body[2].kind,
        TypedStatementKind::Loop { id: LoopId(0), .. }
    ));
}

#[test]
fn nested_dependencies_and_callable_ids_are_source_preorder_not_inference_order() {
    use was_compiler::typed_ast::{BindingOwner, CallableId, FunctionId, TypedClosureBody};
    let source = "fn first() { let a = () => later(); let b = () => { let c = () => a(); c() }; return b(); } fn later() { let d = () => 7; return d(); }";
    let program = Compiler::new().analyze(source).unwrap();
    assert_eq!(closure(&program, 0, 0).id, CallableId(0));
    let b = closure(&program, 0, 1);
    assert_eq!(b.id, CallableId(1));
    assert_eq!(b.owner, BindingOwner::Function(FunctionId(0)));
    assert_eq!(b.dependencies, vec![CallableId(0)]);
    let TypedClosureBody::Block(body) = &b.body else {
        panic!("block");
    };
    let TypedStatementKind::Closure(c) = &body.statements[0].kind else {
        panic!("closure");
    };
    assert_eq!(c.id, CallableId(2));
    assert_eq!(c.owner, BindingOwner::Closure(CallableId(1)));
    assert_eq!(c.dependencies, vec![CallableId(0)]);
    assert_eq!(closure(&program, 1, 0).id, CallableId(3));
    assert_eq!(program.functions[0].return_type, Type::Number);
    assert_eq!(Compiler::new().analyze(source).unwrap(), program);
}

#[test]
fn named_recursion_in_lambda_bodies_requires_named_annotations_but_not_creator_returns() {
    assert!(
        matches!(semantic_error("fn main() { let f = () => main(); }"), SemanticError::RecursiveResultAnnotationRequired { name, .. } if name == "main")
    );
    assert!(matches!(
        semantic_error("fn a() { let f = () => b(); } fn b() -> void { a(); }"),
        SemanticError::RecursiveResultAnnotationRequired { .. }
    ));
    Compiler::new()
        .analyze("fn main() -> number { let f = () => main(); return 1; }")
        .unwrap();
    let program = Compiler::new()
        .analyze("fn main() { let f = () => { return 1; }; }")
        .unwrap();
    assert_eq!(program.functions[0].return_type, Type::Void);
}

#[test]
fn callables_obey_declaration_order_shadowing_scope_and_positional_signatures() {
    for source in [
        "fn main() { let f = () => f(); }",
        "fn main() { let f = () => g(); let g = () => 1; }",
        "fn main() { if true { let f = () => 1; } f(); }",
    ] {
        assert!(
            matches!(
                semantic_error(source),
                SemanticError::UndefinedFunction { .. }
            ),
            "{source}"
        );
    }
    assert!(matches!(
        semantic_error("fn f() -> number { return 1; } fn main() { let f = 1; f(); }"),
        SemanticError::NotCallable { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = () => 1; let f = () => 2; }"),
        SemanticError::DuplicateVariable { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = (x: number) => x; f(); }"),
        SemanticError::ArgumentCountMismatch {
            expected: 1,
            found: 0,
            ..
        }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = (x: number) => x; f(true); }"),
        SemanticError::TypeMismatch { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = (x: number) => x; f(x: 1); }"),
        SemanticError::NamedArgumentsNotSupported { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = (x: void) => missing; }"),
        SemanticError::InvalidVoidType { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() { let f = (x: number, x: number) => missing; }"),
        SemanticError::DuplicateVariable { .. }
    ));
    Compiler::new().analyze("fn f() -> number { return 1; } fn main() { let f = () => f(); let n = { let f = () => f(); f() }; f(); }").unwrap();
}

#[test]
fn closures_call_named_defaults_and_contracts_but_are_not_pipeline_stages() {
    Compiler::new().analyze("fn named(x: number, y: number = x + 1) -> number requires x > 0 { return y; } fn main() { let f = (x: number) => named(x: x); f(1); }").unwrap();
    assert!(matches!(
        semantic_error(
            "fn f(x: number) -> number { return x; } fn main() { let f = (x: number) => x; 1 |> f; }"
        ),
        SemanticError::InvalidPipeline {
            message: "stages must name user-defined functions",
            ..
        }
    ));
    assert!(matches!(
        semantic_error("fn main(x: number = () => missing) {}"),
        SemanticError::InvalidDefaultExpression { .. }
    ));
    assert!(matches!(
        semantic_error("fn main() requires () => missing {}"),
        SemanticError::InvalidContractExpression { .. }
    ));
}

#[test]
fn malformed_and_unsupported_lambda_syntax_stays_in_parser() {
    for expression in [
        "(x) => x",
        "(x: number = 1) => x",
        "(x: number,) => x",
        "() -> number => 1",
        "() requires true => 1",
        "(() => 1)()",
        "(x: number) =>",
        "(x: number => x",
    ] {
        let source = format!("fn main() {{ let f = {expression}; }}");
        assert!(
            matches!(
                Compiler::new().analyze(&source),
                Err(CompileError::Parse(_))
            ),
            "{source}"
        );
    }
    let source = "fn main() { let f = (x: number) => { x }; }";
    for end in 0..source.len() {
        assert!(
            Compiler::new().analyze(&source[..end]).is_err(),
            "prefix {end}"
        );
    }
}

#[test]
fn transparent_lambda_parentheses_and_utf8_trivia_preserve_focused_spans() {
    use was_compiler::typed_ast::TypedExprKind;
    let source = "fn main() { /*é*/ let f = (((x: number) => { x + 1 })); f(2); }";
    let program = Compiler::new().analyze(source).unwrap();
    let f = closure(&program, 0, 0);
    assert_eq!(&source[f.name_span.start()..f.name_span.end()], "f");
    assert_eq!(
        &source[f.span.start()..f.span.end()],
        "(x: number) => { x + 1 }"
    );
    assert_eq!(&source[f.body_span.start()..f.body_span.end()], "{ x + 1 }");
    let TypedStatementKind::Expression(call) = &program.functions[0].body[1].kind else {
        panic!("call");
    };
    let TypedExprKind::LexicalCall { span, .. } = call.kind else {
        panic!("lexical call");
    };
    assert_eq!(&source[span.start()..span.end()], "f(2)");
    let source = "fn main() { let f = () => loop {}; }";
    let error = semantic_error(source);
    let span = error.span().unwrap();
    assert_eq!(&source[span.start()..span.end()], "loop {}");
    assert_eq!(error.to_string(), "cannot infer a concrete closure result");
}

#[test]
fn analysis_and_diagnostics_are_identical_at_all_optimization_levels() {
    use was_compiler::{CompilerOptions, OptimizationLevel};
    let source = "fn main() { let factor = 2; let f = (x: number) => x * factor; let outer = () => { let inner = () => factor + f(21); inner() }; return outer(); }";
    let expected = Compiler::new().analyze(source).unwrap();
    let invalid = "fn main() { for x in [1] { let f = () => { x += 1; }; } }";
    let expected_error = Compiler::new().analyze(invalid).unwrap_err();
    for optimization_level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions { optimization_level });
        assert_eq!(compiler.analyze(source).unwrap(), expected);
        assert_eq!(compiler.analyze(invalid).unwrap_err(), expected_error);
    }
}

#[test]
fn closure_owned_mutable_values_and_read_only_iteration_bindings_keep_existing_rules() {
    Compiler::new().analyze("fn main() { let f = (x: number) => { x += 1; let a = [x]; a[0] = x; for y in a { let z = y; z += 1; } x }; f(1); }").unwrap();
    assert!(
        matches!(semantic_error("fn main() { let f = () => { for x in 1..2 { x = 3; } }; }"), SemanticError::ReadOnlyVariable { name, .. } if name == "x")
    );
    assert!(matches!(
        semantic_error("fn main() { let f = () => loop { break true; }; f() + 1; }"),
        SemanticError::InvalidBinaryOperands { .. }
    ));
}

#[test]
fn calls_use_concrete_results_and_never_only_tails_need_an_inferable_return() {
    Compiler::new().analyze("fn main() { let f = () => { loop {} }; let g = () => { if true { return 1; } (loop {}) }; g(); }").unwrap();
    assert!(
        matches!(semantic_error("fn main() { let f = () => { loop {} }; let value = f(); }"), SemanticError::InvalidVoidType { name, .. } if name == "value")
    );
    assert!(matches!(
        semantic_error("fn main() { let f = () => { (loop {}) }; }"),
        SemanticError::CannotInferClosureResult { .. }
    ));
    let program = Compiler::new()
        .analyze("fn main() { let f = () => { (loop { break 1; }) }; }")
        .unwrap();
    assert_eq!(closure(&program, 0, 0).return_type, Type::Number);
}

#[test]
fn unsupported_placements_annotations_and_lexical_calls_keep_narrow_origins() {
    let source = "fn main() { /*é*/ let f: void = (() => missing); }";
    let error = semantic_error(source);
    let span = error.span().unwrap();
    assert_eq!(&source[span.start()..span.end()], "f");
    let source = "fn main() { /*é*/ (() => missing); }";
    let error = semantic_error(source);
    let span = error.span().unwrap();
    assert_eq!(&source[span.start()..span.end()], "() => missing");
    let source = "fn main() { let f = (x: number) => x; /*é*/ f(1, 2); }";
    let error = semantic_error(source);
    assert!(
        matches!(&error, SemanticError::ArgumentCountMismatch { name, expected: 1, found: 2, .. } if name == "<closure>")
    );
    let span = error.span().unwrap();
    assert_eq!(&source[span.start()..span.end()], "f(1, 2)");
    let source = "fn main() { let f = (x: number) => x; f(x: missing); }";
    let error = semantic_error(source);
    assert!(
        matches!(&error, SemanticError::NamedArgumentsNotSupported { name, .. } if name == "<closure>")
    );
    let span = error.span().unwrap();
    assert_eq!(&source[span.start()..span.end()], "x: missing");
}

#[test]
fn unreachable_and_skipped_lambda_syntax_participates_in_named_recursion_discovery() {
    for source in [
        "fn main() { return; let f = () => main(); }",
        "fn main() { let f = () => { return 1; main(); }; }",
        "fn main() { if false { let f = () => main(); } }",
        "fn main() { let f = () => { let g = () => main(); return 1; }; }",
    ] {
        assert!(
            matches!(
                semantic_error(source),
                SemanticError::RecursiveResultAnnotationRequired { .. }
            ),
            "{source}"
        );
    }
}

fn semantic_error(source: &str) -> SemanticError {
    match Compiler::new().analyze(source).unwrap_err() {
        CompileError::Semantic(error) => error,
        error => panic!("expected semantic error: {error:?}"),
    }
}

#[test]
fn closure_returns_do_not_infer_creator_results_and_unreachable_returns_are_checked() {
    let program = Compiler::new()
        .analyze("fn main() { let f = () => { return 1; }; }")
        .unwrap();
    assert_eq!(program.functions[0].return_type, Type::Void);
    let TypedStatementKind::Closure(closure) = &program.functions[0].body[0].kind else {
        panic!("closure");
    };
    assert_eq!(closure.return_type, Type::Number);
    assert!(matches!(
        semantic_error("fn main() { let f = () => { return 1; return true; }; }"),
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        }
    ));
}

#[test]
fn dependencies_use_callable_source_discovery_order_not_call_order() {
    let program = Compiler::new()
        .analyze("fn main() { let a = () => 1; let b = () => 2; let c = () => b() + a() + b(); }")
        .unwrap();
    let TypedStatementKind::Closure(closure) = &program.functions[0].body[2].kind else {
        panic!("closure");
    };
    assert_eq!(
        closure.dependencies,
        vec![
            was_compiler::typed_ast::CallableId(0),
            was_compiler::typed_ast::CallableId(1)
        ]
    );
}

#[test]
fn recognized_lambda_return_is_a_placement_error_even_in_void_creator() {
    assert!(matches!(
        semantic_error("fn main() -> void { return () => missing; }"),
        SemanticError::InvalidClosure {
            reason: "a closure must initialize a local callable binding",
            ..
        }
    ));
}

#[test]
fn capture_free_local_callable_can_be_declared_and_invoked() {
    let source = "fn main() -> number { let twice = (x: number) => x * 2; return twice(21); }";
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    assert_eq!(program.functions[0].return_type, Type::Number);
    assert!(matches!(
        program.functions[0].body[1].kind,
        TypedStatementKind::Return(Some(_))
    ));
    assert!(matches!(
        program.functions[0].body[0].kind,
        TypedStatementKind::Closure(_)
    ));
}
