use was_compiler::{CodegenError, Compiler, Type, tokens::Span, typed_ast::*};
use wasmparser::{Operator, Parser, Payload, Validator};

fn span() -> Span {
    Span::new(31, 57)
}
fn number(value: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number(value.into()),
        ty: Type::Number.into(),
    }
}
fn statement(kind: TypedStatementKind) -> TypedStatement {
    TypedStatement { kind, span: span() }
}
fn closure(id: usize, owner: BindingOwner, body: TypedClosureBody, result: Type) -> TypedClosure {
    TypedClosure {
        id: CallableId(id),
        owner,
        name: format!("c{id}"),
        name_span: span(),
        params: vec![],
        return_type: result,
        body,
        captures: vec![],
        dependencies: vec![],
        span: span(),
        body_span: span(),
    }
}
fn lexical(id: usize, args: Vec<TypedExpr>, result: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::LexicalCall {
            callable: CallableId(id),
            args,
            span: span(),
        },
        ty: result.into(),
    }
}
fn program(body: Vec<TypedStatement>) -> TypedProgram {
    TypedProgram {
        host_functions: vec![],
        functions: vec![TypedFunction {
            id: FunctionId(0),
            name: "main".into(),
            params: vec![],
            return_type: Type::Number,
            contracts: vec![],
            body,
            span: span(),
        }],
    }
}
fn operators(bytes: &[u8], index: usize) -> Vec<Operator<'_>> {
    Validator::new().validate_all(bytes).unwrap();
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .nth(index)
        .unwrap()
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

fn block(statements: Vec<TypedStatement>, tail: Option<TypedExpr>) -> TypedClosureBody {
    TypedClosureBody::Block(TypedValueBlock {
        statements,
        tail: tail.map(Box::new),
        span: span(),
    })
}
fn assert_invalid(program: &TypedProgram, reason: &str, expected_span: Span) {
    let error = Compiler::new().compile(program).unwrap_err();
    assert!(
        matches!(&error, CodegenError::InvalidClosure { reason: actual, span } if *actual == reason && *span == expected_span),
        "{error:?}"
    );
}

#[test]
fn checked_lifecycle_orders_layout_all_user_bodies_and_helpers_in_preorder() {
    for stage in 0..=6 {
        let mut child = closure(
            if stage == 0 { 99 } else { 1 },
            BindingOwner::Closure(CallableId(0)),
            TypedClosureBody::Expression(Box::new(number(if stage <= 4 {
                "bad-child"
            } else {
                "13"
            }))),
            Type::Number,
        );
        child.span = Span::new(301, 331);
        child.body_span = Span::new(311, 321);
        let mut parent = closure(
            0,
            BindingOwner::Function(FunctionId(0)),
            block(
                vec![statement(TypedStatementKind::Closure(child))],
                Some(number(if stage <= 3 { "bad-parent" } else { "11" })),
            ),
            Type::Number,
        );
        parent.body_span = Span::new(201, 291);
        if let TypedClosureBody::Block(body) = &mut parent.body {
            body.span = parent.body_span;
        }
        let mut sibling = closure(
            2,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(number(if stage <= 5 {
                "bad-sibling"
            } else {
                "17"
            }))),
            Type::Number,
        );
        sibling.body_span = Span::new(401, 421);
        let mut typed = program(vec![
            statement(TypedStatementKind::Closure(parent)),
            statement(TypedStatementKind::Closure(sibling)),
            TypedStatement {
                kind: TypedStatementKind::Return(Some(number(if stage <= 1 {
                    "bad-main"
                } else {
                    "2"
                }))),
                span: Span::new(101, 121),
            },
        ]);
        typed.functions.push(TypedFunction {
            id: FunctionId(1),
            name: "later".into(),
            params: vec![],
            return_type: Type::Number,
            contracts: vec![],
            body: vec![TypedStatement {
                kind: TypedStatementKind::Return(Some(number(if stage <= 2 {
                    "bad-later"
                } else {
                    "3"
                }))),
                span: Span::new(151, 171),
            }],
            span: Span::new(141, 181),
        });
        if stage == 0 {
            assert_invalid(&typed, "invalid callable identity", Span::new(301, 331));
        } else if stage < 6 {
            let (expected_function, expected_literal, expected_span) = match stage {
                1 => ("main", "bad-main", Span::new(101, 121)),
                2 => ("later", "bad-later", Span::new(151, 171)),
                3 => ("<closure>", "bad-parent", Span::new(201, 291)),
                4 => ("<closure>", "bad-child", Span::new(311, 321)),
                5 => ("<closure>", "bad-sibling", Span::new(401, 421)),
                _ => unreachable!(),
            };
            let error = Compiler::new().compile(&typed).unwrap_err();
            assert!(
                matches!(&error, CodegenError::InvalidNumberLiteral { function, literal, span }
                    if function == expected_function && literal == expected_literal && *span == expected_span),
                "stage {stage}: {error:?}"
            );
        } else {
            let bytes = Compiler::new().compile(&typed).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            let bodies = Parser::new(0)
                .parse_all(&bytes)
                .filter(|payload| matches!(payload, Ok(Payload::CodeSectionEntry(_))))
                .count();
            assert_eq!(bodies, 5);
            for (index, expected) in [2.0, 3.0, 11.0, 13.0, 17.0].into_iter().enumerate() {
                let constants: Vec<_> = operators(&bytes, index)
                    .into_iter()
                    .filter_map(|op| match op {
                        Operator::F64Const { value } => Some(f64::from_bits(value.bits())),
                        _ => None,
                    })
                    .collect();
                assert_eq!(constants, [expected], "body {index}");
            }
        }
    }
}

#[test]
fn parent_helper_retains_child_capture_and_dependency_facts_after_scope_exit() {
    let compiler = Compiler::new();
    let typed = compiler
        .analyze(
            "fn main() -> number {
                let parent = () => {
                    if true {
                        let n = 7;
                        let read = () => n;
                        let child = () => n + read();
                        child();
                    }
                    9
                };
                return parent();
            }",
        )
        .unwrap();
    let bytes = compiler.compile(&typed).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let child = operators(&bytes, 10);
    let calls: Vec<_> = child
        .iter()
        .filter_map(|op| match op {
            Operator::Call { function_index } => Some(*function_index),
            _ => None,
        })
        .collect();
    assert_eq!(calls, [9]);
    let offsets: Vec<_> = child
        .iter()
        .filter_map(|op| match op {
            Operator::I32Load { memarg } => Some(memarg.offset),
            _ => None,
        })
        .collect();
    assert_eq!(offsets, [0, 8]);
    assert!(child.iter().any(|op| matches!(op, Operator::F64Add)));
    let parent = operators(&bytes, 8);
    let child_call = parent
        .iter()
        .position(|op| matches!(op, Operator::Call { function_index: 10 }))
        .unwrap();
    let scope_exit = parent[child_call + 1..]
        .iter()
        .position(|op| matches!(op, Operator::End))
        .unwrap()
        + child_call
        + 1;
    assert!(parent[scope_exit + 1..].iter().any(
        |op| matches!(op, Operator::F64Const { value } if f64::from_bits(value.bits()) == 9.0)
    ));
    assert!(
        operators(&bytes, 9)
            .iter()
            .any(|op| matches!(op, Operator::F64Load { memarg } if memarg.offset == 0))
    );
}

#[test]
fn helper_body_and_result_errors_precede_exact_capture_and_dependency_completeness() {
    let compiler = Compiler::new();
    let original = compiler
        .analyze(
            "fn main() -> number {
                let n = 1;
                let first = () => 2;
                let candidate = () => n + first();
                return candidate();
            }",
        )
        .unwrap();
    for (keep_capture, keep_dependency) in [(true, false), (false, true), (true, true)] {
        for failure in 0..3 {
            let mut typed = original.clone();
            let TypedStatementKind::Closure(candidate) = &mut typed.functions[0].body[2].kind
            else {
                panic!()
            };
            if !keep_capture {
                candidate.captures.clear();
            }
            if !keep_dependency {
                candidate.dependencies.clear();
            }
            candidate.span = Span::new(501, 551);
            candidate.body_span = Span::new(521, 541);
            candidate.body = match failure {
                0 => TypedClosureBody::Expression(Box::new(number("bad-body"))),
                1 => block(vec![], None),
                2 => TypedClosureBody::Expression(Box::new(number("3"))),
                _ => unreachable!(),
            };
            match failure {
                0 => {
                    let error = compiler.compile(&typed).unwrap_err();
                    assert!(
                        matches!(&error, CodegenError::InvalidNumberLiteral { function, literal, span }
                            if function == "<closure>" && literal == "bad-body" && *span == Span::new(521, 541)),
                        "{error:?}"
                    );
                }
                1 => assert_invalid(&typed, "invalid closure result", Span::new(521, 541)),
                2 => assert_invalid(
                    &typed,
                    if keep_capture {
                        "invalid capture metadata"
                    } else {
                        "invalid callable dependency"
                    },
                    Span::new(501, 551),
                ),
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn recursive_contexts_keep_local_environment_pointers_and_only_allocator_writes_heap() {
    use was_compiler::{ExportOptions, OptimizationLevel};

    let source = r#"
        fn creator(seed: number, depth: number) -> number {
            let local = seed * 10;
            let first = () => { seed += 1; local += 10; seed + local };
            let second = () => { let leaf = () => first(); leaf() };
            mark(seed);
            if depth > 0 { creator(seed + 10, depth - 1); }
            return second();
        }
    "#;
    let mut baseline = None;
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        compiler.set_exports(ExportOptions::Only(vec!["creator".into()]));
        compiler
            .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
            .unwrap();
        compiler
            .register_host_function("unused", "env", "unused", vec![Type::Bool], Type::Void)
            .unwrap();
        let program = compiler.analyze(source).unwrap();
        let before_registry_edit = compiler.compile(&program).unwrap();
        compiler
            .register_host_function("later", "env", "later", vec![], Type::String)
            .unwrap();
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(bytes, before_registry_edit);
        if let Some(expected) = &baseline {
            assert_eq!(&bytes, expected);
        } else {
            baseline = Some(bytes.clone());
        }
        Validator::new().validate_all(&bytes).unwrap();
        let mut imports = vec![];
        let mut allocator = None;
        let mut body_count = 0;
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::ImportSection(section) => {
                    for import in section.into_imports() {
                        let import = import.unwrap();
                        imports.push((import.module, import.name));
                    }
                }
                Payload::ExportSection(section) => {
                    for export in section {
                        let export = export.unwrap();
                        if export.name == "__alloc" {
                            allocator = Some(export.index);
                        }
                    }
                }
                Payload::CodeSectionStart { count, .. } => body_count = count,
                Payload::TableSection(_) | Payload::StartSection { .. } => {
                    panic!("local callables add neither a table nor a start function")
                }
                _ => {}
            }
        }
        assert_eq!(imports, [("env", "mark"), ("env", "unused")]);
        let allocator = allocator.unwrap();
        assert_eq!(allocator, 3); // Two imports, then the sole user function.
        assert_eq!(body_count, 11); // User, seven memory helpers, three callables.
        for index in 0..body_count {
            let ops = operators(&bytes, index as usize);
            let writes: Vec<_> = ops
                .iter()
                .filter_map(|op| match op {
                    Operator::GlobalSet { global_index } => Some(*global_index),
                    _ => None,
                })
                .collect();
            if index == 1 {
                assert_eq!(writes, [0]);
            } else {
                assert!(writes.is_empty(), "body {index} must not reset the heap");
            }
        }
        let creator = operators(&bytes, 0);
        let allocations: Vec<_> = creator
            .iter()
            .enumerate()
            .filter_map(|(i, op)| {
                matches!(op, Operator::Call { function_index } if *function_index == allocator)
                    .then_some(i)
            })
            .collect();
        assert_eq!(allocations.len(), 4); // Two cells and two environments.
        let sizes: Vec<_> = allocations
            .iter()
            .map(|&i| match creator[i - 1] {
                Operator::I32Const { value } => value,
                _ => panic!("explicit allocation size"),
            })
            .collect();
        assert_eq!(sizes, [8, 8, 16, 8]);
        let Operator::LocalSet { local_index: env } = creator[allocations[3] + 1] else {
            panic!("environment pointer must be invocation-local");
        };
        let host_call = creator
            .iter()
            .position(|op| matches!(op, Operator::Call { function_index: 0 }))
            .unwrap();
        let recursive_call = creator
            .iter()
            .position(|op| matches!(op, Operator::Call { function_index: 2 }))
            .unwrap();
        let second_call = creator
            .iter()
            .rposition(|op| matches!(op, Operator::Call { function_index: 11 }))
            .unwrap();
        assert!(allocations[3] < host_call && host_call < recursive_call);
        assert!(recursive_call < second_call);
        assert!(
            matches!(creator[second_call - 1], Operator::LocalGet { local_index } if local_index == env)
        );
        // The nested leaf forwards the earlier callable's original live environment.
        let leaf = operators(&bytes, 10);
        assert!(leaf.windows(3).any(|ops| matches!(ops,
            [Operator::LocalGet { local_index: 0 }, Operator::I32Load { memarg },
             Operator::Call { function_index: 10 }] if memarg.offset == 0)));
    }
}

#[test]
fn nested_iteration_convergence_preserves_cells_contexts_and_full_module_order() {
    use wasmparser::ValType::{F64, I32};

    // main depends on later's inferred result, but its helpers must still come first.
    let source = r#"
        fn main() {
            for x in 1..3 {
                for y in [1, 2] {
                    let first = () => x;
                    let bridge = () => first();
                    let outer = () => {
                        let middle = () => {
                            let leaf = () => if y in 0..3 {
                                x + y + bridge() + 7 % 2
                            } else { 0 };
                            leaf()
                        };
                        middle()
                    };
                    outer();
                }
            }
            return later();
        }
        fn later() { let marker = () => 91; return marker(); }
    "#;
    let compiler = Compiler::new();
    let bytes = compiler.compile_source(source).unwrap();
    assert_eq!(compiler.compile_source(source).unwrap(), bytes);
    assert_eq!(
        compiler
            .compile(&compiler.analyze(source).unwrap())
            .unwrap(),
        bytes
    );
    Validator::new().validate_all(&bytes).unwrap();

    let mut types = vec![];
    let mut functions = vec![];
    let mut bodies = vec![];
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                types.extend(section.into_iter_err_on_gc_types().map(Result::unwrap));
            }
            Payload::FunctionSection(section) => {
                functions.extend(section.into_iter().map(Result::unwrap));
            }
            Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    // Two source functions; all seven memory helpers; remainder; Construct,
    // Validate, Contains; then first/bridge/outer/middle/leaf/marker in preorder.
    let signatures = [
        (vec![], vec![F64]),
        (vec![], vec![F64]),
        (vec![I32], vec![I32]),
        (vec![I32], vec![I32]),
        (vec![I32], vec![I32]),
        (vec![I32, F64], vec![I32]),
        (vec![I32], vec![F64]),
        (vec![I32, I32], vec![I32]),
        (vec![I32, I32], vec![I32]),
        (vec![F64, F64], vec![F64]),
        (vec![F64, F64, F64, I32], vec![I32]),
        (vec![I32], vec![I32]),
        (vec![F64, I32], vec![I32]),
        (vec![I32], vec![F64]),
        (vec![I32], vec![F64]),
        (vec![I32], vec![F64]),
        (vec![I32], vec![F64]),
        (vec![I32], vec![F64]),
        (vec![], vec![F64]),
    ];
    assert_eq!(functions.len(), 19);
    assert_eq!(bodies.len(), 19);
    for (index, (params, results)) in signatures.iter().enumerate() {
        let signature = &types[functions[index] as usize];
        assert_eq!(signature.params(), params, "function {index}");
        assert_eq!(signature.results(), results, "function {index}");
    }
    let calls = |index| {
        operators(&bytes, index)
            .iter()
            .filter_map(|op| match op {
                Operator::Call { function_index } => Some(*function_index),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // Exact call lists distinguish same-signature runtime helpers, not just valid Wasm.
    for (index, expected) in [
        (2, vec![]),
        (3, vec![]),
        (4, vec![]),
        (5, vec![4]),
        (6, vec![4]),
        (7, vec![3, 3]),
        (8, vec![4, 4]),
        (9, vec![]),
        (10, vec![2]),
        (11, vec![]),
        (12, vec![11]),
        (13, vec![]),
        (14, vec![13]),
        (15, vec![2, 16]),
        (16, vec![2, 17]),
        (17, vec![10, 12, 14, 9]),
        (18, vec![]),
        (1, vec![18]),
    ] {
        assert_eq!(calls(index), expected, "body {index}");
    }
    assert_eq!(calls(0), [10, 11, 2, 2, 4, 5, 2, 2, 2, 2, 15, 1]);
    assert!(
        matches!(operators(&bytes, 18)[0], Operator::F64Const { value } if value.bits() == 0x4056_c000_0000_0000)
    );

    let creator = operators(&bytes, 0);
    let allocations: Vec<_> = creator
        .iter()
        .enumerate()
        .filter_map(|(i, op)| matches!(op, Operator::Call { function_index: 2 }).then_some(i))
        .collect();
    let sizes: Vec<_> = allocations
        .iter()
        .map(|&i| match creator[i - 1] {
            Operator::I32Const { value } => value,
            _ => panic!("literal cell/environment/array allocation size"),
        })
        .collect();
    assert_eq!(sizes, [8, 24, 8, 8, 8, 24]);
    let allocation_local = |at| match creator[allocations[at] + 1] {
        Operator::LocalSet { local_index } | Operator::LocalTee { local_index } => local_index,
        _ => panic!("invocation-local allocation pointer"),
    };
    let x = allocation_local(0);
    let y = allocation_local(2);
    let first = allocation_local(3);
    let bridge = allocation_local(4);
    let outer = allocation_local(5);
    // The original yield cells go into both first and outer, never copied values.
    for (environment, pointer, offset) in [
        (first, x, 0),
        (bridge, first, 0),
        (outer, x, 0),
        (outer, y, 8),
        (outer, bridge, 16),
    ] {
        assert!(creator.windows(3).any(|ops| matches!(ops,
            [Operator::LocalGet { local_index: dest }, Operator::LocalGet { local_index: src },
             Operator::I32Store { memarg }]
            if *dest == environment && *src == pointer && memarg.offset == offset)));
    }
    // Both intermediate helpers allocate only the descendant environment and
    // forward original cell/context pointers in capture-before-context order.
    for index in [15, 16] {
        let ops = operators(&bytes, index);
        assert!(matches!(ops[0], Operator::I32Const { value: 24 }));
        let forwarded: Vec<_> = ops
            .windows(3)
            .filter_map(|ops| match ops {
                [
                    Operator::LocalGet { local_index: 0 },
                    Operator::I32Load { memarg: load },
                    Operator::I32Store { memarg: store },
                ] => Some((load.offset, store.offset)),
                _ => None,
            })
            .collect();
        assert_eq!(forwarded, [(0, 0), (8, 8), (16, 16)]);
        assert!(!ops.iter().any(|op| matches!(op, Operator::F64Store { .. })));
        // Hidden environment is parameter 0; only the new environment is local 1.
        assert_eq!(
            bodies[index]
                .get_locals_reader()
                .unwrap()
                .into_iter()
                .map(Result::unwrap)
                .collect::<Vec<_>>(),
            [(1, I32)]
        );
    }
    let leaf = operators(&bytes, 17);
    for offset in [0, 8] {
        assert!(leaf.windows(3).any(|ops| matches!(ops,
            [Operator::LocalGet { local_index: 0 }, Operator::I32Load { memarg },
             Operator::F64Load { .. }] if memarg.offset == offset)));
    }
    assert!(leaf.windows(3).any(|ops| matches!(ops,
        [Operator::LocalGet { local_index: 0 }, Operator::I32Load { memarg },
         Operator::Call { function_index: 14 }] if memarg.offset == 16)));
    for index in [1, 13, 14, 17, 18] {
        assert_eq!(bodies[index].get_locals_reader().unwrap().get_count(), 0);
    }
}

fn nested_iteration_fixture(outer_iterable: &str, inner_iterable: &str) -> TypedProgram {
    Compiler::new()
        .analyze(&format!(
            "fn main() {{
                for x in {outer_iterable} {{
                    let earlier = () => x;
                    let outer = () => {{
                        for y in {inner_iterable} {{
                            let middle = () => {{
                                let leaf = () => x + y + earlier();
                                leaf()
                            }};
                            middle();
                        }}
                        0
                    }};
                    outer();
                }}
            }}"
        ))
        .unwrap()
}

#[test]
fn nested_iteration_metadata_checks_original_owners_at_every_forwarding_boundary() {
    for (outer_iterable, inner_iterable) in [("1..3", "[1, 2]"), ("[1, 2]", "1..3")] {
        let original = nested_iteration_fixture(outer_iterable, inner_iterable);
        Validator::new()
            .validate_all(&Compiler::new().compile(&original).unwrap())
            .unwrap();
        for mutation in 0..14 {
            let mut forged = original.clone();
            let TypedStatementKind::For { body, .. } = &mut forged.functions[0].body[0].kind else {
                panic!();
            };
            let TypedStatementKind::Closure(outer) = &mut body[1].kind else {
                panic!();
            };
            let TypedClosureBody::Block(outer_body) = &mut outer.body else {
                panic!();
            };
            let TypedStatementKind::For { body, .. } = &mut outer_body.statements[0].kind else {
                panic!();
            };
            let TypedStatementKind::Closure(middle) = &mut body[0].kind else {
                panic!();
            };
            let TypedClosureBody::Block(middle_body) = &mut middle.body else {
                panic!();
            };
            let TypedStatementKind::Closure(leaf) = &mut middle_body.statements[0].kind else {
                panic!();
            };
            // x and y have the same owner-local ID; their owners, not that ID,
            // distinguish the two original read-only iteration bindings.
            assert_eq!(middle.captures[0].variable, middle.captures[1].variable);
            assert_ne!(middle.captures[0].owner, middle.captures[1].owner);
            // Missing inherited requirements fail when the child is declared;
            // unused extra requirements fail when the parent's body completes.
            let (reason, origin) = match mutation {
                0 => {
                    outer.captures.clear();
                    ("invalid capture metadata", middle.span)
                }
                1 => {
                    outer.captures[0].mutability = CaptureMutability::Mutable;
                    ("invalid capture metadata", outer.span)
                }
                2 => {
                    middle.captures.remove(0);
                    ("invalid capture metadata", outer.span)
                }
                3 => {
                    middle.captures[1].mutability = CaptureMutability::Mutable;
                    ("invalid capture metadata", middle.span)
                }
                4 => {
                    leaf.captures[0].mutability = CaptureMutability::Mutable;
                    ("invalid capture metadata", leaf.span)
                }
                5 => {
                    leaf.captures.remove(1);
                    ("invalid capture metadata", middle.span)
                }
                6 => {
                    // A real binding with the same numeric ID is not a substitute.
                    leaf.captures[0].owner = leaf.captures[1].owner;
                    ("invalid capture metadata", leaf.span)
                }
                7 => {
                    leaf.captures.swap(0, 1);
                    ("invalid capture metadata", leaf.span)
                }
                8 => {
                    middle.captures.push(middle.captures[1].clone());
                    ("invalid capture metadata", middle.span)
                }
                9 => {
                    outer.dependencies.clear();
                    ("invalid callable dependency", outer.span)
                }
                10 => {
                    middle.dependencies.clear();
                    ("invalid callable dependency", outer.span)
                }
                11 => {
                    leaf.dependencies.clear();
                    ("invalid callable dependency", middle.span)
                }
                12 => {
                    let origin = middle.span;
                    // Keep the binding's identity/type/initialization, but replace
                    // its original iteration provenance with an ordinary mutable let.
                    let TypedStatementKind::For {
                        variable,
                        name,
                        body,
                        ..
                    } = outer_body.statements.remove(0).kind
                    else {
                        panic!();
                    };
                    outer_body
                        .statements
                        .push(statement(TypedStatementKind::Let {
                            id: variable,
                            name,
                            ty: Type::Number,
                            value: number("1"),
                        }));
                    outer_body.statements.extend(body);
                    ("invalid capture metadata", origin)
                }
                13 => {
                    let origin = middle.span;
                    // The original binding still exists, but its iteration scope
                    // has ended before this descendant-requiring declaration.
                    let declarations_and_calls = std::mem::take(body);
                    outer_body.statements.extend(declarations_and_calls);
                    ("invalid capture metadata", origin)
                }
                _ => unreachable!(),
            };
            let error = Compiler::new().compile(&forged).unwrap_err();
            assert!(
                matches!(&error,
                CodegenError::InvalidClosure { reason: actual, span }
                if *actual == reason && *span == origin),
                "mutation {mutation}: {error:?}; expected {reason} at {origin:?}"
            );
        }
    }
}

#[test]
fn nested_iteration_original_provenance_rejects_writes_in_unreachable_leaf_bodies() {
    use was_compiler::ast::BinaryOp;

    for (outer_iterable, inner_iterable) in [("1..3", "[1, 2]"), ("[1, 2]", "1..3")] {
        let original = nested_iteration_fixture(outer_iterable, inner_iterable);
        for captured_index in [0, 1] {
            for compound in [
                None,
                Some(BinaryOp::Add),
                Some(BinaryOp::Subtract),
                Some(BinaryOp::Multiply),
                Some(BinaryOp::Divide),
                Some(BinaryOp::Remainder),
            ] {
                let mut forged = original.clone();
                let TypedStatementKind::For { body, .. } = &mut forged.functions[0].body[0].kind
                else {
                    panic!();
                };
                let TypedStatementKind::Closure(outer) = &mut body[1].kind else {
                    panic!();
                };
                let TypedClosureBody::Block(outer_body) = &mut outer.body else {
                    panic!();
                };
                let TypedStatementKind::For { body, .. } = &mut outer_body.statements[0].kind
                else {
                    panic!();
                };
                let TypedStatementKind::Closure(middle) = &mut body[0].kind else {
                    panic!();
                };
                let TypedClosureBody::Block(middle_body) = &mut middle.body else {
                    panic!();
                };
                let TypedStatementKind::Closure(leaf) = &mut middle_body.statements[0].kind else {
                    panic!();
                };
                let capture = &leaf.captures[captured_index];
                let id = capture.variable;
                let target = TypedExpr {
                    kind: TypedExprKind::Captured {
                        owner: capture.owner,
                        id,
                        name: if captured_index == 0 { "x" } else { "y" }.into(),
                    },
                    ty: Type::Number.into(),
                };
                let write = statement(if let Some(op) = compound {
                    TypedStatementKind::CompoundAssignment {
                        target,
                        op,
                        value: number("1"),
                    }
                } else {
                    TypedStatementKind::Assignment {
                        target,
                        value: number("1"),
                    }
                });
                // Retain the original expression to keep both capture and live
                // dependency requirements complete after replacing the leaf body.
                let TypedClosureBody::Expression(value) = &leaf.body else {
                    panic!();
                };
                leaf.body = block(
                    vec![
                        statement(TypedStatementKind::Return(Some(value.as_ref().clone()))),
                        write,
                    ],
                    None,
                );
                let error = Compiler::new().compile(&forged).unwrap_err();
                assert!(
                    matches!(error,
                    CodegenError::ReadOnlyVariable { id: actual, span: actual_span, .. }
                    if actual == id && actual_span == span()),
                    "{error:?}"
                );
            }
        }
    }
}

#[test]
fn iteration_cells_allocate_inside_the_yield_region_before_the_body() {
    for iterable in ["1..3", "[1, 2]"] {
        let source = format!(
            "fn main() {{ for x in {iterable} {{
                let f = () => x; f();
                if true && {{ if false {{ continue; }} else {{}} true }} {{
                    {{ if true {{ break; }} else {{}} }};
                }}
                continue;
            }} }}"
        );
        let bytes = Compiler::new().compile_source(&source).unwrap();
        let ops = operators(&bytes, 0);
        let loop_start = ops
            .iter()
            .position(|op| matches!(op, Operator::Loop { .. }))
            .unwrap();
        let exit = loop_start
            + ops[loop_start..]
                .iter()
                .position(|op| matches!(op, Operator::BrIf { relative_depth: 1 }))
                .unwrap();
        let yield_at = exit
            + ops[exit..]
                .iter()
                .position(|op| {
                    if iterable.starts_with('[') {
                        matches!(op, Operator::F64Load { .. })
                    } else {
                        matches!(op, Operator::F64ConvertI64S)
                    }
                })
                .unwrap();
        // Alloc is the first runtime helper after the sole user function.
        let allocations: Vec<_> = ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| matches!(op, Operator::Call { function_index: 1 }).then_some(i))
            .collect();
        let cell = *allocations.iter().find(|&&i| i > yield_at).unwrap();
        let env = *allocations.iter().find(|&&i| i > cell).unwrap();
        assert!(exit < yield_at && yield_at < cell && cell < env);
        assert!(matches!(ops[cell - 1], Operator::I32Const { value: 8 }));
        let Operator::LocalSet {
            local_index: cell_local,
        } = ops[cell + 1]
        else {
            panic!("cell destination");
        };
        assert!(
            matches!(ops[cell + 2], Operator::LocalGet { local_index } if local_index == cell_local)
        );
        assert!(
            matches!(ops[cell + 4], Operator::F64Store { memarg } if memarg.offset == 0 && memarg.align == 3)
        );
        assert!(
            ops[cell + 5..env]
                .iter()
                .any(|op| matches!(op, Operator::Block { .. }))
        );
        assert!(
            matches!(ops[env + 3], Operator::LocalGet { local_index } if local_index == cell_local)
        );
        assert!(matches!(ops[env + 4], Operator::I32Store { memarg } if memarg.offset == 0));
        // Distinguish nested continue/break from the final continue and backedge.
        let branches: Vec<_> = ops
            .iter()
            .filter_map(|op| match op {
                Operator::BrIf { relative_depth } => Some((true, *relative_depth)),
                Operator::Br { relative_depth } => Some((false, *relative_depth)),
                _ => None,
            })
            .collect();
        assert_eq!(
            branches,
            [(true, 1), (false, 3), (false, 5), (false, 0), (false, 0)]
        );
        let backedge = ops
            .iter()
            .rposition(|op| matches!(op, Operator::Br { .. }))
            .unwrap();
        assert!(matches!(
            ops[backedge - 6],
            Operator::Br { relative_depth: 0 }
        ));
        assert!(matches!(ops[backedge - 5], Operator::End)); // body latch
        let Operator::LocalGet {
            local_index: induction,
        } = ops[backedge - 4]
        else {
            panic!("induction read");
        };
        assert!(
            matches!(ops[backedge - 1], Operator::LocalSet { local_index } if local_index == induction)
        );
        if iterable.starts_with('[') {
            assert!(
                matches!(ops[exit - 3], Operator::LocalGet { local_index } if local_index == induction)
            );
            assert!(matches!(ops[backedge - 3], Operator::I32Const { value: 1 }));
            assert!(matches!(ops[backedge - 2], Operator::I32Add));
        } else {
            assert!(
                matches!(ops[yield_at - 1], Operator::LocalGet { local_index } if local_index == induction)
            );
            assert!(matches!(ops[backedge - 3], Operator::LocalGet { .. }));
            assert!(matches!(ops[backedge - 2], Operator::I64Add));
        }
        assert!(matches!(ops[backedge + 1], Operator::End)); // loop retest
        assert!(matches!(ops[backedge + 2], Operator::End)); // exit block
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
                .count(),
            if iterable.starts_with('[') { 3 } else { 2 }
        );
    }
}

#[test]
fn iteration_captures_reject_forged_writes_even_after_return_or_in_skipped_operands() {
    use was_compiler::ast::BinaryOp;
    let compiler = Compiler::new();
    let original = compiler
        .analyze("fn main() { for x in [1] { let f = () => x; } }")
        .unwrap();
    for compound in [
        None,
        Some(BinaryOp::Add),
        Some(BinaryOp::Subtract),
        Some(BinaryOp::Multiply),
        Some(BinaryOp::Divide),
        Some(BinaryOp::Remainder),
    ] {
        for skipped in [false, true] {
            let mut forged = original.clone();
            let TypedStatementKind::For { body, .. } = &mut forged.functions[0].body[0].kind else {
                panic!();
            };
            let TypedStatementKind::Closure(c) = &mut body[0].kind else {
                panic!();
            };
            let TypedClosureBody::Expression(target) = &c.body else {
                panic!();
            };
            let target = target.as_ref().clone();
            let id = c.captures[0].variable;
            let write = statement(if let Some(op) = compound {
                TypedStatementKind::CompoundAssignment {
                    target,
                    op,
                    value: number("1"),
                }
            } else {
                TypedStatementKind::Assignment {
                    target,
                    value: number("1"),
                }
            });
            c.body = if skipped {
                block(
                    vec![statement(TypedStatementKind::Expression(TypedExpr {
                        kind: TypedExprKind::Binary {
                            left: Box::new(TypedExpr {
                                kind: TypedExprKind::Bool(false),
                                ty: Type::Bool.into(),
                            }),
                            op: BinaryOp::And,
                            right: Box::new(TypedExpr {
                                kind: TypedExprKind::Block(TypedValueBlock {
                                    statements: vec![write],
                                    tail: Some(Box::new(TypedExpr {
                                        kind: TypedExprKind::Bool(true),
                                        ty: Type::Bool.into(),
                                    })),
                                    span: span(),
                                }),
                                ty: Type::Bool.into(),
                            }),
                        },
                        ty: Type::Bool.into(),
                    }))],
                    Some(number("0")),
                )
            } else {
                block(
                    vec![
                        statement(TypedStatementKind::Return(Some(number("0")))),
                        write,
                    ],
                    None,
                )
            };
            let error = compiler.compile(&forged).unwrap_err();
            assert!(
                matches!(error, CodegenError::ReadOnlyVariable { id: actual, span: actual_span, .. } if actual == id && actual_span == span()),
                "{error:?}"
            );
        }
    }
}

#[test]
fn iteration_capture_metadata_is_derived_from_live_original_provenance() {
    let compiler = Compiler::new();
    for iterable in ["1..3", "[1, 2]"] {
        let original = compiler
            .analyze(&format!(
                "fn main() {{ for x in {iterable} {{ let f = () => x; }} }}"
            ))
            .unwrap();
        for mutation in 0..6 {
            let mut forged = original.clone();
            let TypedStatementKind::For { variable, body, .. } =
                &mut forged.functions[0].body[0].kind
            else {
                panic!();
            };
            let TypedStatementKind::Closure(c) = &mut body[0].kind else {
                panic!();
            };
            let origin = c.span;
            match mutation {
                0 => c.captures[0].mutability = CaptureMutability::Mutable,
                1 => c.captures[0].ty = Type::Bool,
                2 => c.captures[0].owner = BindingOwner::Closure(c.id),
                3 => c.captures[0].variable = VariableId(999),
                4 => {
                    // Sparse identities are valid, but must agree at declaration,
                    // metadata and use sites; numeric order is not provenance.
                    *variable = VariableId(999);
                    c.captures[0].variable = *variable;
                    let TypedClosureBody::Expression(expr) = &mut c.body else {
                        panic!();
                    };
                    let TypedExprKind::Captured { id, .. } = &mut expr.kind else {
                        panic!();
                    };
                    *id = *variable;
                }
                5 => {
                    let declaration = body.remove(0);
                    forged.functions[0].body.push(declaration);
                }
                _ => unreachable!(),
            }
            if mutation == 4 {
                Validator::new()
                    .validate_all(&compiler.compile(&forged).unwrap())
                    .unwrap();
            } else {
                assert_invalid(&forged, "invalid capture metadata", origin);
            }
        }
        // The iterable is evaluated before the iteration binding initializes.
        let mut early = original.clone();
        let TypedStatementKind::For { iterable, body, .. } = &mut early.functions[0].body[0].kind
        else {
            panic!();
        };
        let declaration = body.remove(0);
        let TypedStatementKind::Closure(c) = &declaration.kind else {
            panic!();
        };
        let origin = c.span;
        let tail = iterable.as_ref().clone();
        **iterable = TypedExpr {
            ty: tail.ty,
            kind: TypedExprKind::Block(TypedValueBlock {
                statements: vec![declaration],
                tail: Some(Box::new(tail)),
                span: span(),
            }),
        };
        assert_invalid(&early, "invalid capture metadata", origin);
    }
}

#[test]
fn many_captures_with_repeated_accesses_preserve_canonical_slots_and_metadata_checks() {
    use std::fmt::Write;

    const CAPTURES: usize = 128;
    const ROUNDS: usize = 8;
    let mut source = String::from("fn main() -> number {");
    for index in 0..CAPTURES {
        write!(source, "let value{index} = {index};").unwrap();
    }
    source.push_str("let update = () => {");
    for _ in 0..ROUNDS {
        for index in (0..CAPTURES).rev() {
            write!(source, "value{index} = value{index} + 1;").unwrap();
        }
    }
    source.push_str("value0 }; return update(); }");

    let compiler = Compiler::new();
    let typed = compiler.analyze(&source).unwrap();
    let original = typed.clone();
    let bytes = compiler.compile(&typed).unwrap();
    assert_eq!(compiler.compile_source(&source).unwrap(), bytes);
    assert_eq!(compiler.compile(&typed).unwrap(), bytes);
    assert_eq!(typed, original);

    let helper = operators(&bytes, 8);
    let offsets: Vec<_> = helper
        .iter()
        .filter_map(|op| match op {
            Operator::I32Load { memarg } => Some(memarg.offset),
            _ => None,
        })
        .collect();
    let mut expected = Vec::new();
    for _ in 0..ROUNDS {
        for index in (0..CAPTURES).rev() {
            let offset = u64::try_from(index).unwrap() * 8;
            expected.extend([offset, offset]);
        }
    }
    expected.push(0);
    assert_eq!(offsets, expected);

    for mutation in 0..5 {
        let mut forged = typed.clone();
        let TypedStatementKind::Closure(closure) = &mut forged.functions[0].body[CAPTURES].kind
        else {
            panic!("closure")
        };
        let origin = match mutation {
            0 => {
                closure.captures.push(closure.captures[0].clone());
                closure.span
            }
            1 => {
                closure.captures.swap(0, CAPTURES - 1);
                closure.span
            }
            2 => {
                closure.captures.pop();
                let TypedClosureBody::Block(block) = &closure.body else {
                    panic!("block")
                };
                block.statements[0].span
            }
            3 => {
                let TypedClosureBody::Block(block) = &mut closure.body else {
                    panic!("block")
                };
                let TypedExprKind::Captured { owner, .. } = &mut block.tail.as_mut().unwrap().kind
                else {
                    panic!("capture")
                };
                *owner = BindingOwner::Closure(closure.id);
                block.span
            }
            _ => {
                let TypedClosureBody::Block(block) = &mut closure.body else {
                    panic!("block")
                };
                block.tail = Some(Box::new(number("0")));
                block.statements.clear();
                closure.span
            }
        };
        assert_invalid(&forged, "invalid capture metadata", origin);
    }
}

#[test]
fn wide_independent_siblings_keep_direct_helpers_and_lexical_dependency_checks() {
    const WIDTH: usize = 2048;
    let mut statements = Vec::with_capacity(WIDTH + 1);
    for id in 0..WIDTH {
        statements.push(statement(TypedStatementKind::Closure(closure(
            id,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(number("7"))),
            Type::Number,
        ))));
    }
    statements.push(statement(TypedStatementKind::Return(Some(lexical(
        WIDTH - 1,
        vec![],
        Type::Number,
    )))));
    let mut typed = program(statements);
    let bytes = Compiler::new().compile(&typed).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let bodies = Parser::new(0)
        .parse_all(&bytes)
        .filter(|payload| matches!(payload, Ok(Payload::CodeSectionEntry(_))))
        .count();
    assert_eq!(bodies, WIDTH + 1);
    assert!(operators(&bytes, 0).iter().any(|op| matches!(op, Operator::Call { function_index } if *function_index == u32::try_from(WIDTH).unwrap())));
    for payload in Parser::new(0).parse_all(&bytes) {
        assert!(!matches!(
            payload.unwrap(),
            Payload::MemorySection(_)
                | Payload::GlobalSection(_)
                | Payload::DataSection(_)
                | Payload::TableSection(_)
        ));
    }

    let TypedStatementKind::Closure(last) = &mut typed.functions[0].body[WIDTH - 1].kind else {
        panic!()
    };
    last.body = TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number)));
    last.dependencies = vec![CallableId(0)];
    let bytes = Compiler::new().compile(&typed).unwrap();
    assert!(
        operators(&bytes, WIDTH)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 1 }))
    );
    let TypedStatementKind::Closure(last) = &mut typed.functions[0].body[WIDTH - 1].kind else {
        panic!()
    };
    last.dependencies.clear();
    last.span = Span::new(301, 351);
    assert_invalid(&typed, "invalid callable dependency", Span::new(301, 351));
}

#[test]
fn missing_dependency_metadata_preserves_body_checks_and_distinct_error_origins() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let call_span = Span::new(101, 107);
    let mut call = lexical(0, vec![], Type::Number);
    let TypedExprKind::LexicalCall { span, .. } = &mut call.kind else {
        panic!()
    };
    *span = call_span;
    let mut second = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(call.clone())),
        Type::Number,
    );
    second.span = Span::new(81, 109);
    let fixture = |second| {
        program(vec![
            statement(TypedStatementKind::Closure(first.clone())),
            statement(TypedStatementKind::Closure(second)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ])
    };
    assert_invalid(
        &fixture(second.clone()),
        "invalid callable dependency",
        second.span,
    );
    second.body = block(
        vec![statement(TypedStatementKind::Expression(call.clone()))],
        Some(number("invalid")),
    );
    assert!(matches!(
        Compiler::new().compile(&fixture(second.clone())),
        Err(CodegenError::InvalidNumberLiteral { .. })
    ));

    // An earlier declaration in an exited sibling scope is not an omitted permission.
    let mut hidden = first;
    hidden.span = Span::new(41, 79);
    second.body = TypedClosureBody::Expression(Box::new(call));
    let typed = program(vec![
        statement(TypedStatementKind::If {
            cond: Box::new(TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            }),
            then: vec![statement(TypedStatementKind::Closure(hidden))],
            else_: None,
        }),
        statement(TypedStatementKind::Closure(second)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&typed, "invalid callable call", call_span);
}

#[test]
fn nested_missing_dependencies_keep_the_parent_declaration_error_origin() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let mut child = closure(
        2,
        BindingOwner::Closure(CallableId(1)),
        TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
        Type::Number,
    );
    child.dependencies = vec![CallableId(0)];
    child.span = Span::new(201, 231);
    let mut parent = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(child))],
            Some(number("1")),
        ),
        Type::Number,
    );
    parent.span = Span::new(151, 251);
    let typed = program(vec![
        statement(TypedStatementKind::Closure(first)),
        statement(TypedStatementKind::Closure(parent)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&typed, "invalid callable dependency", Span::new(151, 251));
}

#[test]
fn nested_calls_preserve_transitive_external_dependencies_and_source_preorder() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let mut child = closure(
        2,
        BindingOwner::Closure(CallableId(1)),
        TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
        Type::Number,
    );
    child.dependencies = vec![CallableId(0)];
    let mut parent = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(child))],
            Some(lexical(2, vec![], Type::Number)),
        ),
        Type::Number,
    );
    parent.dependencies = vec![CallableId(0)];
    let program = program(vec![
        statement(TypedStatementKind::Closure(first)),
        statement(TypedStatementKind::Closure(parent)),
        statement(TypedStatementKind::Return(Some(lexical(
            1,
            vec![],
            Type::Number,
        )))),
    ]);
    let bytes = Compiler::new().compile(&program).unwrap();
    for (body, target) in [(0, 2), (2, 3), (3, 1)] {
        assert!(operators(&bytes, body).iter().any(
            |op| matches!(op, Operator::Call { function_index } if *function_index == target)
        ));
    }
    let mut forged = program.clone();
    let TypedStatementKind::Closure(parent) = &mut forged.functions[0].body[1].kind else {
        panic!()
    };
    parent.dependencies.clear();
    assert_invalid(&forged, "invalid callable dependency", span());
}

#[test]
fn forged_identity_owner_and_capture_metadata_are_rejected_even_when_unused() {
    let good = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    for (mut declaration, reason) in [
        (good.clone(), "invalid callable identity"),
        (good.clone(), "invalid closure ownership"),
        (good.clone(), "invalid capture metadata"),
    ] {
        match reason {
            "invalid callable identity" => declaration.id = CallableId(8),
            "invalid closure ownership" => declaration.owner = BindingOwner::Closure(CallableId(0)),
            _ => declaration.captures.push(TypedCapture {
                owner: BindingOwner::Function(FunctionId(0)),
                variable: VariableId(0),
                ty: Type::Number,
                mutability: CaptureMutability::Mutable,
            }),
        }
        let p = program(vec![
            statement(TypedStatementKind::Return(Some(number("1")))),
            statement(TypedStatementKind::Closure(declaration)),
        ]);
        assert_invalid(&p, reason, span());
    }
    let duplicate = program(vec![
        statement(TypedStatementKind::Closure(good.clone())),
        statement(TypedStatementKind::Closure(good)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&duplicate, "invalid callable identity", span());
}

#[test]
fn lexical_calls_require_declaration_time_scope_and_canonical_exact_dependencies() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let before = program(vec![
        statement(TypedStatementKind::Expression(lexical(
            0,
            vec![],
            Type::Number,
        ))),
        statement(TypedStatementKind::Closure(first.clone())),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert_invalid(&before, "invalid callable call", span());
    let escaped = program(vec![
        statement(TypedStatementKind::If {
            cond: Box::new(TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            }),
            then: vec![statement(TypedStatementKind::Closure(first.clone()))],
            else_: None,
        }),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![],
            Type::Number,
        )))),
    ]);
    assert_invalid(&escaped, "invalid callable call", span());
    let self_call = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
        Type::Number,
    );
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(self_call)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid callable call",
        span(),
    );
    for dependencies in [
        vec![],
        vec![CallableId(0), CallableId(0)],
        vec![CallableId(1)],
        vec![CallableId(77)],
    ] {
        let mut second = closure(
            1,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number))),
            Type::Number,
        );
        second.dependencies = dependencies;
        assert_invalid(
            &program(vec![
                statement(TypedStatementKind::Closure(first.clone())),
                statement(TypedStatementKind::Closure(second)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]),
            "invalid callable dependency",
            span(),
        );
    }
    let mut unused = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    unused.dependencies = vec![CallableId(0)];
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(first)),
            statement(TypedStatementKind::Closure(unused)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid callable dependency",
        span(),
    );
}

#[test]
fn nested_helpers_plan_locals_and_scratch_without_polluting_the_creator() {
    let array = TypedExpr {
        kind: TypedExprKind::Array(vec![number("3")], was_compiler::ArrayElementType::Number),
        ty: Type::Array(was_compiler::ArrayElementType::Number).into(),
    };
    let nested = closure(
        1,
        BindingOwner::Closure(CallableId(0)),
        block(
            vec![statement(TypedStatementKind::Let {
                id: VariableId(0),
                name: "array".into(),
                ty: Type::Array(was_compiler::ArrayElementType::Number),
                value: array,
            })],
            Some(number("9")),
        ),
        Type::Number,
    );
    let outer = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(nested))],
            Some(lexical(1, vec![], Type::Number)),
        ),
        Type::Number,
    );
    let bytes = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(outer)),
            statement(TypedStatementKind::Return(Some(lexical(
                0,
                vec![],
                Type::Number,
            )))),
        ]))
        .unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let bodies: Vec<_> = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 10); // user, seven memory helpers, enclosing and nested helpers
    assert_eq!(bodies[0].get_locals_reader().unwrap().get_count(), 0);
    assert_eq!(bodies[8].get_locals_reader().unwrap().get_count(), 0);
    assert_eq!(
        bodies[9]
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
        [(1, wasmparser::ValType::I32), (1, wasmparser::ValType::I32)]
    );
    assert!(
        operators(&bytes, 8)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 9 }))
    );
    assert!(
        operators(&bytes, 9)
            .iter()
            .any(|op| matches!(op, Operator::LocalSet { local_index: 0 }))
    );
    assert!(
        operators(&bytes, 9)
            .iter()
            .any(|op| matches!(op, Operator::LocalSet { local_index: 1 }))
    );
}

#[test]
fn body_discovery_appends_helpers_after_memory_remainder_and_range_helpers() {
    use was_compiler::ast::BinaryOp;
    let remainder = TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(number("7")),
            op: BinaryOp::Remainder,
            right: Box::new(number("2")),
        },
        ty: Type::Number.into(),
    };
    let range = TypedExpr {
        kind: TypedExprKind::Range {
            start: Box::new(remainder),
            end: Box::new(number("3")),
            step: None,
            inclusive: true,
        },
        ty: Type::Range.into(),
    };
    let contains = TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(number("1")),
            op: BinaryOp::In,
            right: Box::new(range),
        },
        ty: Type::Bool.into(),
    };
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(contains)),
        Type::Bool,
    );
    let bytes = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(declaration)),
            statement(TypedStatementKind::Expression(lexical(
                0,
                vec![],
                Type::Bool,
            ))),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]))
        .unwrap();
    let calls: Vec<_> = operators(&bytes, 12)
        .iter()
        .filter_map(|op| match op {
            Operator::Call { function_index } => Some(*function_index),
            _ => None,
        })
        .collect();
    assert_eq!(calls, [8, 9, 11]); // remainder, range construction, membership
    assert!(
        operators(&bytes, 0)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 12 }))
    );
}

#[test]
fn unreachable_closure_tails_are_checked_against_the_signature() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Return(Some(number("7"))))],
            Some(TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool.into(),
            }),
        ),
        Type::Number,
    );
    let error = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(declaration)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]))
        .unwrap_err();
    assert!(
        matches!(error, CodegenError::TypeMismatch { function, expected: Type::Number, found: Type::Bool, span: actual } if function == "<closure>" && actual == span())
    );
}

#[test]
fn closure_signatures_validate_unused_handle_parameters_before_the_body() {
    for (ty, expected_calls) in [
        (Type::String, vec![2]),
        (Type::Array(was_compiler::ArrayElementType::Number), vec![3]),
        (Type::Range, vec![9]),
    ] {
        let mut declaration = closure(
            0,
            BindingOwner::Function(FunctionId(0)),
            TypedClosureBody::Expression(Box::new(number("7"))),
            Type::Number,
        );
        declaration.params.push(TypedParam {
            id: VariableId(41),
            name: "unused".into(),
            ty,
            span: span(),
        });
        let bytes = Compiler::new()
            .compile(&program(vec![
                statement(TypedStatementKind::Closure(declaration)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]))
            .unwrap();
        let helper = if ty == Type::Range { 10 } else { 8 };
        let ops = operators(&bytes, helper);
        assert!(matches!(
            ops.first(),
            Some(Operator::LocalGet { local_index: 0 })
        ));
        assert_eq!(
            ops.iter()
                .filter_map(|op| match op {
                    Operator::Call { function_index } => Some(*function_index),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            expected_calls
        );
        assert!(ops.iter().any(|op| matches!(op, Operator::Drop)));
        let body = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|p| match p.unwrap() {
                Payload::CodeSectionEntry(body) => Some(body),
                _ => None,
            })
            .nth(helper)
            .unwrap();
        assert_eq!(body.get_locals_reader().unwrap().get_count(), 0);
    }
}

#[test]
fn every_helper_is_checked_with_independent_value_and_loop_contexts() {
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("invalid"))),
        Type::Number,
    );
    let error = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Return(Some(number("1")))),
            statement(TypedStatementKind::Closure(declaration.clone())),
        ]))
        .unwrap_err();
    assert!(
        matches!(error, CodegenError::InvalidNumberLiteral { function, .. } if function == "<closure>")
    );
    declaration.body = TypedClosureBody::Expression(Box::new(TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(8),
            name: "outer".into(),
        },
        ty: Type::Number.into(),
    }));
    let p = program(vec![
        statement(TypedStatementKind::Let {
            id: VariableId(8),
            name: "outer".into(),
            ty: Type::Number,
            value: number("1"),
        }),
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert!(matches!(
        Compiler::new().compile(&p),
        Err(CodegenError::UnknownVariable {
            id: VariableId(8),
            ..
        })
    ));
    for transfer in [
        TypedStatementKind::Break { target: LoopId(3) },
        TypedStatementKind::Continue { target: LoopId(3) },
    ] {
        let declaration = closure(
            0,
            BindingOwner::Function(FunctionId(0)),
            block(vec![statement(transfer)], None),
            Type::Void,
        );
        let p = program(vec![
            statement(TypedStatementKind::While {
                id: LoopId(3),
                cond: Box::new(TypedExpr {
                    kind: TypedExprKind::Bool(false),
                    ty: Type::Bool.into(),
                }),
                body: vec![statement(TypedStatementKind::Closure(declaration))],
            }),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]);
        assert!(matches!(
            Compiler::new().compile(&p),
            Err(CodegenError::InvalidLoopTarget {
                target: LoopId(3),
                ..
            })
        ));
    }
}

#[test]
fn lexical_call_checks_arity_result_and_all_arguments_after_divergence() {
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    declaration.params = vec![
        TypedParam {
            id: VariableId(31),
            name: "a".into(),
            ty: Type::Number,
            span: span(),
        },
        TypedParam {
            id: VariableId(55),
            name: "b".into(),
            ty: Type::Number,
            span: span(),
        },
    ];
    for call in [
        lexical(0, vec![], Type::Number),
        lexical(0, vec![number("1"), number("2")], Type::Bool),
    ] {
        assert_invalid(
            &program(vec![
                statement(TypedStatementKind::Closure(declaration.clone())),
                statement(TypedStatementKind::Expression(call)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]),
            "invalid callable call",
            span(),
        );
    }
    let divergent = TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![],
        },
        ty: ExprResult::Never,
    };
    let mut call = lexical(0, vec![divergent, number("invalid")], Type::Number);
    call.ty = ExprResult::Never;
    let p = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Expression(call)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    assert!(matches!(
        Compiler::new().compile(&p),
        Err(CodegenError::InvalidNumberLiteral { .. })
    ));
}

#[test]
fn closure_result_checks_returns_tails_fallthrough_and_never_only_bodies() {
    let divergent = TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(0),
            body: vec![],
        },
        ty: ExprResult::Never,
    };
    for (body, result) in [
        (
            TypedClosureBody::Expression(Box::new(divergent)),
            Type::Number,
        ),
        (block(vec![], None), Type::Number),
    ] {
        let declaration = closure(0, BindingOwner::Function(FunctionId(0)), body, result);
        assert_invalid(
            &program(vec![
                statement(TypedStatementKind::Closure(declaration)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]),
            "invalid closure result",
            span(),
        );
    }
    for body in [
        block(
            vec![statement(TypedStatementKind::Return(Some(number("7"))))],
            None,
        ),
        block(
            vec![statement(TypedStatementKind::Return(Some(number("7"))))],
            Some(number("9")),
        ),
    ] {
        let declaration = closure(0, BindingOwner::Function(FunctionId(0)), body, Type::Number);
        let bytes = Compiler::new()
            .compile(&program(vec![
                statement(TypedStatementKind::Closure(declaration)),
                statement(TypedStatementKind::Return(Some(number("1")))),
            ]))
            .unwrap();
        assert!(
            operators(&bytes, 1)
                .iter()
                .any(|op| matches!(op, Operator::Return))
        );
    }
    let void = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Loop {
                id: LoopId(0),
                body: vec![],
            })],
            None,
        ),
        Type::Void,
    );
    let bytes = Compiler::new()
        .compile(&program(vec![
            statement(TypedStatementKind::Closure(void)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]))
        .unwrap();
    Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn integrated_source_retains_named_calls_normalized_defaults_contracts_and_owner_local_loops() {
    use was_compiler::{CompilerOptions, ExportOptions, OptimizationLevel};
    for source in [
        "fn named(a: number, b: number = a + 1) -> number requires a >= 0 { return a + b; } fn main() -> number { let f = (x: number) => named(a: x); return f(3); }",
        "fn main() -> number { let f = (n: number) => { let a = [1, 2]; let sum = 0; for x in a { sum += x; } for x in 1..=n { if x == 2 { continue; } sum += x; } sum }; return f(3); }",
        "fn main() -> number { let a = () => 1; let b = () => { let c = () => a(); c() }; return { let a = () => 2; b() }; }",
        "fn main() -> number { let f = () => loop { break { let g = () => 7; g() }; }; return f(); }",
        "fn main() -> number { let f = () => match true { true => { let g = () => 7; g() }, false => 9 }; return f(); }",
        "fn named(x: number) -> number { return x; } fn main() -> number { let f = (x: number) => x |> named; return f(3); }",
    ] {
        let typed = Compiler::new().analyze(source).unwrap();
        let baseline = Compiler::new().compile(&typed).unwrap();
        Validator::new().validate_all(&baseline).unwrap();
        for optimization in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let compiler = Compiler::with_options(CompilerOptions {
                optimization_level: optimization,
            });
            assert_eq!(compiler.compile(&typed).unwrap(), baseline, "{source}");
        }
        let hidden =
            was_compiler::codegen::compile_with_exports(&typed, &ExportOptions::None).unwrap();
        Validator::new().validate_all(&hidden).unwrap();
        let code_count = |bytes: &[u8]| {
            Parser::new(0)
                .parse_all(bytes)
                .filter(|p| matches!(p, Ok(Payload::CodeSectionEntry(_))))
                .count()
        };
        assert_eq!(code_count(&baseline), code_count(&hidden));
    }
}

#[test]
fn lexical_permission_is_not_global_existence_and_calls_keep_their_own_spans() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    let call_span = Span::new(101, 107);
    let call = TypedExpr {
        kind: TypedExprKind::LexicalCall {
            callable: CallableId(0),
            args: vec![],
            span: call_span,
        },
        ty: Type::Number.into(),
    };
    p.functions.push(TypedFunction {
        id: FunctionId(1),
        name: "other".into(),
        params: vec![],
        return_type: Type::Number,
        contracts: vec![],
        body: vec![statement(TypedStatementKind::Return(Some(call)))],
        span: span(),
    });
    assert_invalid(&p, "invalid callable call", call_span);
    let unknown = TypedExpr {
        kind: TypedExprKind::LexicalCall {
            callable: CallableId(77),
            args: vec![],
            span: call_span,
        },
        ty: Type::Number.into(),
    };
    assert_invalid(
        &program(vec![statement(TypedStatementKind::Return(Some(unknown)))]),
        "invalid callable call",
        call_span,
    );
}

#[test]
fn malformed_dependency_order_and_nested_ownership_have_stable_declaration_origins() {
    let first = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let second = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("8"))),
        Type::Number,
    );
    let mut third = closure(
        2,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("9"))),
        Type::Number,
    );
    third.span = Span::new(110, 149);
    third.dependencies = vec![CallableId(1), CallableId(0)];
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(first)),
            statement(TypedStatementKind::Closure(second)),
            statement(TypedStatementKind::Closure(third)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid callable dependency",
        Span::new(110, 149),
    );
    let mut nested = closure(
        1,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    nested.span = Span::new(201, 231);
    let outer = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![statement(TypedStatementKind::Closure(nested))],
            Some(number("1")),
        ),
        Type::Number,
    );
    assert_invalid(
        &program(vec![
            statement(TypedStatementKind::Closure(outer)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]),
        "invalid closure ownership",
        Span::new(201, 231),
    );
}

#[test]
fn lexical_calls_are_rejected_by_contract_and_default_whitelists() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(TypedExpr {
            kind: TypedExprKind::Bool(true),
            ty: Type::Bool.into(),
        })),
        Type::Bool,
    );
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(number("1")))),
    ]);
    p.functions[0].contracts.push(TypedContract {
        value: lexical(0, vec![], Type::Bool),
        span: Span::new(81, 93),
    });
    assert!(
        matches!(Compiler::new().compile(&p), Err(CodegenError::InvalidContractExpression { reason: "contract expression is outside the pure-expression whitelist", span: actual }) if actual == Span::new(81, 93))
    );
    let mut typed = Compiler::new().analyze("fn named(a: number = 1) -> number { return a; } fn main() -> number { let f = () => 7; return named(); }").unwrap();
    let TypedStatementKind::Return(Some(TypedExpr {
        kind: TypedExprKind::NormalizedCall(sequence),
        ..
    })) = &mut typed.functions[1].body[1].kind
    else {
        panic!("normalized default")
    };
    sequence.bindings[0].value = lexical(0, vec![], Type::Number);
    assert!(matches!(
        Compiler::new().compile(&typed),
        Err(CodegenError::InvalidNormalizedCall {
            reason: "default initializer is outside the pure-expression whitelist",
            ..
        })
    ));
}

#[test]
fn helper_signatures_have_only_explicit_parameters_and_sparse_ids_are_owner_local() {
    let read = TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(40),
            name: "x".into(),
        },
        ty: Type::Number.into(),
    };
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(read)),
        Type::Number,
    );
    declaration.params = vec![
        TypedParam {
            id: VariableId(40),
            name: "x".into(),
            ty: Type::Number,
            span: span(),
        },
        TypedParam {
            id: VariableId(90),
            name: "flag".into(),
            ty: Type::Bool,
            span: span(),
        },
    ];
    let p = program(vec![
        statement(TypedStatementKind::Let {
            id: VariableId(40),
            name: "creator".into(),
            ty: Type::Number,
            value: number("3"),
        }),
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![
                number("8"),
                TypedExpr {
                    kind: TypedExprKind::Bool(true),
                    ty: Type::Bool.into(),
                },
            ],
            Type::Number,
        )))),
    ]);
    let bytes = Compiler::new().compile(&p).unwrap();
    let signatures: Vec<_> = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::TypeSection(types) => Some(types),
            _ => None,
        })
        .flat_map(|types| types.into_iter_err_on_gc_types().map(Result::unwrap))
        .collect();
    assert_eq!(signatures.len(), 2);
    assert_eq!(
        signatures[1].params(),
        [wasmparser::ValType::F64, wasmparser::ValType::I32]
    );
    assert_eq!(signatures[1].results(), [wasmparser::ValType::F64]);
    assert!(matches!(
        operators(&bytes, 1).as_slice(),
        [Operator::LocalGet { local_index: 0 }, Operator::End]
    ));
    let ops = operators(&bytes, 0);
    let call_position = ops
        .iter()
        .position(|op| matches!(op, Operator::Call { function_index: 1 }))
        .unwrap();
    assert!(
        matches!(&ops[call_position - 2], Operator::F64Const { value } if value.bits() == 8f64.to_bits())
    );
    assert!(matches!(
        &ops[call_position - 1],
        Operator::I32Const { value: 1 }
    ));
}

#[test]
fn every_concrete_body_result_and_void_compiles_without_exporting_helpers() {
    for body in [
        "1",
        "true",
        "\"é\"",
        "[1, 2]",
        "1..3",
        "{}",
        "{ return; }",
        "{ if true { return 1; } 2 }",
        "loop { break [1]; }",
        "if true { \"one\" } else { \"two\" }",
    ] {
        let source = format!("fn main() {{ let f = () => {body}; f(); }}");
        let typed = Compiler::new().analyze(&source).unwrap();
        let bytes = Compiler::new().compile(&typed).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        for payload in Parser::new(0).parse_all(&bytes) {
            if let Payload::ExportSection(exports) = payload.unwrap() {
                let names: Vec<_> = exports.into_iter().map(|e| e.unwrap().name).collect();
                assert!(
                    names
                        .iter()
                        .all(|name| ["main", "memory", "__alloc"].contains(name))
                );
            }
        }
    }
}

#[test]
fn helpers_reject_duplicate_parameters_uninitialized_locals_and_read_only_writes() {
    let mut declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    declaration.params = vec![
        TypedParam {
            id: VariableId(3),
            name: "x".into(),
            ty: Type::Number,
            span: span(),
        },
        TypedParam {
            id: VariableId(3),
            name: "y".into(),
            ty: Type::Number,
            span: span(),
        },
    ];
    let fixture = |declaration| {
        program(vec![
            statement(TypedStatementKind::Closure(declaration)),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ])
    };
    assert!(
        matches!(Compiler::new().compile(&fixture(declaration.clone())), Err(CodegenError::DuplicateVariableId { function, id: VariableId(3), .. }) if function == "<closure>")
    );
    declaration.params.truncate(1);
    declaration.params[0].ty = Type::Void;
    assert!(
        matches!(Compiler::new().compile(&fixture(declaration)), Err(CodegenError::VoidParameter { function, index: 0 }) if function == "<closure>")
    );
    let read = TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(8),
            name: "later".into(),
        },
        ty: Type::Number.into(),
    };
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        block(
            vec![
                statement(TypedStatementKind::Expression(read)),
                statement(TypedStatementKind::Let {
                    id: VariableId(8),
                    name: "later".into(),
                    ty: Type::Number,
                    value: number("1"),
                }),
            ],
            Some(number("7")),
        ),
        Type::Number,
    );
    assert!(
        matches!(Compiler::new().compile(&fixture(declaration)), Err(CodegenError::UninitializedVariable { function, id: VariableId(8), .. }) if function == "<closure>")
    );
    let mut typed = Compiler::new()
        .analyze("fn main() { let f = () => { for x in [1] { let y = x; } }; }")
        .unwrap();
    let TypedStatementKind::Closure(declaration) = &mut typed.functions[0].body[0].kind else {
        panic!()
    };
    let TypedClosureBody::Block(block) = &mut declaration.body else {
        panic!()
    };
    let TypedStatementKind::For { variable, body, .. } = &mut block.statements[0].kind else {
        panic!()
    };
    body.push(statement(TypedStatementKind::Assignment {
        target: TypedExpr {
            kind: TypedExprKind::Identifier {
                id: *variable,
                name: "x".into(),
            },
            ty: Type::Number.into(),
        },
        value: number("2"),
    }));
    assert!(
        matches!(Compiler::new().compile(&typed), Err(CodegenError::ReadOnlyVariable { function, .. }) if function == "<closure>")
    );
}

#[test]
fn helper_normalized_bindings_keep_their_final_call_only_read_permission() {
    let mut typed = Compiler::new()
        .analyze(
            "fn named(a: number = 1) -> number { return a; } fn main() { let f = () => named(); }",
        )
        .unwrap();
    let TypedStatementKind::Closure(declaration) = &mut typed.functions[1].body[0].kind else {
        panic!()
    };
    let old = std::mem::replace(&mut declaration.body, block(vec![], None));
    let TypedClosureBody::Expression(call) = old else {
        panic!()
    };
    let TypedExprKind::NormalizedCall(sequence) = &call.kind else {
        panic!()
    };
    let id = sequence.bindings[0].id;
    declaration.body = block(
        vec![statement(TypedStatementKind::Expression(*call))],
        Some(TypedExpr {
            kind: TypedExprKind::Identifier {
                id,
                name: "forged".into(),
            },
            ty: Type::Number.into(),
        }),
    );
    assert!(matches!(
        Compiler::new().compile(&typed),
        Err(CodegenError::InvalidNormalizedCall {
            reason: "synthetic binding is only readable by its final call",
            ..
        })
    ));
}

#[test]
fn public_typed_callable_compiles_to_unexported_direct_helper_without_memory() {
    let declaration = closure(
        0,
        BindingOwner::Function(FunctionId(0)),
        TypedClosureBody::Expression(Box::new(number("7"))),
        Type::Number,
    );
    let program = program(vec![
        statement(TypedStatementKind::Closure(declaration)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![],
            Type::Number,
        )))),
    ]);
    let original = program.clone();
    let bytes = Compiler::new().compile(&program).unwrap();
    assert_eq!(program, original);
    assert!(
        operators(&bytes, 0)
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 1 }))
    );
    assert!(
        operators(&bytes, 1)
            .iter()
            .any(|op| matches!(op, Operator::F64Const { value } if value.bits() == 7f64.to_bits()))
    );
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::MemorySection(_)
            | Payload::GlobalSection(_)
            | Payload::DataSection(_)
            | Payload::TableSection(_) => panic!("callable-only storage"),
            Payload::ExportSection(exports) => assert_eq!(
                exports
                    .into_iter()
                    .map(|e| e.unwrap().name)
                    .collect::<Vec<_>>(),
                ["main"]
            ),
            _ => {}
        }
    }
}

#[test]
fn ancestor_capture_is_forwarded_without_allocating_another_cell() {
    let owner = BindingOwner::Function(FunctionId(0));
    let capture = TypedCapture {
        owner,
        variable: VariableId(71),
        ty: Type::Number,
        mutability: CaptureMutability::Mutable,
    };
    let mut inner = closure(
        1,
        BindingOwner::Closure(CallableId(0)),
        TypedClosureBody::Expression(Box::new(TypedExpr {
            kind: TypedExprKind::Captured {
                owner,
                id: VariableId(71),
                name: "value".into(),
            },
            ty: Type::Number.into(),
        })),
        Type::Number,
    );
    inner.captures.push(capture.clone());
    let mut outer = closure(
        0,
        owner,
        block(
            vec![statement(TypedStatementKind::Closure(inner))],
            Some(lexical(1, vec![], Type::Number)),
        ),
        Type::Number,
    );
    outer.captures.push(capture);
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(outer)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![],
            Type::Number,
        )))),
    ]);
    p.functions[0].params.push(TypedParam {
        id: VariableId(71),
        name: "value".into(),
        ty: Type::Number,
        span: span(),
    });
    let bytes = Compiler::new().compile(&p).unwrap();
    let outer_ops = operators(&bytes, 8);
    assert_eq!(
        outer_ops
            .iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
            .count(),
        1
    );
    assert!(outer_ops.windows(2).any(|ops| matches!(ops, [Operator::LocalGet { local_index: 0 }, Operator::I32Load { memarg }] if memarg.offset == 0)));
    assert!(
        outer_ops
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 9 }))
    );
    let inner_ops = operators(&bytes, 9);
    assert!(matches!(
        inner_ops[0],
        Operator::LocalGet { local_index: 0 }
    ));
    assert!(matches!(inner_ops[1], Operator::I32Load { memarg } if memarg.offset == 0));
    assert!(matches!(inner_ops[2], Operator::F64Load { .. }));
}

fn transitive_context_fixture() -> TypedProgram {
    let root = BindingOwner::Function(FunctionId(0));
    let middle = BindingOwner::Closure(CallableId(2));
    let captured = |owner| TypedExpr {
        kind: TypedExprKind::Captured {
            owner,
            id: VariableId(91),
            name: "value".into(),
        },
        ty: Type::Number.into(),
    };
    let metadata = |owner| TypedCapture {
        owner,
        variable: VariableId(91),
        ty: Type::Number,
        mutability: CaptureMutability::Mutable,
    };
    let free = closure(
        0,
        root,
        TypedClosureBody::Expression(Box::new(number("1"))),
        Type::Number,
    );
    let mut first = closure(
        1,
        root,
        TypedClosureBody::Expression(Box::new(captured(root))),
        Type::Number,
    );
    first.captures = vec![metadata(root)];
    let mut inner = closure(
        3,
        middle,
        block(
            vec![
                statement(TypedStatementKind::Expression(lexical(
                    0,
                    vec![],
                    Type::Number,
                ))),
                statement(TypedStatementKind::Expression(lexical(
                    1,
                    vec![],
                    Type::Number,
                ))),
                statement(TypedStatementKind::Expression(captured(root))),
                statement(TypedStatementKind::Expression(TypedExpr {
                    kind: TypedExprKind::Captured {
                        owner: middle,
                        id: VariableId(7),
                        name: "own_local".into(),
                    },
                    ty: Type::Number.into(),
                })),
            ],
            Some(captured(middle)),
        ),
        Type::Number,
    );
    inner.captures = vec![
        metadata(root),
        metadata(middle),
        TypedCapture {
            owner: middle,
            variable: VariableId(7),
            ty: Type::Number,
            mutability: CaptureMutability::Mutable,
        },
    ];
    inner.dependencies = vec![CallableId(0), CallableId(1)];
    let mut second = closure(
        2,
        root,
        block(
            vec![
                statement(TypedStatementKind::Let {
                    id: VariableId(7),
                    name: "own_local".into(),
                    ty: Type::Number,
                    value: number("3"),
                }),
                statement(TypedStatementKind::Closure(inner)),
            ],
            Some(lexical(3, vec![], Type::Number)),
        ),
        Type::Number,
    );
    second.params.push(TypedParam {
        id: VariableId(91),
        name: "own".into(),
        ty: Type::Number,
        span: span(),
    });
    second.captures = vec![metadata(root)];
    second.dependencies = vec![CallableId(0), CallableId(1)];
    let mut leaf = closure(
        5,
        BindingOwner::Closure(CallableId(4)),
        TypedClosureBody::Expression(Box::new(lexical(2, vec![number("5")], Type::Number))),
        Type::Number,
    );
    leaf.dependencies = vec![CallableId(2)];
    let mut wrapper = closure(
        4,
        root,
        block(
            vec![statement(TypedStatementKind::Closure(leaf))],
            Some(lexical(5, vec![], Type::Number)),
        ),
        Type::Number,
    );
    wrapper.dependencies = vec![CallableId(2)];
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(free)),
        statement(TypedStatementKind::Closure(first)),
        statement(TypedStatementKind::Closure(second)),
        statement(TypedStatementKind::Closure(wrapper)),
        statement(TypedStatementKind::Return(Some(lexical(
            4,
            vec![],
            Type::Number,
        )))),
    ]);
    p.functions[0].params.push(TypedParam {
        id: VariableId(91),
        name: "root".into(),
        ty: Type::Number,
        span: span(),
    });
    p
}

#[test]
fn transitive_contexts_keep_capture_slots_before_live_dependency_pointers() {
    let bytes = Compiler::new()
        .compile(&transitive_context_fixture())
        .unwrap();
    let count_alloc = |index| {
        operators(&bytes, index)
            .iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
            .count()
    };
    assert_eq!(count_alloc(0), 4); // One root cell and three nonempty declarations.
    assert_eq!(count_alloc(10), 3); // Own parameter/local cells and the nested environment.
    assert_eq!(count_alloc(11), 0);
    assert_eq!(count_alloc(12), 1); // Forward the existing second environment to the leaf.
    assert_eq!(count_alloc(13), 0);
    let stores: Vec<_> = operators(&bytes, 10)
        .iter()
        .filter_map(|op| match op {
            Operator::I32Store { memarg } => Some(memarg.offset),
            _ => None,
        })
        .collect();
    assert_eq!(stores, [0, 8, 16, 24]);
    let inner = operators(&bytes, 11);
    assert!(inner.windows(3).any(|ops| matches!(ops,
        [Operator::LocalGet { local_index: 0 }, Operator::I32Load { memarg }, Operator::Call { function_index: 9 }] if memarg.offset == 24)));
    assert!(
        inner
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 8 }))
    );
    let leaf = operators(&bytes, 13);
    assert!(matches!(leaf[0], Operator::LocalGet { local_index: 0 }));
    assert!(matches!(leaf[1], Operator::I32Load { memarg } if memarg.offset == 0));
    assert!(
        leaf.iter()
            .any(|op| matches!(op, Operator::Call { function_index: 10 }))
    );
    let mut types = vec![];
    let mut functions = vec![];
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                types.extend(section.into_iter_err_on_gc_types().map(Result::unwrap))
            }
            Payload::FunctionSection(section) => {
                functions.extend(section.into_iter().map(Result::unwrap))
            }
            Payload::TableSection(_) => panic!("lexical calls must remain direct"),
            _ => {}
        }
    }
    for (index, params) in [
        (8, vec![]),
        (9, vec![wasmparser::ValType::I32]),
        (10, vec![wasmparser::ValType::I32, wasmparser::ValType::F64]),
        (11, vec![wasmparser::ValType::I32]),
        (12, vec![wasmparser::ValType::I32]),
        (13, vec![wasmparser::ValType::I32]),
    ] {
        assert_eq!(types[functions[index] as usize].params(), params);
    }
}

#[test]
fn transitive_requirements_reject_missing_extra_noncanonical_and_wrong_context_metadata() {
    let fixture = transitive_context_fixture();
    for mutation in 0..9 {
        let mut forged = fixture.clone();
        let TypedStatementKind::Closure(second) = &mut forged.functions[0].body[2].kind else {
            panic!()
        };
        let origin = second.span;
        let TypedClosureBody::Block(body) = &mut second.body else {
            panic!()
        };
        let TypedStatementKind::Closure(inner) = &mut body.statements[1].kind else {
            panic!()
        };
        let reason = match mutation {
            0 => {
                second.captures.clear();
                "invalid capture metadata"
            }
            1 => {
                inner.captures.swap(0, 1);
                "invalid capture metadata"
            }
            2 => {
                inner.captures.push(inner.captures[1].clone());
                "invalid capture metadata"
            }
            3 => {
                inner.captures[0].ty = Type::Bool;
                "invalid capture metadata"
            }
            4 => {
                inner.captures[0].owner = BindingOwner::Closure(CallableId(1));
                "invalid capture metadata"
            }
            5 => {
                second.dependencies.clear();
                "invalid callable dependency"
            }
            6 => {
                inner.dependencies.reverse();
                "invalid callable dependency"
            }
            7 => {
                inner.dependencies.push(CallableId(1));
                "invalid callable dependency"
            }
            8 => {
                inner.dependencies.push(CallableId(2));
                "invalid callable dependency"
            }
            _ => unreachable!(),
        };
        assert_invalid(&forged, reason, origin);
    }
    let mut extra = fixture.clone();
    extra.functions[0].params.push(TypedParam {
        id: VariableId(7),
        name: "unused".into(),
        ty: Type::Bool,
        span: span(),
    });
    let TypedStatementKind::Closure(second) = &mut extra.functions[0].body[2].kind else {
        panic!()
    };
    second.captures.push(TypedCapture {
        owner: second.owner,
        variable: VariableId(7),
        ty: Type::Bool,
        mutability: CaptureMutability::Mutable,
    });
    assert_invalid(&extra, "invalid capture metadata", span());

    // The callable still exists and needs context, but its declaration is no longer visible.
    let mut hidden = fixture.clone();
    let first = hidden.functions[0].body.remove(1);
    hidden.functions[0].body.insert(
        1,
        statement(TypedStatementKind::If {
            cond: Box::new(TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            }),
            then: vec![first],
            else_: None,
        }),
    );
    assert_invalid(&hidden, "invalid callable dependency", span());
}

#[test]
fn ancestor_forwarding_preserves_every_concrete_type_through_three_intermediates() {
    for ty in [
        Type::Number,
        Type::Bool,
        Type::String,
        Type::Array(was_compiler::ArrayElementType::Number),
        Type::Range,
    ] {
        let owner = BindingOwner::Function(FunctionId(0));
        let capture = TypedCapture {
            owner,
            variable: VariableId(73),
            ty,
            mutability: CaptureMutability::Mutable,
        };
        let mut child = closure(
            3,
            BindingOwner::Closure(CallableId(2)),
            TypedClosureBody::Expression(Box::new(TypedExpr {
                kind: TypedExprKind::Captured {
                    owner,
                    id: VariableId(73),
                    name: "root".into(),
                },
                ty: ty.into(),
            })),
            ty,
        );
        child.captures.push(capture.clone());
        for id in (0..3).rev() {
            let mut parent = closure(
                id,
                if id == 0 {
                    owner
                } else {
                    BindingOwner::Closure(CallableId(id - 1))
                },
                block(
                    vec![statement(TypedStatementKind::Closure(child))],
                    Some(lexical(id + 1, vec![], ty)),
                ),
                ty,
            );
            parent.captures.push(capture.clone());
            child = parent;
        }
        let mut p = program(vec![
            statement(TypedStatementKind::Closure(child)),
            statement(TypedStatementKind::Expression(lexical(0, vec![], ty))),
            statement(TypedStatementKind::Return(Some(number("1")))),
        ]);
        p.functions[0].params.push(TypedParam {
            id: VariableId(73),
            name: "root".into(),
            ty,
            span: span(),
        });
        let mut compiler = Compiler::new();
        compiler.set_exports(was_compiler::ExportOptions::None);
        let bytes = compiler.compile(&p).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        // Range entry validation adds two runtime helpers; the other types use seven memory helpers.
        let first_helper = if ty == Type::Range { 10 } else { 8 };
        for index in first_helper..first_helper + 3 {
            assert_eq!(
                operators(&bytes, index)
                    .iter()
                    .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn captured_parameter_uses_shared_cell_and_hidden_environment() {
    let owner = BindingOwner::Function(FunctionId(0));
    let mut c = closure(
        0,
        owner,
        TypedClosureBody::Expression(Box::new(TypedExpr {
            kind: TypedExprKind::Captured {
                owner,
                id: VariableId(7),
                name: "value".into(),
            },
            ty: Type::Number.into(),
        })),
        Type::Number,
    );
    c.captures.push(TypedCapture {
        owner,
        variable: VariableId(7),
        ty: Type::Number,
        mutability: CaptureMutability::Mutable,
    });
    let mut p = program(vec![
        statement(TypedStatementKind::Closure(c)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![],
            Type::Number,
        )))),
    ]);
    p.functions[0].params.push(TypedParam {
        id: VariableId(7),
        name: "value".into(),
        ty: Type::Number,
        span: span(),
    });
    let bytes = Compiler::new().compile(&p).unwrap();
    let creator = operators(&bytes, 0);
    assert_eq!(
        creator
            .iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
            .count(),
        2
    );
    assert!(
        creator
            .iter()
            .any(|op| matches!(op, Operator::F64Store { memarg } if memarg.offset == 0))
    );
    let helper = operators(&bytes, 8);
    assert!(matches!(helper[0], Operator::LocalGet { local_index: 0 }));
    assert!(matches!(helper[1], Operator::I32Load { memarg } if memarg.offset == 0));
    assert!(matches!(helper[2], Operator::F64Load { memarg } if memarg.offset == 0));
    let types: Vec<_> = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::TypeSection(t) => Some(
                t.into_iter_err_on_gc_types()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        types
            .iter()
            .any(|t| t.params() == [wasmparser::ValType::I32]
                && t.results() == [wasmparser::ValType::F64])
    );
}

#[test]
fn capture_storage_is_eager_but_environment_allocation_stays_in_the_declaration_branch() {
    let bytes = Compiler::new().compile_source("fn main(flag: bool) -> number { let value = 1; if flag { let read = () => value; read(); } value = 2; return value; }").unwrap();
    let ops = operators(&bytes, 0);
    let branch = ops
        .iter()
        .position(|op| matches!(op, Operator::If { .. }))
        .unwrap();
    let allocations: Vec<_> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| matches!(op, Operator::Call { function_index: 1 }).then_some(i))
        .collect();
    assert_eq!(allocations.len(), 2);
    assert!(allocations[0] < branch && allocations[1] > branch);
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::F64Store { .. }))
            .count(),
        2
    );
    assert!(ops.iter().any(|op| matches!(op, Operator::F64Load { .. })));
}

#[test]
fn capture_metadata_must_be_complete_canonical_initialized_and_original() {
    let compiler = Compiler::new();
    let original = compiler.analyze("fn main(a: number, b: bool) -> number { let use_both = () => { if b { a = 2; } a }; return use_both(); }").unwrap();
    compiler.compile(&original).unwrap();
    for mutation in 0..9 {
        let mut forged = original.clone();
        let TypedStatementKind::Closure(c) = &mut forged.functions[0].body[0].kind else {
            panic!()
        };
        match mutation {
            0 => c.captures.reverse(),
            1 => c.captures.push(c.captures[0].clone()),
            2 => c.captures[0].variable = VariableId(999),
            3 => c.captures[0].owner = BindingOwner::Closure(CallableId(0)),
            4 => c.captures[0].ty = Type::Bool,
            5 => c.captures[0].mutability = CaptureMutability::ReadOnly,
            6 => {
                c.captures.remove(0);
            }
            7 => {
                c.captures.clear();
            }
            _ => {
                c.body = TypedClosureBody::Expression(Box::new(number("1")));
            }
        }
        let error = compiler.compile(&forged).unwrap_err();
        assert!(
            matches!(
                error,
                CodegenError::InvalidClosure {
                    reason: "invalid capture metadata",
                    ..
                }
            ),
            "mutation {mutation}: {error:?}"
        );
    }
}

#[test]
fn all_capture_types_use_uniform_slots_and_preserve_handle_validation_before_promotion() {
    let source = r#"fn main(n: number, b: bool, s: string, a: number[], r: range) -> number requires n >= 0 { let change = () => { n = 2; b = false; s = "new"; a[0] = n; a = [3]; r = 1..3; if b { len(a) } else { if 1 in r { n } else { 0 } } }; return change(); }"#;
    let mut compiler = Compiler::new();
    compiler.set_exports(was_compiler::ExportOptions::None);
    let typed = compiler.analyze(source).unwrap();
    let bytes = compiler.compile(&typed).unwrap();
    let ops = operators(&bytes, 0);
    let first_alloc = ops
        .iter()
        .position(|op| matches!(op, Operator::Call { function_index: 1 }))
        .unwrap();
    let calls_before: Vec<_> = ops[..first_alloc]
        .iter()
        .filter_map(|op| {
            if let Operator::Call { function_index } = op {
                Some(*function_index)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls_before, [2, 3, 9]);
    let contract = ops
        .iter()
        .position(|op| matches!(op, Operator::If { .. }))
        .unwrap();
    assert!(contract < first_alloc);
    assert!(
        !ops[..first_alloc]
            .iter()
            .any(|op| matches!(op, Operator::F64Load { .. } | Operator::I32Load { .. }))
    );
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
            .count(),
        6
    );
    let slot_offsets: Vec<_> = ops
        .iter()
        .filter_map(|op| {
            if let Operator::I32Store { memarg } = op {
                (memarg.offset != 0).then_some(memarg.offset)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(slot_offsets, [8, 16, 24, 32]);
    let helper = operators(&bytes, 11);
    assert!(
        helper
            .iter()
            .any(|op| matches!(op, Operator::F64Store { .. }))
    );
    assert!(
        helper
            .iter()
            .any(|op| matches!(op, Operator::I32Store { .. }))
    );
}

#[test]
fn nested_helpers_capture_only_their_immediate_creators_own_bindings() {
    let source = "fn main() -> number { let outer = () => { let local = 1; let inner = (x: number) => { local = local + x; local }; inner(2) }; return outer(); }";
    let bytes = Compiler::new().compile_source(source).unwrap();
    let main = operators(&bytes, 0);
    assert_eq!(
        main.iter()
            .filter(|op| matches!(op, Operator::Call { .. }))
            .count(),
        1
    );
    assert!(
        !main
            .iter()
            .any(|op| matches!(op, Operator::I32Const { .. }))
    );
    let creator = operators(&bytes, 8);
    assert_eq!(
        creator
            .iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
            .count(),
        2
    );
    let nested = operators(&bytes, 9);
    assert!(
        nested
            .iter()
            .any(|op| matches!(op, Operator::LocalGet { local_index: 1 }))
    );
    assert!(
        nested
            .iter()
            .any(|op| matches!(op, Operator::F64Store { .. }))
    );
}

#[test]
fn forged_capture_places_support_all_compounds_in_creator_and_helper() {
    let compiler = Compiler::new();
    for source in [
        "fn main() -> number { let n = 1; let f = () => { n = n + 2; n }; n = n + 3; return f(); }",
        "fn main() -> number { let n = [1]; let f = () => { n[0] = 2; n[0] }; n[0] = 3; return f(); }",
    ] {
        let original = compiler.analyze(source).unwrap();
        compiler.compile(&original).unwrap();
        for (helper, op) in [false, true].into_iter().flat_map(|helper| {
            use was_compiler::ast::BinaryOp::*;
            [Add, Subtract, Multiply, Divide, Remainder].map(|op| (helper, op))
        }) {
            let mut forged = original.clone();
            let statement = if helper {
                let TypedStatementKind::Closure(c) = &mut forged.functions[0].body[1].kind else {
                    panic!()
                };
                let TypedClosureBody::Block(block) = &mut c.body else {
                    panic!()
                };
                &mut block.statements[0]
            } else {
                &mut forged.functions[0].body[2]
            };
            let TypedStatementKind::Assignment { target, .. } = &statement.kind else {
                panic!()
            };
            statement.kind = TypedStatementKind::CompoundAssignment {
                target: target.clone(),
                op,
                value: number("1"),
            };
            let bytes = compiler.compile(&forged).unwrap();
            let helper_index = if op == was_compiler::ast::BinaryOp::Remainder {
                9
            } else {
                8
            };
            let ops = operators(&bytes, if helper { helper_index } else { 0 });
            assert!(ops.iter().any(|instruction| {
                use was_compiler::ast::BinaryOp::*;
                matches!(
                    (&op, instruction),
                    (Add, Operator::F64Add)
                        | (Subtract, Operator::F64Sub)
                        | (Multiply, Operator::F64Mul)
                        | (Divide, Operator::F64Div)
                        | (Remainder, Operator::Call { function_index: 8 })
                )
            }));
        }
    }
}

#[test]
fn captures_reject_out_of_scope_uninitialized_unused_and_synthetic_originals() {
    let compiler = Compiler::new();
    let source = "fn main() -> number { let value = 1; let read = () => value; return read(); }";
    let original = compiler.analyze(source).unwrap();
    let mut early = original.clone();
    early.functions[0].body.swap(0, 1);
    let TypedStatementKind::Closure(c) = &early.functions[0].body[0].kind else {
        panic!()
    };
    assert_invalid(&early, "invalid capture metadata", c.span);
    let mut escaped = original.clone();
    let declaration = escaped.functions[0].body.remove(0);
    escaped.functions[0].body.insert(
        0,
        statement(TypedStatementKind::If {
            cond: Box::new(TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            }),
            then: vec![declaration],
            else_: None,
        }),
    );
    let TypedStatementKind::Closure(c) = &escaped.functions[0].body[1].kind else {
        panic!()
    };
    assert_invalid(&escaped, "invalid capture metadata", c.span);

    let mut iteration = compiler
        .analyze(
            "fn main() -> number { for value in [1] { let read = () => 1; read(); } return 0; }",
        )
        .unwrap();
    let TypedStatementKind::For { variable, body, .. } = &mut iteration.functions[0].body[0].kind
    else {
        panic!()
    };
    let TypedStatementKind::Closure(c) = &mut body[0].kind else {
        panic!()
    };
    c.captures.push(TypedCapture {
        owner: c.owner,
        variable: *variable,
        ty: Type::Number,
        mutability: CaptureMutability::ReadOnly,
    });
    let origin = c.span;
    assert_invalid(&iteration, "invalid capture metadata", origin);

    let mut synthetic = compiler.analyze("fn named(a: number = 1) -> number { return a; } fn main() -> number { let read = () => 1; return named(a: 1); }").unwrap();
    let declaration = synthetic.functions[1].body.remove(0);
    let TypedStatementKind::Return(Some(TypedExpr {
        kind: TypedExprKind::NormalizedCall(sequence),
        ..
    })) = &mut synthetic.functions[1].body[0].kind
    else {
        panic!()
    };
    let binding = &mut sequence.bindings[0];
    let mut declaration = declaration;
    let TypedStatementKind::Closure(c) = &mut declaration.kind else {
        panic!()
    };
    c.captures.push(TypedCapture {
        owner: c.owner,
        variable: binding.id,
        ty: binding.ty,
        mutability: CaptureMutability::Mutable,
    });
    let origin = c.span;
    binding.value = TypedExpr {
        kind: TypedExprKind::Block(TypedValueBlock {
            statements: vec![declaration],
            tail: Some(Box::new(number("1"))),
            span: span(),
        }),
        ty: Type::Number.into(),
    };
    assert_invalid(&synthetic, "invalid capture metadata", origin);
}

#[test]
fn missing_capture_entries_fail_and_context_dependencies_forward_live_environments() {
    let compiler = Compiler::new();
    let mut missing = compiler
        .analyze("fn main() -> number { let f = () => 1; return f(); }")
        .unwrap();
    let TypedStatementKind::Closure(c) = &mut missing.functions[0].body[0].kind else {
        panic!()
    };
    c.body = TypedClosureBody::Expression(Box::new(TypedExpr {
        kind: TypedExprKind::Captured {
            owner: c.owner,
            id: VariableId(999),
            name: "missing".into(),
        },
        ty: Type::Number.into(),
    }));
    assert_invalid(&missing, "invalid capture metadata", c_body_span(&missing));
    let mut forwarded = compiler.analyze("fn main() -> number { let n = 1; let outer = () => { let inner = () => 1; inner() }; return outer(); }").unwrap();
    let TypedStatementKind::Closure(outer) = &mut forwarded.functions[0].body[1].kind else {
        panic!()
    };
    let TypedClosureBody::Block(block) = &mut outer.body else {
        panic!()
    };
    let TypedStatementKind::Closure(inner) = &mut block.statements[0].kind else {
        panic!()
    };
    inner.captures.push(TypedCapture {
        owner: BindingOwner::Function(FunctionId(0)),
        variable: VariableId(0),
        ty: Type::Number,
        mutability: CaptureMutability::Mutable,
    });
    let origin = inner.span;
    assert_invalid(&forwarded, "invalid capture metadata", origin);
    let mut dependency = compiler
        .analyze("fn main() -> number { let n = 1; let a = () => n; let b = () => 2; return a(); }")
        .unwrap();
    let TypedStatementKind::Closure(b) = &mut dependency.functions[0].body[2].kind else {
        panic!()
    };
    b.dependencies.push(CallableId(0));
    b.body = TypedClosureBody::Expression(Box::new(lexical(0, vec![], Type::Number)));
    let bytes = compiler.compile(&dependency).unwrap();
    let b_ops = operators(&bytes, 9);
    assert!(matches!(b_ops[0], Operator::LocalGet { local_index: 0 }));
    assert!(matches!(b_ops[1], Operator::I32Load { memarg } if memarg.offset == 0));
    assert!(matches!(b_ops[2], Operator::Call { function_index: 8 }));
}
fn c_body_span(program: &TypedProgram) -> Span {
    let TypedStatementKind::Closure(c) = &program.functions[0].body[0].kind else {
        panic!()
    };
    c.body_span
}

#[test]
fn shared_storage_is_deterministic_across_options_and_keeps_helper_exports_private() {
    use was_compiler::{CompilerOptions, OptimizationLevel};
    let source = "fn main(a: number) -> number { let x = 1; let first = () => { x = x + a; x }; let second = () => x; first(); return second(); }";
    let mut expected = None;
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        if let Some(expected) = &expected {
            assert_eq!(&bytes, expected);
        } else {
            expected = Some(bytes.clone());
        }
        let exports: Vec<_> = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|p| match p.unwrap() {
                Payload::ExportSection(exports) => Some(
                    exports
                        .into_iter()
                        .map(|e| e.unwrap().name)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(exports, ["main", "memory", "__alloc"]);
        assert!(
            !Parser::new(0)
                .parse_all(&bytes)
                .any(|p| matches!(p.unwrap(), Payload::TableSection(_)))
        );
    }
}

#[test]
fn capturing_helpers_validate_and_promote_their_own_parameters_with_hidden_index_shift() {
    let source = r#"fn main(n: number) -> number { let outer = (a: string, b: number) => { let inner = () => { a = "changed"; b = b + 1; b }; n = n + 1; inner() }; return outer("initial", 2); }"#;
    let typed = Compiler::new().analyze(source).unwrap();
    let original = typed.clone();
    let bytes = Compiler::new().compile(&typed).unwrap();
    assert_eq!(typed, original);
    let helper = operators(&bytes, 8);
    assert!(matches!(helper[0], Operator::LocalGet { local_index: 1 }));
    assert!(matches!(helper[1], Operator::Call { function_index: 2 }));
    assert!(matches!(helper[2], Operator::Drop));
    let env_alloc = helper
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, Operator::Call { function_index: 1 }))
        .nth(2)
        .unwrap()
        .0;
    let incoming: Vec<_> = helper[..env_alloc]
        .windows(2)
        .filter_map(|pair| match (&pair[0], &pair[1]) {
            (
                Operator::LocalGet { local_index },
                Operator::I32Store { .. } | Operator::F64Store { .. },
            ) => Some(*local_index),
            _ => None,
        })
        .collect();
    assert_eq!(incoming, [1, 2]);
    let nested = operators(&bytes, 9);
    let offsets: Vec<_> = nested
        .iter()
        .filter_map(|op| {
            if let Operator::I32Load { memarg } = op {
                Some(memarg.offset)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(offsets, [0, 8, 8, 8]);
}

fn helper_statements(program: &mut TypedProgram) -> &mut Vec<TypedStatement> {
    let TypedStatementKind::Closure(c) = &mut program.functions[0].body[1].kind else {
        panic!()
    };
    let TypedClosureBody::Block(block) = &mut c.body else {
        panic!()
    };
    &mut block.statements
}

fn make_compound(statement: &mut TypedStatement, op: was_compiler::ast::BinaryOp) {
    let TypedStatementKind::Assignment { target, value } = &statement.kind else {
        panic!()
    };
    statement.kind = TypedStatementKind::CompoundAssignment {
        target: target.clone(),
        op,
        value: value.clone(),
    };
}

#[test]
fn captured_scalar_compound_keeps_cell_and_old_value_below_mutating_rhs_without_scratch() {
    let compiler = Compiler::new();
    let mut p = compiler.analyze("fn main() -> number { let n = 1; let f = () => { n = { n = 100; 2 }; 0 }; return f(); }").unwrap();
    make_compound(
        &mut helper_statements(&mut p)[0],
        was_compiler::ast::BinaryOp::Add,
    );
    let bytes = compiler.compile(&p).unwrap();
    let ops = operators(&bytes, 8);
    // The original cell address and f64 are a live stack prefix while the RHS
    // overwrites that same cell. The final store must not reload either one.
    assert!(matches!(ops[0], Operator::LocalGet { local_index: 0 }));
    assert!(matches!(ops[1], Operator::I32Load { memarg } if memarg.offset == 0));
    assert!(matches!(ops[2], Operator::LocalGet { local_index: 0 }));
    assert!(matches!(ops[3], Operator::I32Load { memarg } if memarg.offset == 0));
    assert!(matches!(ops[4], Operator::F64Load { memarg } if memarg.offset == 0));
    assert!(ops.windows(3).any(|w| matches!(w,
        [Operator::F64Const { value }, Operator::F64Store { .. }, Operator::F64Const { .. }]
        if value.bits() == 100f64.to_bits()
    )));
    assert!(ops.windows(2).any(|w| matches!(w,
        [Operator::F64Add, Operator::F64Store { memarg }] if memarg.offset == 0
    )));
    let body = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|p| match p.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .nth(8)
        .unwrap();
    assert_eq!(body.get_locals_reader().unwrap().get_count(), 0);
}

#[test]
fn captured_index_places_save_original_address_and_old_value_before_binding_reassignment() {
    let compiler = Compiler::new();
    let source = "fn main() -> number { let a = [1]; let f = () => { a[0] = { a = [100]; 2 }; 0 }; return f(); }";
    let original = compiler.analyze(source).unwrap();
    // Ordinary source assignment and forged compound share the same place path.
    let ordinary = compiler.compile_source(source).unwrap();
    assert_eq!(ordinary, compiler.compile(&original).unwrap());
    for compound in [false, true] {
        let mut p = original.clone();
        if compound {
            make_compound(
                &mut helper_statements(&mut p)[0],
                was_compiler::ast::BinaryOp::Multiply,
            );
        }
        let bytes = compiler.compile(&p).unwrap();
        let ops = operators(&bytes, 8);
        let checked = ops
            .iter()
            .position(|op| matches!(op, Operator::Call { function_index: 4 }))
            .unwrap();
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Operator::Call { function_index: 4 }))
                .count(),
            1
        );
        let allocation = ops
            .iter()
            .position(|op| matches!(op, Operator::Call { function_index: 1 }))
            .unwrap();
        assert!(checked < allocation);
        assert!(matches!(ops[0], Operator::LocalGet { local_index: 0 }));
        assert!(matches!(ops[1], Operator::I32Load { memarg } if memarg.offset == 0));
        assert!(matches!(ops[2], Operator::I32Load { memarg } if memarg.offset == 0));
        if compound {
            let Operator::LocalSet {
                local_index: address,
            } = ops[checked + 1]
            else {
                panic!()
            };
            let Operator::LocalSet { local_index: old } = ops[checked + 4] else {
                panic!()
            };
            assert!(
                matches!(ops[checked + 2], Operator::LocalGet { local_index } if local_index == address)
            );
            assert!(matches!(ops[checked + 3], Operator::F64Load { memarg } if memarg.offset == 0));
            assert!(
                matches!(ops[checked + 5], Operator::LocalGet { local_index } if local_index == address)
            );
            assert!(
                matches!(ops[checked + 6], Operator::LocalGet { local_index } if local_index == old)
            );
            assert!(checked + 6 < allocation);
            assert!(
                ops.windows(2)
                    .any(|w| matches!(w, [Operator::F64Mul, Operator::F64Store { .. }]))
            );
            let body = Parser::new(0)
                .parse_all(&bytes)
                .filter_map(|p| match p.unwrap() {
                    Payload::CodeSectionEntry(body) => Some(body),
                    _ => None,
                })
                .nth(8)
                .unwrap();
            let locals: Vec<_> = body
                .get_locals_reader()
                .unwrap()
                .into_iter()
                .map(Result::unwrap)
                .collect();
            assert_eq!(
                locals,
                [
                    (1, wasmparser::ValType::I32),
                    (1, wasmparser::ValType::F64),
                    (1, wasmparser::ValType::I32)
                ]
            );
        }
        // Rebinding the array cell occurs before the final element write, with
        // no second address check or handle load after RHS effects.
        let rebind = ops
            .iter()
            .rposition(|op| matches!(op, Operator::I32Store { .. }))
            .unwrap();
        let final_write = ops
            .iter()
            .rposition(|op| matches!(op, Operator::F64Store { .. }))
            .unwrap();
        assert!(allocation < rebind && rebind < final_write);
        assert!(!ops[rebind + 1..final_write].iter().any(|op| matches!(
            op,
            Operator::I32Load { .. } | Operator::F64Load { .. } | Operator::Call { .. }
        )));
    }
}

#[test]
fn captured_compounds_keep_numeric_place_rhs_operator_and_owner_checks() {
    use was_compiler::ast::BinaryOp;
    let compiler = Compiler::new();
    for initializer in ["true", "\"text\"", "[1]", "0..2"] {
        let mut p = compiler.analyze(&format!("fn main() -> number {{ let n = {initializer}; let f = () => {{ n = n; 0 }}; return f(); }}")).unwrap();
        make_compound(&mut helper_statements(&mut p)[0], BinaryOp::Add);
        let error = compiler.compile(&p).unwrap_err();
        assert!(
            matches!(
                error,
                CodegenError::TypeMismatch {
                    expected: Type::Number,
                    ..
                }
            ),
            "{initializer}: {error:?}"
        );
    }
    for indexed in [false, true] {
        let (initializer, place) = if indexed { ("[1]", "n[0]") } else { ("1", "n") };
        let mut original = compiler.analyze(&format!("fn main() -> number {{ let n = {initializer}; let f = () => {{ {place} = 2; 0 }}; return f(); }}")).unwrap();
        make_compound(&mut helper_statements(&mut original)[0], BinaryOp::Add);
        compiler.compile(&original).unwrap();
        for mutation in 0..7 {
            let mut p = original.clone();
            let statement = &mut helper_statements(&mut p)[0];
            let origin = statement.span;
            let TypedStatementKind::CompoundAssignment { target, op, value } = &mut statement.kind
            else {
                panic!()
            };
            match mutation {
                0 => *op = BinaryOp::Equal,
                1 => {
                    *value = TypedExpr {
                        kind: TypedExprKind::Bool(false),
                        ty: Type::Bool.into(),
                    }
                }
                2 => *target = number("1"),
                3..=5 => {
                    let base = if let TypedExprKind::Index { expr, .. } = &mut target.kind {
                        expr.as_mut()
                    } else {
                        target
                    };
                    let TypedExprKind::Captured { owner, id, .. } = &mut base.kind else {
                        panic!()
                    };
                    match mutation {
                        3 => *owner = BindingOwner::Closure(CallableId(0)),
                        4 => *id = VariableId(9999),
                        _ => base.ty = Type::Bool.into(),
                    }
                }
                _ if indexed => {
                    let TypedExprKind::Index { index, .. } = &mut target.kind else {
                        panic!()
                    };
                    **index = TypedExpr {
                        kind: TypedExprKind::Bool(false),
                        ty: Type::Bool.into(),
                    };
                }
                _ => target.ty = ExprResult::Never,
            }
            let error = compiler.compile(&p).unwrap_err();
            assert_eq!(error.span(), Some(origin));
            match mutation {
                0 => assert!(matches!(error, CodegenError::UnsupportedOperator { .. })),
                1 => assert!(matches!(
                    error,
                    CodegenError::TypeMismatch {
                        expected: Type::Number,
                        found: Type::Bool,
                        ..
                    }
                )),
                2 => assert!(matches!(
                    error,
                    CodegenError::InvalidAssignmentTarget { .. }
                )),
                3 | 4 => assert!(matches!(
                    error,
                    CodegenError::InvalidClosure {
                        reason: "invalid capture metadata",
                        ..
                    }
                )),
                _ => assert!(matches!(
                    error,
                    CodegenError::InvalidClosure { .. }
                        | CodegenError::TypeMismatch { .. }
                        | CodegenError::ExprResultMismatch { .. }
                )),
            }
        }
    }
}

#[test]
fn sparse_captured_compounds_distinguish_original_owner_from_same_id_helper_parameter() {
    let owner = BindingOwner::Function(FunctionId(0));
    let id = VariableId(7001);
    let captured = TypedExpr {
        kind: TypedExprKind::Captured {
            owner,
            id,
            name: "outer".into(),
        },
        ty: Type::Number.into(),
    };
    let own = TypedExpr {
        kind: TypedExprKind::Identifier {
            id,
            name: "inner".into(),
        },
        ty: Type::Number.into(),
    };
    let param = TypedParam {
        id,
        name: "inner".into(),
        ty: Type::Number,
        span: span(),
    };
    let mut c = closure(
        0,
        owner,
        block(
            vec![statement(TypedStatementKind::CompoundAssignment {
                target: captured,
                op: was_compiler::ast::BinaryOp::Subtract,
                value: own.clone(),
            })],
            Some(own.clone()),
        ),
        Type::Number,
    );
    c.params.push(param.clone());
    c.captures.push(TypedCapture {
        owner,
        variable: id,
        ty: Type::Number,
        mutability: CaptureMutability::Mutable,
    });
    let mut p = program(vec![
        // Promotion is structural: this compound precedes the declaration.
        statement(TypedStatementKind::CompoundAssignment {
            target: own,
            op: was_compiler::ast::BinaryOp::Add,
            value: number("3"),
        }),
        statement(TypedStatementKind::Closure(c)),
        statement(TypedStatementKind::Return(Some(lexical(
            0,
            vec![number("2")],
            Type::Number,
        )))),
    ]);
    p.functions[0].params.push(param);
    let bytes = Compiler::new().compile(&p).unwrap();
    let helper = operators(&bytes, 8);
    assert!(helper.windows(3).any(|w| matches!(
        w,
        [
            Operator::LocalGet { local_index: 1 },
            Operator::F64Sub,
            Operator::F64Store { .. }
        ]
    )));
    let creator = operators(&bytes, 0);
    assert!(creator.windows(4).any(|w| matches!(w,
        [Operator::LocalGet { local_index: a }, Operator::LocalGet { local_index: b }, Operator::F64Load { .. }, Operator::F64Const { .. }] if a == b
    )));
    let mut forged = p;
    let TypedStatementKind::Closure(c) = &mut forged.functions[0].body[1].kind else {
        panic!()
    };
    let TypedClosureBody::Block(block) = &mut c.body else {
        panic!()
    };
    let TypedStatementKind::CompoundAssignment { target, .. } = &mut block.statements[0].kind
    else {
        panic!()
    };
    let TypedExprKind::Captured { owner, .. } = &mut target.kind else {
        panic!()
    };
    *owner = BindingOwner::Closure(CallableId(0));
    assert_invalid(&forged, "invalid capture metadata", span());
}

#[test]
fn captured_compounds_preserve_helper_local_transfers_divergence_and_skipped_checks() {
    use was_compiler::ast::BinaryOp;
    let compiler = Compiler::new();
    for (initializer, place) in [("1", "n"), ("[1]", "n[0]")] {
        for rhs in ["{ return 7; }", "loop {}", "{ loop { break; } 2 }"] {
            let mut p = compiler.analyze(&format!("fn main() -> number {{ let n = {initializer}; let f = () => {{ if false {{ return 7; }} {place} = {rhs}; 0 }}; return f(); }}")).unwrap();
            make_compound(&mut helper_statements(&mut p)[1], BinaryOp::Remainder);
            let bytes = compiler.compile(&p).unwrap();
            // Remainder discovery must still append its helper before callables,
            // even when the RHS never supplies the second operand.
            let ops = operators(&bytes, 9);
            assert!(
                ops.iter()
                    .any(|op| matches!(op, Operator::Call { function_index: 8 }))
            );
        }
        for transfer in ["break", "continue"] {
            let mut p = compiler.analyze(&format!("fn main() -> number {{ let n = {initializer}; let f = () => {{ if false {{ return 7; }} loop {{ {place} = {{ {transfer}; }}; }} 0 }}; return f(); }}")).unwrap();
            let TypedStatementKind::Loop { body, .. } = &mut helper_statements(&mut p)[1].kind
            else {
                panic!()
            };
            make_compound(&mut body[0], BinaryOp::Add);
            let bytes = compiler.compile(&p).unwrap();
            let ops = operators(&bytes, 8);
            let expected = if transfer == "break" { 2 } else { 1 };
            assert!(ops.iter().any(
                |op| matches!(op, Operator::Br { relative_depth } if *relative_depth == expected)
            ));
        }
        let mut p = compiler.analyze(&format!("fn main() -> number {{ let n = {initializer}; let f = () => {{ return 7; {place} = 2; }}; return f(); }}")).unwrap();
        make_compound(&mut helper_statements(&mut p)[1], BinaryOp::Add);
        let TypedStatementKind::CompoundAssignment { value, .. } =
            &mut helper_statements(&mut p)[1].kind
        else {
            panic!()
        };
        *value = TypedExpr {
            kind: TypedExprKind::Bool(false),
            ty: Type::Bool.into(),
        };
        assert!(matches!(
            compiler.compile(&p),
            Err(CodegenError::TypeMismatch {
                expected: Type::Number,
                found: Type::Bool,
                ..
            })
        ));
    }
}

#[test]
fn captured_compounds_reject_every_nonnumeric_rhs_category_and_check_divergent_indices() {
    use was_compiler::ast::BinaryOp;
    let compiler = Compiler::new();
    for (initializer, place) in [("1", "n"), ("[1]", "n[0]")] {
        let mut original = compiler.analyze(&format!("fn main() -> number {{ let n = {initializer}; let f = () => {{ {place} = 2; 0 }}; return f(); }}")).unwrap();
        make_compound(&mut helper_statements(&mut original)[0], BinaryOp::Add);
        for ty in [
            Type::Bool,
            Type::String,
            Type::Array(was_compiler::ArrayElementType::Number),
            Type::Range,
            Type::Void,
        ] {
            let mut p = original.clone();
            let TypedStatementKind::CompoundAssignment { value, .. } =
                &mut helper_statements(&mut p)[0].kind
            else {
                panic!()
            };
            // Public annotations are untrusted, including impossible storage categories.
            value.ty = ty.into();
            let error = compiler.compile(&p).unwrap_err();
            assert!(
                matches!(
                    error,
                    CodegenError::TypeMismatch {
                        expected: Type::Number,
                        ..
                    } | CodegenError::ExprResultMismatch { .. }
                ),
                "{ty:?}: {error:?}"
            );
        }
    }
    let mut p = compiler.analyze("fn main() -> number { let n = [1]; let f = () => { n[{ return 7; }] = 2; 0 }; return f(); }").unwrap();
    make_compound(&mut helper_statements(&mut p)[0], BinaryOp::Add);
    let bytes = compiler.compile(&p).unwrap();
    let ops = operators(&bytes, 8);
    let returned = ops
        .iter()
        .position(|op| matches!(op, Operator::Return))
        .unwrap();
    let checked = ops
        .iter()
        .position(|op| matches!(op, Operator::Call { function_index: 4 }))
        .unwrap();
    assert!(returned < checked);
    let TypedStatementKind::CompoundAssignment { value, .. } =
        &mut helper_statements(&mut p)[0].kind
    else {
        panic!()
    };
    // A noncompleting index must not skip RHS checking.
    *value = TypedExpr {
        kind: TypedExprKind::Bool(false),
        ty: Type::Bool.into(),
    };
    assert!(matches!(
        compiler.compile(&p),
        Err(CodegenError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        })
    ));
}

#[test]
fn noncompleting_initializer_allocates_no_cell_before_unreachable_declaration() {
    let bytes = Compiler::new()
        .compile_source(
            "fn main() -> number { let n: number = loop {}; let read = () => n; return read(); }",
        )
        .unwrap();
    let creator = operators(&bytes, 0);
    // Only the structurally checked, unreachable declaration contains an allocator call.
    assert_eq!(
        creator
            .iter()
            .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
            .count(),
        1
    );
    assert!(
        !creator
            .iter()
            .any(|op| matches!(op, Operator::F64Store { .. }))
    );
}
