use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    semantic::SemanticError,
    tokens::Span,
    typed_ast::{
        CallBindingOrigin, FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedNormalizedCall,
        TypedProgram, TypedStatement, TypedStatementKind, VariableId,
    },
};
use wasmparser::{Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const DIAMOND: &str = "// é: forward calls and invocation-local defaults
fn main() { return leaf(c: leaf(3), a: left() + right()); }
fn left() { return leaf(c: 9, a: 3); }
fn right() { return leaf(4); }
fn leaf(a: number, b: number = a + 1, c: number = b * 2) { return c; }";

fn span(source: &str, fragment: &str) -> Span {
    let start = source.find(fragment).unwrap();
    Span::new(start, start + fragment.len())
}

fn returned(program: &mut TypedProgram, function: usize) -> &mut TypedExpr {
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[function].body[0].kind
    else {
        panic!("expected return")
    };
    value
}

fn sequence(value: &mut TypedExpr) -> &mut TypedNormalizedCall {
    let TypedExprKind::NormalizedCall(sequence) = &mut value.kind else {
        panic!("expected normalized call")
    };
    sequence
}

fn fixture() -> TypedProgram {
    Compiler::new().analyze(DIAMOND).unwrap()
}

fn assert_codegen_error(program: &TypedProgram, expected: CodegenError) {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile(program).unwrap_err(),
            expected,
            "{level:?}"
        );
    }
}

fn assert_semantic_error(source: &str, expected: SemanticError) {
    for level in LEVELS {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        let CompileError::Semantic(actual) = compiler.analyze(source).unwrap_err() else {
            panic!("expected semantic error: {source}")
        };
        assert_eq!(actual, expected, "{level:?}: {source}");
    }
}

fn identifier(id: VariableId) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Identifier {
            id,
            name: "forged".into(),
        },
        ty: Type::Number.into(),
    }
}

#[test]
fn inferred_diamond_rechecks_normalized_targets_and_final_results_in_nested_calls() {
    enum Mutation {
        WrongTarget,
        UnknownTarget,
        WrongResult,
    }

    for case in [
        Mutation::WrongTarget,
        Mutation::UnknownTarget,
        Mutation::WrongResult,
    ] {
        let mut program = fixture();
        let outer = sequence(returned(&mut program, 0));
        let inner = sequence(&mut outer.bindings[0].value);
        let error_span = span(DIAMOND, "c: leaf(3)");
        let expected = match case {
            Mutation::WrongTarget => {
                let TypedExprKind::Call { func, .. } = &mut inner.call.kind else {
                    panic!("call")
                };
                *func = FunctionRef::User(FunctionId(1));
                CodegenError::InvalidNormalizedCall {
                    reason: "final call name does not match its target",
                    span: error_span,
                }
            }
            Mutation::UnknownTarget => {
                let TypedExprKind::Call { func, .. } = &mut inner.call.kind else {
                    panic!("call")
                };
                *func = FunctionRef::User(FunctionId(999));
                CodegenError::UnknownFunctionReference {
                    function: "main".into(),
                    reference: FunctionRef::User(FunctionId(999)),
                    span: error_span,
                }
            }
            Mutation::WrongResult => {
                inner.call.ty = Type::Bool.into();
                CodegenError::InvalidNormalizedCall {
                    reason: "final call result does not match its signature",
                    span: error_span,
                }
            }
        };
        assert_codegen_error(&program, expected);
    }
}

#[test]
fn nested_inferred_calls_cannot_supply_foreign_or_duplicate_formal_mapping_ids() {
    enum Mapping {
        ForeignNestedBinding,
        DuplicateBinding,
    }

    for case in [Mapping::ForeignNestedBinding, Mapping::DuplicateBinding] {
        let mut program = fixture();
        let outer = sequence(returned(&mut program, 0));
        let reason = match case {
            Mapping::ForeignNestedBinding => {
                let foreign = sequence(&mut outer.bindings[0].value).bindings[0].id;
                outer.formal_to_binding[0] = foreign;
                let TypedExprKind::Call { args, .. } = &mut outer.call.kind else {
                    panic!("call")
                };
                args[0] = identifier(foreign);
                "formal mapping must own every referenced binding"
            }
            Mapping::DuplicateBinding => {
                outer.formal_to_binding[0] = outer.formal_to_binding[2];
                let TypedExprKind::Call { args, .. } = &mut outer.call.kind else {
                    panic!("call")
                };
                args[0] = identifier(outer.formal_to_binding[0]);
                "formal mapping must be a bijection"
            }
        };
        assert_codegen_error(
            &program,
            CodegenError::InvalidNormalizedCall {
                reason,
                span: span(DIAMOND, "return leaf(c: leaf(3), a: left() + right());"),
            },
        );
    }
}

#[test]
fn nested_diamond_initializers_retain_invocation_ownership_and_default_scope() {
    enum Read {
        EnclosingExplicit,
        EnclosingDefault,
        CompletedNested,
    }

    for case in [
        Read::EnclosingExplicit,
        Read::EnclosingDefault,
        Read::CompletedNested,
    ] {
        let mut program = fixture();
        let outer = sequence(returned(&mut program, 0));
        let enclosing_id = outer.formal_to_binding[0];
        let (reason, error_span) = match case {
            Read::EnclosingExplicit => {
                let inner = sequence(&mut outer.bindings[0].value);
                inner.bindings[0].value = identifier(enclosing_id);
                (
                    "synthetic binding is only readable by its final call",
                    span(DIAMOND, "3"),
                )
            }
            Read::EnclosingDefault => {
                let inner = sequence(&mut outer.bindings[0].value);
                inner.bindings[1].value = identifier(enclosing_id);
                (
                    "defaults may only read owned earlier-formal bindings",
                    span(DIAMOND, "a + 1"),
                )
            }
            Read::CompletedNested => {
                let nested_id = sequence(&mut outer.bindings[0].value).formal_to_binding[0];
                outer.bindings[1].value = identifier(nested_id);
                (
                    "synthetic binding is only readable by its final call",
                    span(DIAMOND, "a: left() + right()"),
                )
            }
        };
        assert_codegen_error(
            &program,
            CodegenError::InvalidNormalizedCall {
                reason,
                span: error_span,
            },
        );
    }
}

#[test]
fn defaults_cannot_read_written_earlier_but_formal_later_inferred_arguments_even_when_skipped() {
    enum Context {
        Reachable,
        Unreachable,
        ShortCircuited,
    }

    for context in [
        Context::Reachable,
        Context::Unreachable,
        Context::ShortCircuited,
    ] {
        let mut program;
        let (source, value) = match context {
            Context::Reachable => {
                let source = DIAMOND.replace(
                    "return leaf(c: leaf(3), a: left() + right());",
                    "return leaf(c: right(), a: left());",
                );
                program = Compiler::new().analyze(&source).unwrap();
                (source, returned(&mut program, 0))
            }
            Context::Unreachable => {
                let source = DIAMOND.replace(
                    "return leaf(c: leaf(3), a: left() + right());",
                    "return 1; leaf(c: right(), a: left());",
                );
                program = Compiler::new().analyze(&source).unwrap();
                let TypedStatementKind::Expression(value) = &mut program.functions[0].body[1].kind
                else {
                    panic!("expression")
                };
                (source, value)
            }
            Context::ShortCircuited => {
                let source = DIAMOND.replace(
                    "return leaf(c: leaf(3), a: left() + right());",
                    "false && leaf(c: right(), a: left()) == 0; return 1;",
                );
                program = Compiler::new().analyze(&source).unwrap();
                let TypedStatementKind::Expression(value) = &mut program.functions[0].body[0].kind
                else {
                    panic!("expression")
                };
                let TypedExprKind::Binary { right, .. } = &mut value.kind else {
                    panic!("logical and")
                };
                let TypedExprKind::Binary { left, .. } = &mut right.kind else {
                    panic!("equality")
                };
                (source, left.as_mut())
            }
        };
        let call = sequence(value);
        call.bindings[2].value = identifier(call.formal_to_binding[2]);
        assert_codegen_error(
            &program,
            CodegenError::InvalidNormalizedCall {
                reason: "defaults may only read owned earlier-formal bindings",
                span: span(&source, "a + 1"),
            },
        );
    }
}

#[test]
fn nested_inferred_bindings_cannot_escape_into_unreachable_unexported_statements() {
    let mut program = fixture();
    let outer = sequence(returned(&mut program, 0));
    let nested_id = sequence(&mut outer.bindings[0].value).bindings[0].id;
    let error_span = span(DIAMOND, "return leaf(c: leaf(3), a: left() + right());");
    program.functions[0].body.push(TypedStatement {
        kind: TypedStatementKind::Expression(identifier(nested_id)),
        span: error_span,
    });
    assert_codegen_error(
        &program,
        CodegenError::InvalidNormalizedCall {
            reason: "synthetic binding is only readable by its final call",
            span: error_span,
        },
    );
}

#[test]
fn combined_inferred_signatures_do_not_hide_body_or_normalized_result_mismatches() {
    let mut program = fixture();
    program.functions[0].return_type = Type::Bool;
    assert_codegen_error(
        &program,
        CodegenError::TypeMismatch {
            function: "main".into(),
            expected: Type::Bool,
            found: Type::Number,
            span: span(DIAMOND, "return leaf(c: leaf(3), a: left() + right());"),
        },
    );

    let mut program = fixture();
    program.functions[3].return_type = Type::Bool;
    assert_codegen_error(
        &program,
        CodegenError::InvalidNormalizedCall {
            reason: "final call result does not match its signature",
            span: span(DIAMOND, "return leaf(c: leaf(3), a: left() + right());"),
        },
    );

    let mut program = fixture();
    let error_span = span(DIAMOND, "return c;");
    program.functions[3].body.push(TypedStatement {
        kind: TypedStatementKind::Return(Some(TypedExpr {
            kind: TypedExprKind::Bool(false),
            ty: Type::Bool.into(),
        })),
        span: error_span,
    });
    assert_codegen_error(
        &program,
        CodegenError::TypeMismatch {
            function: "leaf".into(),
            expected: Type::Number,
            found: Type::Bool,
            span: error_span,
        },
    );
}

#[test]
fn combined_diagnostics_progress_from_declarations_to_graph_to_dependencies_to_final_bodies() {
    let source = "// é: the earlier annotated body must be checked last
fn final_body() -> number { return false; }
fn root() { return left() + right(); }
fn left() { return leaf(value: true); }
fn right() { return leaf(); }
fn leaf(value: number = 1) { if true { return value; } return false; }
fn recur(value: number = 1) { return recur(value: value); }
fn unused(value: number = unknown) { return value; }";
    assert_semantic_error(
        source,
        SemanticError::UndefinedVariable {
            name: "unknown".into(),
            span: span(source, "unknown"),
        },
    );
    let source = source.replace("= unknown", "= 2");
    assert_semantic_error(
        &source,
        SemanticError::RecursiveResultAnnotationRequired {
            name: "recur".into(),
            span: span(
                &source,
                "fn recur(value: number = 1) { return recur(value: value); }",
            ),
        },
    );
    let source = source.replace(
        "fn recur(value: number = 1) {",
        "fn recur(value: number = 1) -> number {",
    );
    assert_semantic_error(
        &source,
        SemanticError::IncompatibleInferredReturns {
            expected: Type::Number,
            found: Type::Bool,
            span: span(
                &source,
                "fn leaf(value: number = 1) { if true { return value; } return false; }",
            ),
        },
    );
    let source = source.replace("if true { return value; } return false;", "return value;");
    assert_semantic_error(
        &source,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: span(&source, "value: true"),
        },
    );
    let source = source.replace("value: true", "value: 3");
    assert_semantic_error(
        &source,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: span(&source, "return false;"),
        },
    );
}

#[test]
fn newly_ready_diamond_branches_report_named_argument_errors_in_source_order() {
    let left = "fn left() { return leaf(a: true); }";
    let right = "fn right() { return leaf(c: false, a: 4); }";
    for (branches, origin) in [
        (format!("{left} {right}"), "a: true"),
        (format!("{right} {left}"), "c: false"),
    ] {
        let source = format!(
            "// é\nfn root() {{ return left() + right(); }} {branches} fn leaf(a: number, b: number = a + 1, c: number = b * 2) {{ return c; }}"
        );
        assert_semantic_error(
            &source,
            SemanticError::TypeMismatch {
                expected: Type::Number,
                found: Type::Bool,
                span: span(&source, origin),
            },
        );
    }
}

#[test]
fn unused_or_supplied_defaults_validate_whitelist_and_literals_before_diamond_inference() {
    let overflow = "9".repeat(400);
    for supplied in ["", "fn supplied() { return unused(value: 2); }"] {
        let expression = "false && leaf(a: true) == 0";
        let source = format!(
            "{DIAMOND} fn recur() {{ return recur(); }} fn unused(value: bool = {expression}) {{ return value; }} {supplied}"
        );
        assert_semantic_error(
            &source,
            SemanticError::InvalidDefaultExpression {
                reason: "only positional builtin len calls are allowed",
                span: span(&source, expression),
            },
        );
        let source = format!(
            "{DIAMOND} fn recur() {{ return recur(); }} fn unused(value: number = {overflow}) {{ return value; }} {supplied}"
        );
        assert_semantic_error(
            &source,
            SemanticError::DefaultNumberLiteralOutOfRange {
                literal: overflow.clone(),
                span: span(&source, &overflow),
            },
        );
    }
}

#[test]
fn ordinary_literals_in_combined_programs_remain_checked_at_lowering_with_their_own_origins() {
    let overflow = "9".repeat(400);
    let argument = format!("c: {overflow}");
    let sources = [
        (DIAMOND.replace("c: leaf(3)", &argument), "main", argument),
        (
            format!("{DIAMOND} fn unused() {{ return {overflow}; }}"),
            "unused",
            format!("return {overflow};"),
        ),
    ];
    for (source, function, origin) in sources {
        let program = Compiler::new().analyze(&source).unwrap();
        assert_codegen_error(
            &program,
            CodegenError::NumberLiteralOutOfRange {
                function: function.into(),
                literal: overflow.clone(),
                span: span(&source, &origin),
            },
        );
    }
}

#[test]
fn forward_named_default_diamond_keeps_concrete_signatures_and_source_order_calls() {
    let mut program = fixture();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.return_type))
            .collect::<Vec<_>>(),
        [
            ("main", Type::Number),
            ("left", Type::Number),
            ("right", Type::Number),
            ("leaf", Type::Number)
        ]
    );
    let outer = sequence(returned(&mut program, 0));
    assert_eq!(
        outer
            .bindings
            .iter()
            .map(|binding| binding.origin)
            .collect::<Vec<_>>(),
        [
            CallBindingOrigin::Explicit { argument: 0 },
            CallBindingOrigin::Explicit { argument: 1 },
            CallBindingOrigin::Default { formal: 1 },
        ]
    );
    assert_eq!(outer.bindings[0].span, span(DIAMOND, "c: leaf(3)"));
    assert_eq!(outer.bindings[1].span, span(DIAMOND, "a: left() + right()"));
    assert_eq!(outer.bindings[2].span, span(DIAMOND, "a + 1"));
    assert_eq!(
        outer.formal_to_binding,
        [
            outer.bindings[1].id,
            outer.bindings[2].id,
            outer.bindings[0].id
        ]
    );
    let inner = sequence(&mut outer.bindings[0].value);
    assert_eq!(
        inner
            .bindings
            .iter()
            .map(|binding| binding.origin)
            .collect::<Vec<_>>(),
        [
            CallBindingOrigin::Explicit { argument: 0 },
            CallBindingOrigin::Default { formal: 1 },
            CallBindingOrigin::Default { formal: 2 },
        ]
    );
    assert_eq!(inner.bindings[0].span, span(DIAMOND, "3"));
    let mut compiler = Compiler::new();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let mut signatures = Vec::new();
        let mut functions = Vec::new();
        let mut calls = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::TypeSection(reader) => {
                    signatures.extend(reader.into_iter_err_on_gc_types().map(Result::unwrap))
                }
                Payload::FunctionSection(reader) => {
                    functions.extend(reader.into_iter().map(Result::unwrap))
                }
                Payload::CodeSectionEntry(body) => calls.push(
                    body.get_operators_reader()
                        .unwrap()
                        .into_iter()
                        .map(Result::unwrap)
                        .filter_map(|op| match op {
                            Operator::Call { function_index } => Some(function_index),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => {}
            }
        }
        assert_eq!(functions, [0, 0, 0, 1]);
        assert_eq!(signatures[0].params(), []);
        assert_eq!(signatures[0].results(), [ValType::F64]);
        assert_eq!(signatures[1].params(), [ValType::F64; 3]);
        assert_eq!(signatures[1].results(), [ValType::F64]);
        assert_eq!(calls, [vec![3, 1, 2, 3], vec![3], vec![3], vec![]]);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
    }
}
