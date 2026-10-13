#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    lexer::Lexer,
    parser::Parser as SourceParser,
    semantic::SemanticError,
    typed_ast::{FunctionId, FunctionRef, TypedExprKind, TypedStatementKind},
};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn semantic_error(source: &str) -> SemanticError {
    match Compiler::new().analyze(source) {
        Err(CompileError::Semantic(error)) => error,
        result => panic!("expected semantic error for {source}, found {result:?}"),
    }
}

fn assert_result(source: &str, expected: Type) {
    let program = Compiler::new().analyze(source).unwrap();
    assert_eq!(program.functions[0].return_type, expected, "{source}");
}

fn assert_recursive(source: &str, expected_name: &str) {
    let error = semantic_error(source);
    assert!(
        matches!(
            &error,
            SemanticError::RecursiveResultAnnotationRequired { name, .. } if name == expected_name
        ),
        "{source}: {error:?}"
    );
    let tokens = Lexer::new(source).tokenize();
    let raw = SourceParser::new(&tokens).parse_program().unwrap();
    let function = raw
        .functions
        .iter()
        .find(|function| function.name == expected_name)
        .unwrap();
    assert_eq!(error.span(), Some(function.span));
}

fn assert_type_error(source: &str, excerpt: &str) {
    let error = semantic_error(source);
    assert!(
        matches!(&error, SemanticError::TypeMismatch { .. }),
        "{source}: {error:?}"
    );
    let span = error.span().unwrap();
    assert_eq!(&source[span.start()..span.end()], excerpt);
}

#[test]
fn omitted_result_infers_number_without_changing_parameter_types() {
    let program = Compiler::new()
        .analyze("fn square(x: number) { return x * x; }")
        .unwrap();
    assert_eq!(program.functions[0].return_type, Type::Number);
    assert_eq!(program.functions[0].params[0].ty, Type::Number);
}

#[test]
fn raw_result_annotations_are_optional_but_parameters_and_named_tails_are_not() {
    let tokens = Lexer::new("fn n(x: number) { return x; } fn v() -> void {}").tokenize();
    let raw = SourceParser::new(&tokens).parse_program().unwrap();
    assert_eq!(raw.functions[0].return_type, None);
    assert_eq!(raw.functions[1].return_type, Some(Type::Void));
    for source in [
        "fn n(x) { return x; }",
        "fn n() { 42 }",
        "fn n() -> { return 1; }",
        "fn n() -> missing { return 1; }",
    ] {
        assert!(
            matches!(Compiler::new().analyze(source), Err(CompileError::Parse(_))),
            "{source}"
        );
    }
}

#[test]
fn all_concrete_language_results_are_inferred() {
    for (source, expected) in [
        ("fn n() { return 42; }", Type::Number),
        ("fn b() { return true; }", Type::Bool),
        ("fn s() { return \"é\"; }", Type::String),
        ("fn a() { return []; }", Type::NumberArray),
        ("fn r() { return 1..=9 by 2; }", Type::Range),
        ("fn v() {}", Type::Void),
        ("fn v() { return; }", Type::Void),
        ("fn v() { 42; }", Type::Void),
        ("fn v() { loop {} }", Type::Void),
        ("fn v() { loop { continue; } }", Type::Void),
        (
            "fn v() { if true { return; } else { loop {} } }",
            Type::Void,
        ),
    ] {
        assert_result(source, expected);
    }
}

#[test]
fn never_only_value_return_syntax_requires_an_annotation_even_when_unreachable() {
    for source in [
        "fn spin() { return loop {}; }",
        "fn spin() { loop {} return 1; }",
        "fn spin() { loop {} let x: number = { return 1; 0 }; }",
        "fn spin() { loop {} (loop { break { return 1; 0 }; }); }",
        "fn spin() { return; return 1; }",
    ] {
        let error = semantic_error(source);
        assert!(
            matches!(&error, SemanticError::CannotInferFunctionResult { name, .. } if name == "spin"),
            "{source}: {error:?}"
        );
        assert!(error.span().is_some());
    }
    for ty in ["number", "bool", "string", "number[]", "range"] {
        let source = format!("fn spin() -> {ty} {{ return loop {{}}; }}");
        Compiler::new().analyze(&source).unwrap();
    }
}

#[test]
fn mixed_reachable_returns_have_stable_first_type_and_reject_void_value_mixtures() {
    for (body, expected, found) in [
        (
            "if flag { return 1; } return true;",
            Type::Number,
            Type::Bool,
        ),
        (
            "if flag { return true; } return 1;",
            Type::Bool,
            Type::Number,
        ),
        ("if flag { return; } return 1;", Type::Void, Type::Number),
        ("if flag { return 1; } return;", Type::Number, Type::Void),
        (
            "if flag { return \"x\"; } return [];",
            Type::String,
            Type::NumberArray,
        ),
        (
            "if flag { return 0..2; } return 1;",
            Type::Range,
            Type::Number,
        ),
    ] {
        let source = format!("fn choose(flag: bool) {{ {body} }}");
        let error = semantic_error(&source);
        assert!(
            matches!(error, SemanticError::IncompatibleInferredReturns { expected: e, found: f, .. } if e == expected && f == found),
            "{source}: {error:?}"
        );
    }
}

#[test]
fn nonvoid_inference_keeps_conservative_definite_return_rules() {
    for body in [
        "if flag { return 1; }",
        "while true { return 1; }",
        "for i in 0..1 { return i; }",
        "for i in [1] { return i; }",
        "loop { if flag { return 1; } break; }",
    ] {
        let source = format!("fn incomplete(flag: bool) {{ {body} }}");
        assert!(
            matches!(semantic_error(&source), SemanticError::MissingReturn { name, .. } if name == "incomplete"),
            "{source}"
        );
    }
    assert_result(
        "fn complete(flag: bool) { if flag { return 1; } else { loop {} } }",
        Type::Number,
    );
}

#[test]
fn reachable_returns_compose_through_nested_expression_and_loop_regions() {
    for source in [
        "fn n() { return { return 7; 0 }; }",
        "fn n() { let x: number = { return 7; 0 }; return x; }",
        "fn n() { loop { return 7; } }",
        "fn n() { return loop { if true { return 7; } break 8; }; }",
        "fn n() { while { return 7; true } {} }",
        "fn n() { for i in { return 7; 0..1 } {} }",
        "fn n() { for i in [1] { if i == 1 { return 7; } } return 8; }",
        "fn n() { while false { return 7; } return 8; }",
        "fn n() { let a = [0]; a[{ return 7; 0 }] += { return 8; 0 }; return 9; }",
        "fn n() { return { return 7; true } && { return 8; true }; }",
        "fn n() { return 1..{ return 7; 8 }; }",
        "fn n() { return [{ return 7; 0 }]; }",
        "fn n() { return if true { { return 7; 0 } } else { 8 }; }",
    ] {
        assert_result(source, Type::Number);
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::None);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
    }
    assert_result(
        "fn b() { false && { return true; false }; return false; }",
        Type::Bool,
    );
    assert_result(
        "fn b() { true || { return false; true }; return true; }",
        Type::Bool,
    );
}

#[test]
fn short_circuit_rhs_constrains_possible_returns_without_constant_folding() {
    for expression in ["false && { return 1; true }", "true || { return 1; false }"] {
        let source = format!("fn b() {{ {expression}; return true; }}");
        assert!(
            matches!(
                semantic_error(&source),
                SemanticError::IncompatibleInferredReturns {
                    expected: Type::Number,
                    found: Type::Bool,
                    ..
                }
            ),
            "{source}"
        );
    }
}

#[test]
fn unreachable_returns_do_not_select_a_type_but_final_check_rejects_mismatches() {
    for (source, expected, found) in [
        (
            "fn n() { return 1; return true; }",
            Type::Number,
            Type::Bool,
        ),
        (
            "fn n() { loop { return 1; return true; } }",
            Type::Number,
            Type::Bool,
        ),
        (
            "fn n() { return { return 1; true } && { return false; true }; }",
            Type::Number,
            Type::Bool,
        ),
        (
            "fn n() { return loop { break 1; return false; }; }",
            Type::Number,
            Type::Bool,
        ),
        (
            "fn n() { loop { break; return false; } return 1; }",
            Type::Number,
            Type::Bool,
        ),
        (
            "fn n() { for i in { return 1; [] } { return false; } }",
            Type::Number,
            Type::Bool,
        ),
    ] {
        assert!(
            matches!(semantic_error(source), SemanticError::TypeMismatch { expected: e, found: f, .. } if e == expected && f == found),
            "{source}"
        );
    }
    assert_result("fn n() { return 1; return 2; }", Type::Number);
    assert!(
        matches!(semantic_error("fn n() { return 1; missing; }"), SemanticError::UndefinedVariable { name, .. } if name == "missing")
    );
}

#[test]
fn recursion_requires_annotations_even_with_anchors_skipped_calls_or_unused_functions() {
    for source in [
        "fn recur() { return recur(); }",
        "fn recur() { if true { return 0; } return recur(); }",
        "fn recur() { return 0; recur(); }",
        "fn recur() { false && recur() == 0; return 0; }",
        "fn main() -> number { return 1; } fn recur() { return recur(); }",
        "fn a() -> number { return b(); } fn b() { return a(); }",
        "fn a() -> number { return b(); } fn b() { return c(); } fn c() -> number { return a(); }",
    ] {
        assert_recursive(
            source,
            if source.contains("fn b() {") {
                "b"
            } else {
                "recur"
            },
        );
    }
    assert_recursive(
        "fn first() { return second(); } fn second() { return first(); }",
        "first",
    );
    assert_recursive(
        "fn first() { return first(); } fn second() { return second(); }",
        "first",
    );
}

#[test]
fn recursive_graph_traverses_every_raw_expression_and_statement_child() {
    for snippet in [
        "let x = recur();",
        "let x = -recur();",
        "let x = !recur();",
        "let x = recur() + 1;",
        "let x = 1 + recur();",
        "let x = [1, recur()];",
        "let x = recur()..2;",
        "let x = 0..recur();",
        "let x = 0..2 by recur();",
        "let x = 1 in recur();",
        "let x = a[recur()];",
        "a[recur()] = 1;",
        "a[0] += recur();",
        "a = recur();",
        "consume(recur());",
        "consume(value: recur());",
        "if recur() { }",
        "if true { recur(); } else {}",
        "if true {} else { recur(); }",
        "while recur() {}",
        "while true { recur(); }",
        "for i in recur() {}",
        "for i in [] { recur(); }",
        "loop { recur(); break; }",
        "(loop { break recur(); });",
        "({ recur() });",
        "let x = if true { recur() } else { 0 };",
        "let x = if true { 0 } else { recur() };",
        "return recur();",
    ] {
        let source = format!("fn recur() {{ return 1; {snippet} }}");
        assert_recursive(&source, "recur");
    }
}

#[test]
fn explicit_recursive_components_and_old_body_error_order_are_preserved() {
    let source = "fn a() -> number { return b(); } fn b() -> number { return a(); }";
    let program = Compiler::new().analyze(source).unwrap();
    Validator::new()
        .validate_all(&Compiler::new().compile(&program).unwrap())
        .unwrap();
    let source =
        "fn first() -> number { return missing; } fn second() -> number { return second(); }";
    assert!(
        matches!(semantic_error(source), SemanticError::UndefinedVariable { name, .. } if name == "missing")
    );
}

#[test]
fn unannotated_dependencies_resolve_in_both_source_orders_including_skipped_calls() {
    for (caller, target, expected) in [
        (
            "fn caller() { return target(); }",
            "fn target() { return 1; }",
            Type::Number,
        ),
        ("fn caller() { target(); }", "fn target() {}", Type::Void),
        (
            "fn caller() { return 1; target(); }",
            "fn target() { return 2; }",
            Type::Number,
        ),
        (
            "fn caller() { false && target(); return false; }",
            "fn target() { return true; }",
            Type::Bool,
        ),
    ] {
        for target_first in [false, true] {
            let source = if target_first {
                format!("{target} {caller}")
            } else {
                format!("{caller} {target}")
            };
            let program = Compiler::new().analyze(&source).unwrap();
            assert_eq!(
                program.functions[usize::from(target_first)].return_type,
                expected
            );
            Validator::new()
                .validate_all(&Compiler::new().compile(&program).unwrap())
                .unwrap();
        }
    }
}

#[test]
fn resolved_calls_keep_written_type_check_order_and_checker_span_origins() {
    assert_type_error(
        "fn caller() { return combine(b: second(), a: first()); } fn first() { return true; } fn second() { return false; } fn combine(a: number, b: number) -> number { return a + b; }",
        "b: second()",
    );
    assert_type_error(
        "fn caller() { let x: number = { let x = 0; target() }; return x; } fn target() { return true; }",
        "let x: number = { let x = 0; target() };",
    );
    let source = "fn caller() { return second() + first(); } fn first() { return true; } fn second() { return false; }";
    let error = semantic_error(source);
    assert!(
        matches!(&error, SemanticError::InvalidBinaryOperands { operator, left: Type::Bool, right: Type::Bool, .. } if *operator == "+")
    );
    let span = error.span().unwrap();
    assert_eq!(
        &source[span.start()..span.end()],
        "return second() + first();"
    );
}

#[test]
fn lexical_shadowing_excludes_variables_from_user_call_edges() {
    for source in [
        "fn f(f: number) { return f(); }",
        "fn f() { let f = 1; return f(); }",
        "fn f() { if true { let f = 1; f(); } return 1; }",
        "fn f() { while false { let f = 1; f(); } return 1; }",
        "fn f() { for f in [] { f(); } return 1; }",
        "fn f() { return { let f = 1; f() }; }",
        "fn f() { return if true { let f = 1; f() } else { 0 }; }",
        "fn f() { return loop { let f = 1; break f(); }; }",
        "fn f() { let f = 1; if true { let f = f(); } return 1; }",
    ] {
        assert!(
            matches!(semantic_error(source), SemanticError::NotCallable { name, .. } if name == "f"),
            "{source}"
        );
    }
    let source = "fn target() { return 1; } fn caller(target: number) { return target(); }";
    assert!(
        matches!(semantic_error(source), SemanticError::NotCallable { name, .. } if name == "target")
    );
}

#[test]
fn initializers_and_iterables_resolve_before_their_new_bindings() {
    assert_recursive("fn f() { let f = f(); return 1; }", "f");
    assert_recursive("fn f() { for f in f() {} return 1; }", "f");
    assert_recursive("fn f() { return { let f = f(); f }; }", "f");
    assert_result(
        "fn caller() { let target = target(); return target; } fn target() { return 2; }",
        Type::Number,
    );
    assert_result(
        "fn caller() { for target in target() {} return 1; } fn target() { return []; }",
        Type::Number,
    );
    assert_result(
        "fn caller() { let target = target(); return target; } fn target() -> number { return 2; }",
        Type::Number,
    );
}

#[test]
fn branch_block_and_for_names_do_not_leak_into_siblings_or_enclosing_scopes() {
    for body in [
        "if true { let f = 1; } else { f(); } return 1;",
        "if true { let f = 1; } f(); return 1;",
        "while false { let f = 1; } f(); return 1;",
        "for f in [] {} f(); return 1;",
        "({ let f = 1; 0 }); f(); return 1;",
        "let x = if true { let f = 1; 0 } else { f() }; return 1;",
        "(loop { let f = 1; break 0; }); f(); return 1;",
    ] {
        assert_recursive(&format!("fn f() {{ {body} }}"), "f");
    }
    // An argument's block bindings must not shadow later arguments.
    assert_recursive("fn f() { consume({ let f = 1; 0 }, f()); return 1; }", "f");
    // Even when the outer call is shadowed, its argument syntax contributes edges.
    assert_recursive("fn f() { let consume = 1; consume(f()); return 1; }", "f");
}

#[test]
fn host_builtin_and_annotated_forward_calls_are_available_during_inference() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("sample", "env", "sample", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("emit", "env", "emit", vec![Type::Number], Type::Void)
        .unwrap();
    let program = compiler
        .analyze(
            "fn n() { return sample() + len([1, 2]) + later(value: 3); }
         fn v() { emit(4); }
         fn later(value: number) -> number { return value; }",
        )
        .unwrap();
    assert_eq!(program.functions[0].return_type, Type::Number);
    assert_eq!(program.functions[1].return_type, Type::Void);
    Validator::new()
        .validate_all(&compiler.compile(&program).unwrap())
        .unwrap();
}

#[test]
fn declaration_errors_precede_graph_errors_and_graph_errors_precede_probes() {
    let source = "fn f() { return f(); } fn f() {}";
    assert!(
        matches!(semantic_error(source), SemanticError::DuplicateFunction { name, .. } if name == "f")
    );
    let source = "fn f() { return f(); } fn g(x: void) {}";
    assert!(
        matches!(semantic_error(source), SemanticError::InvalidVoidType { name, .. } if name == "x")
    );
    let source = "fn bad() { return missing; } fn recur() { return recur(); }";
    assert_recursive(source, "recur");
    let source =
        "fn bad() { return missing; } fn caller() { return target(); } fn target() { return 1; }";
    assert!(
        matches!(semantic_error(source), SemanticError::UndefinedVariable { name, .. } if name == "missing")
    );
}

#[test]
fn duplicate_parameters_precede_inference_graph_errors() {
    let source = "fn bad(x: number, x: number) -> void {} fn recurse() { return recurse(); }";
    let tokens = Lexer::new(source).tokenize();
    let raw = SourceParser::new(&tokens).parse_program().unwrap();
    assert_eq!(
        semantic_error(source),
        SemanticError::DuplicateVariable {
            name: "x".into(),
            span: raw.functions[0].params[1].span,
        }
    );
}

#[test]
fn fully_annotated_programs_keep_body_errors_before_later_duplicate_parameters() {
    let source = "fn first() -> void { missing(); } fn bad(x: number, x: number) -> void {}";
    assert!(matches!(
        semantic_error(source),
        SemanticError::UndefinedFunction { name, .. } if name == "missing"
    ));
}

#[test]
fn graph_diagnostics_use_source_order_across_dependencies_and_recursive_components() {
    assert_recursive(
        "fn caller() { return target(); } fn recur() { return recur(); } fn target() { return 1; }",
        "recur",
    );
    assert_recursive(
        "fn recur() { return recur(); } fn caller() { return target(); } fn target() { return 1; }",
        "recur",
    );
    assert_recursive(
        "fn first() -> number { return late(); } fn early() { return early(); } fn late() { return first(); }",
        "early",
    );
}

#[test]
fn dependency_probes_choose_the_smallest_source_index_after_each_completion() {
    for (source, expected) in [
        (
            "fn first() { target(); return missing_first; } fn target() { return 1; } fn later() { return missing_later; }",
            "missing_first",
        ),
        (
            "fn first() { return consume(second(), third()); } fn second() { return missing_second; } fn third() { return missing_third; } fn consume(a: number, b: number) -> number { return a; }",
            "missing_second",
        ),
    ] {
        let error = semantic_error(source);
        assert!(
            matches!(&error, SemanticError::UndefinedVariable { name, .. } if name == expected),
            "{error:?}"
        );
        let origin = error.span().unwrap();
        assert_eq!(
            &source[origin.start()..origin.end()],
            format!("return {expected};")
        );
    }
}

#[test]
fn annotated_results_break_scheduling_dependencies_but_not_recursive_graph_checks() {
    assert_result(
        "fn first() { return bridge(); } fn bridge() -> number { return leaf(); } fn leaf() { return 1; }",
        Type::Number,
    );
    assert_recursive(
        "fn first() { return bridge(); } fn bridge() -> number { return first(); } fn bad() { return missing; }",
        "first",
    );
    let source = "fn first() { return bridge(); } fn bridge() -> number { return leaf(); } fn leaf() { return missing_leaf; }";
    assert!(
        matches!(semantic_error(source), SemanticError::UndefinedVariable { name, .. } if name == "missing_leaf")
    );
}

#[test]
fn probes_precede_final_source_order_checks_without_reordering_function_ids() {
    let source = "fn explicit() -> number { return missing; } fn inferred(flag: bool) { if flag { return 1; } return true; }";
    assert!(matches!(
        semantic_error(source),
        SemanticError::IncompatibleInferredReturns {
            expected: Type::Number,
            found: Type::Bool,
            ..
        }
    ));
    let source = "fn explicit() -> number { return missing; } fn inferred() { return 1; }";
    assert!(
        matches!(semantic_error(source), SemanticError::UndefinedVariable { name, .. } if name == "missing")
    );
    let program = Compiler::new()
        .analyze(
            "fn first() -> number { return 3; }
         fn second() -> number { return third(); }
         fn third() { return first(); }
         fn fourth() { return true; }",
        )
        .unwrap();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|function| (&*function.name, function.id))
            .collect::<Vec<_>>(),
        [
            ("first", FunctionId(0)),
            ("second", FunctionId(1)),
            ("third", FunctionId(2)),
            ("fourth", FunctionId(3))
        ]
    );
    for (caller, callee) in [(1, 2), (2, 0)] {
        let TypedStatementKind::Return(Some(value)) = &program.functions[caller].body[0].kind
        else {
            panic!("return")
        };
        assert!(
            matches!(&value.kind, TypedExprKind::Call { func: FunctionRef::User(FunctionId(id)), .. } if *id == callee)
        );
    }
}

const PARITY_SOURCE: &str = r#"
    fn n(x: number) /*number*/ { return sample(x) + 1; }
    fn b(x: bool) /*bool*/ { return !x; }
    fn s() /*string*/ { return "é"; }
    fn a() /*number[]*/ { return [1, 2]; }
    fn v() /*void*/ { emit(3); return; }
    fn r(start: number) /*range*/ { return start..=5 by 2; }
    fn choose(flag: bool) /*number*/ {
        if flag { return { let x = 7; x }; }
        return loop { break 9; };
    }
    fn main() -> number { let total = 0; for i in r(1) { total += i; } return total; }
"#;

fn parity_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(
        ["n", "b", "s", "a", "v", "choose", "main"]
            .map(String::from)
            .to_vec(),
    ));
    compiler
        .register_host_function("sample", "env", "sample", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("emit", "env", "emit", vec![Type::Number], Type::Void)
        .unwrap();
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler
}

fn annotated_parity_source() -> String {
    let mut source = PARITY_SOURCE.to_owned();
    for ty in ["number", "bool", "string", "number[]", "void", "range"] {
        source = source.replace(&format!("/*{ty}*/"), &format!("-> {ty}"));
    }
    source
}

#[derive(Debug, Default)]
struct Decoded {
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
}

fn decode(bytes: &[u8]) -> Decoded {
    Validator::new().validate_all(bytes).unwrap();
    let mut decoded = Decoded::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                for signature in section.into_iter_err_on_gc_types() {
                    let signature = signature.unwrap();
                    decoded
                        .signatures
                        .push((signature.params().to_vec(), signature.results().to_vec()));
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import.unwrap();
                    let TypeRef::Func(index) = import.ty else {
                        panic!("function imports only")
                    };
                    decoded
                        .imports
                        .push((import.module.into(), import.name.into(), index));
                }
            }
            Payload::FunctionSection(section) => decoded
                .functions
                .extend(section.into_iter().map(Result::unwrap)),
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    decoded
                        .exports
                        .push((export.name.into(), export.kind, export.index));
                }
            }
            _ => {}
        }
    }
    decoded
}

#[test]
fn inferred_results_have_explicit_annotation_bytes_and_abi_at_every_level() {
    let mut compiler = parity_compiler();
    let inferred = compiler.analyze(PARITY_SOURCE).unwrap();
    let explicit = compiler.analyze(&annotated_parity_source()).unwrap();
    assert_eq!(
        inferred
            .functions
            .iter()
            .map(|function| function.return_type)
            .collect::<Vec<_>>(),
        [
            Type::Number,
            Type::Bool,
            Type::String,
            Type::NumberArray,
            Type::Void,
            Type::Range,
            Type::Number,
            Type::Number
        ]
    );
    let snapshot = inferred.clone();
    let baseline = compiler.compile(&explicit).unwrap();
    let decoded = decode(&baseline);
    assert_eq!(
        decoded
            .imports
            .iter()
            .map(|(module, field, _)| (module.as_str(), field.as_str()))
            .collect::<Vec<_>>(),
        [("env", "sample"), ("env", "emit"), ("env", "unused")]
    );
    let expected = [
        (vec![ValType::F64], vec![ValType::F64]),
        (vec![ValType::I32], vec![ValType::I32]),
        (vec![], vec![ValType::I32]),
        (vec![], vec![ValType::I32]),
        (vec![], vec![]),
        (vec![ValType::F64], vec![ValType::I32]),
        (vec![ValType::I32], vec![ValType::F64]),
        (vec![], vec![ValType::F64]),
    ];
    for (index, signature) in expected.iter().enumerate() {
        let type_index = usize::try_from(decoded.functions[index]).unwrap();
        assert_eq!(
            &decoded.signatures[type_index], signature,
            "source function {index}"
        );
    }
    for (name, source_index) in [
        ("n", 0),
        ("b", 1),
        ("s", 2),
        ("a", 3),
        ("v", 4),
        ("choose", 6),
        ("main", 7),
    ] {
        assert!(
            decoded
                .exports
                .contains(&(name.into(), ExternalKind::Func, 3 + source_index))
        );
    }
    assert!(!decoded.exports.iter().any(|(name, _, _)| name == "r"));
    assert!(
        decoded
            .exports
            .contains(&("memory".into(), ExternalKind::Memory, 0))
    );
    // An analyzed program owns its import snapshot, including unused registrations.
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&inferred).unwrap(), baseline, "{level:?}");
        assert_eq!(compiler.compile(&explicit).unwrap(), baseline, "{level:?}");
        assert_eq!(inferred, snapshot);
    }
    let mut compiler = parity_compiler();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.analyze(PARITY_SOURCE).unwrap(), inferred);
        assert_eq!(compiler.compile_source(PARITY_SOURCE).unwrap(), baseline);
    }
}

#[test]
fn inferred_ranges_remain_private_and_export_selection_does_not_skip_body_checks() {
    let source = "fn make() { return 1..=5 by 2; } fn main() -> number { let sum = 0; for i in make() { sum += i; } return sum; }";
    let mut compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    assert_eq!(program.functions[0].return_type, Type::Range);
    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::RangeExport {
            function: "make".into()
        })
    );
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    Validator::new()
        .validate_all(&compiler.compile(&program).unwrap())
        .unwrap();
    compiler.set_exports(ExportOptions::None);
    let decoded = decode(&compiler.compile(&program).unwrap());
    assert!(
        decoded
            .exports
            .iter()
            .any(|(name, kind, _)| name == "memory" && *kind == ExternalKind::Memory)
    );
    let source = "fn hidden() { return missing; } fn main() -> number { return 1; }";
    assert!(
        matches!(compiler.compile_source(source), Err(CompileError::Semantic(SemanticError::UndefinedVariable { name, .. })) if name == "missing")
    );
    let program = compiler.analyze("fn read(r: range) { return 1; }").unwrap();
    compiler.set_exports(ExportOptions::All);
    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::RangeExport {
            function: "read".into()
        })
    );
}

#[test]
fn unrelated_inference_does_not_require_annotations_on_an_explicit_recursive_component() {
    let source = "fn a() -> number { return b(); } fn b() -> number { return a(); } fn empty() {}";
    let program = Compiler::new().analyze(source).unwrap();
    assert_eq!(program.functions[2].return_type, Type::Void);
    Validator::new()
        .validate_all(&Compiler::new().compile(&program).unwrap())
        .unwrap();
}

#[test]
fn value_return_syntax_is_found_in_every_nested_raw_expression_region() {
    for snippet in [
        "let x: number = -{ return 1; 0 };",
        "let x: number = 1 + { return 1; 0 };",
        "let x: number[] = [{ return 1; 0 }];",
        "let x: range = 0..{ return 1; 2 };",
        "let x: range = 0..2 by { return 1; 1 };",
        "let a = [0]; a[{ return 1; 0 }] = 1;",
        "let a = [0]; a[0] += { return 1; 0 };",
        "len([{ return 1; 0 }]);",
        "if { return 1; true } {}",
        "while { return 1; true } {}",
        "for i in { return 1; [] } {}",
        "for i in [] { return 1; }",
        "({ return 1; 0 });",
        "(loop { break { return 1; 0 }; });",
        "let x = if true { 0 } else { return 1; 0 };",
        "false && { return 1; true };",
    ] {
        let source = format!("fn spin() {{ loop {{}} {snippet} }}");
        assert!(
            matches!(semantic_error(&source), SemanticError::CannotInferFunctionResult { name, .. } if name == "spin"),
            "{source}"
        );
    }
}

#[test]
fn local_never_inference_errors_precede_function_result_selection() {
    let source = "fn spin() { loop {} let x = -{ return 1; 0 }; }";
    let error = semantic_error(source);
    assert!(matches!(
        &error,
        SemanticError::CannotInferNeverType { name, .. } if name == "x"
    ));
    let span = error.span().unwrap();
    assert_eq!(
        &source[span.start()..span.end()],
        "let x = -{ return 1; 0 };"
    );
}

#[test]
fn large_annotated_to_inferred_cycle_reports_the_unannotated_source_member() {
    let mut source = String::new();
    for index in 0..512 {
        let annotation = if index == 300 { "" } else { "-> number" };
        source.push_str(&format!(
            "fn f{index}() {annotation} {{ return f{}(); }}\n",
            (index + 1) % 512
        ));
    }
    assert_recursive(&source, "f300");
}

#[test]
fn backend_rejects_forged_inferred_signature_and_hidden_return_mismatch_at_every_level() {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    let mut forged = compiler.analyze("fn n() { return 1; }").unwrap();
    let span = forged.functions[0].body[0].span;
    forged.functions[0].return_type = Type::Bool;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile(&forged),
            Err(CodegenError::TypeMismatch {
                function: "n".into(),
                expected: Type::Bool,
                found: Type::Number,
                span,
            })
        );
    }
    let mut forged = compiler.analyze("fn n() { return 1; return 2; }").unwrap();
    let span = forged.functions[0].body[1].span;
    let TypedStatementKind::Return(Some(value)) = &mut forged.functions[0].body[1].kind else {
        panic!("return")
    };
    value.kind = TypedExprKind::Bool(false);
    value.ty = Type::Bool.into();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile(&forged),
            Err(CodegenError::TypeMismatch {
                function: "n".into(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            })
        );
    }
}

#[test]
fn backend_reconstructs_return_payload_category_instead_of_trusting_inferred_metadata() {
    let mut program = Compiler::new().analyze("fn n() { return 1; }").unwrap();
    let span = program.functions[0].body[0].span;
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[0].body[0].kind else {
        panic!("return")
    };
    value.kind = TypedExprKind::Bool(true);
    assert_eq!(
        Compiler::new().compile(&program),
        Err(CodegenError::TypeMismatch {
            function: "n".into(),
            expected: Type::Bool,
            found: Type::Number,
            span,
        })
    );
}

fn run_node(bytes: &[u8], assertions: &str) {
    Validator::new().validate_all(bytes).unwrap();
    let script = format!(
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {{
            env: {{
                sample(value) {{ events.push(value); return value; }},
                emit(value) {{ events.push(value); }},
                unused() {{ throw new Error('unused import executed'); }},
            }}
        }}).exports;
        {assertions}
    "#
    );
    let output = test_process::output(
        Command::new("node").arg("-e").arg(script),
        Some(bytes),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_inferred_all_type_results_preserve_scalar_descriptor_and_internal_range_abi() {
    let mut compiler = parity_compiler();
    let script = r#"
        assert.equal(e.n(4), 5);
        assert.deepEqual(events.splice(0), [4]);
        assert.equal(e.b(0), 1);
        assert.equal(e.b(1), 0);
        const text = e.s();
        let view = new DataView(e.memory.buffer);
        const ptr = view.getUint32(text, true), length = view.getUint32(text + 4, true);
        assert.equal(length, 2);
        assert.equal(new TextDecoder('utf-8', {fatal: true}).decode(new Uint8Array(e.memory.buffer, ptr, length)), 'é');
        const first = e.a(), second = e.a();
        assert.notEqual(first, second);
        view = new DataView(e.memory.buffer);
        const p = view.getUint32(first, true), q = view.getUint32(second, true);
        assert.equal(view.getUint32(first + 4, true), 2);
        assert.equal(view.getFloat64(p, true), 1);
        assert.equal(view.getFloat64(p + 8, true), 2);
        view.setFloat64(p, 99, true);
        assert.equal(view.getFloat64(q, true), 1);
        assert.equal(e.v(), undefined);
        assert.deepEqual(events.splice(0), [3]);
        assert.equal(e.choose(1), 7);
        assert.equal(e.choose(0), 9);
        assert.equal(e.main(), 9);
        assert.equal(e.r, undefined);
        assert.deepEqual(events, []);
    "#;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(&compiler.compile_source(PARITY_SOURCE).unwrap(), script);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_inferred_nested_returns_loops_aliases_named_calls_and_traps_preserve_effect_order() {
    let source = r#"
        fn pair(a: number, b: number) -> number { emit(50); return a * 10 + b; }
        fn returned() {
            return pair(b: sample(1), a: { sample(2); return 77; 0 });
        }
        fn value_loop() {
            return 10 + loop {
                pair(b: sample(3), a: { break 8; 0 });
                break 99;
            };
        }
        fn arrays() {
            let values = [1, 2]; let alias = values; let count = 0;
            for value in values {
                count += value;
                if value == 1 { alias[1] = 8; values = [99]; continue; }
            }
            return count * 100 + alias[1] * 10 + values[0];
        }
        fn range_sum() {
            let sum = 0;
            for i in 1..=5 by 2 { if i == 3 { continue; } sum += i; }
            return sum;
        }
        fn condition() { while { return 7; true } {} }
        fn iterable() { for i in { return 8; [] } {} }
        fn skipped() { return false && { sample(900); true }; }
        fn optional_return() { false && { return 1; true }; return 2; }
        fn trapped() { let a = [1]; a[2] = sample(901); return 0; }
        fn zero() { return -0; }
        fn nan() { return 0 / 0; }
        fn infinity() { return 1 / 0; }
        fn remainder() { return -4 % 2; }
        fn subnormal() { return 0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005; }
    "#;
    let script = r#"
        assert.equal(e.returned(), 77);
        assert.deepEqual(events.splice(0), [1, 2]);
        assert.equal(e.value_loop(), 18);
        assert.deepEqual(events.splice(0), [3]);
        assert.equal(e.arrays(), 1079);
        assert.equal(e.range_sum(), 6);
        assert.equal(e.condition(), 7);
        assert.equal(e.iterable(), 8);
        assert.equal(e.skipped(), 0);
        assert.equal(e.optional_return(), 2);
        assert.deepEqual(events, []);
        assert.throws(() => e.trapped(), WebAssembly.RuntimeError);
        assert.deepEqual(events, []);
        assert.ok(Object.is(e.zero(), -0));
        assert.ok(Number.isNaN(e.nan()));
        assert.equal(e.infinity(), Infinity);
        assert.ok(Object.is(e.remainder(), -0));
        assert.equal(e.subnormal(), Number.MIN_VALUE);
    "#;
    let mut compiler = parity_compiler();
    compiler.set_exports(ExportOptions::All);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(&compiler.compile_source(source).unwrap(), script);
    }
}

#[test]
#[ignore = "requires Node.js with ESM support and a freshly rebuilt browser package in pkg"]
fn node_rebuilt_browser_result_inference_matches_native_bytes_diagnostics_and_recovery() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let package = root.join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "missing {}; rebuild the isolated browser package before this ignored test",
            package.join(name).display()
        );
    }

    // Length-prefixed fields keep native bytes and exact UTF-8 diagnostics out of
    // command-line quoting and avoid writing temporary fixtures or adding serde.
    let mut input = Vec::new();
    let mut field = |bytes: &[u8]| {
        let length = u32::try_from(bytes.len()).unwrap();
        input.extend_from_slice(&length.to_le_bytes());
        input.extend_from_slice(bytes);
    };
    let explicit = annotated_parity_source();
    field(PARITY_SOURCE.as_bytes());
    field(explicit.as_bytes());
    let mut compiler = parity_compiler();
    let mut expected = Vec::with_capacity(LEVELS.len());
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(PARITY_SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(compiler.compile_source(&explicit).unwrap(), bytes);
        field(&bytes);
        expected.push(bytes);
    }

    let cases = [
        (
            "// é\r\n\tfn recurse() {\r\n\treturn recurse();\r\n}\r\n",
            "fn recurse() {\r\n\treturn recurse();\r\n}",
            "<browser>:2:2: semantic error:",
        ),
        (
            "// é\r\nfn callee() { return true; }\r\nfn caller() {\r\n\treturn callee() + 1;\r\n}\r\n",
            "return callee() + 1;",
            "<browser>:4:2: semantic error:",
        ),
        (
            "// é\r\n\tfn mixed(flag: bool) {\r\n\tif flag { return 1; }\r\n\treturn true;\r\n}\r\n",
            "fn mixed(flag: bool) {\r\n\tif flag { return 1; }\r\n\treturn true;\r\n}",
            "<browser>:2:2: semantic error:",
        ),
        (
            "// é\r\n\tfn spin() {\r\n\treturn loop {};\r\n}\r\n",
            "fn spin() {\r\n\treturn loop {};\r\n}",
            "<browser>:2:2: semantic error:",
        ),
    ];
    for (index, (source, excerpt, header)) in cases.into_iter().enumerate() {
        let error = compiler.compile_source(source).unwrap_err();
        match (index, &error) {
            (
                0,
                CompileError::Semantic(SemanticError::RecursiveResultAnnotationRequired {
                    name,
                    ..
                }),
            ) => {
                assert_eq!(name, "recurse");
            }
            (
                1,
                CompileError::Semantic(SemanticError::InvalidBinaryOperands {
                    operator,
                    left: Type::Bool,
                    right: Type::Number,
                    ..
                }),
            ) if *operator == "+" => {}
            (
                2,
                CompileError::Semantic(SemanticError::IncompatibleInferredReturns {
                    expected: Type::Number,
                    found: Type::Bool,
                    ..
                }),
            ) => {}
            (3, CompileError::Semantic(SemanticError::CannotInferFunctionResult { name, .. })) => {
                assert_eq!(name, "spin");
            }
            _ => panic!("unexpected native diagnostic for {source}: {error:?}"),
        }
        let span = error.span().unwrap();
        assert_eq!(&source[span.start()..span.end()], excerpt);
        let message = was_compiler::diagnostic::render("<browser>", source, &error);
        assert!(message.starts_with(header), "{message}");
        assert!(!message.contains('\r'));
        assert!(!message.contains('\t'));
        assert!(message.contains('^'));
        field(source.as_bytes());
        field(message.as_bytes());
        for (level, bytes) in LEVELS.into_iter().zip(&expected) {
            compiler.set_optimization_level(level);
            let current = compiler.compile_source(source).unwrap_err();
            assert_eq!(
                was_compiler::diagnostic::render("<browser>", source, &current),
                message
            );
            assert_eq!(&compiler.compile_source(PARITY_SOURCE).unwrap(), bytes);
            assert_eq!(&compiler.compile_source(&explicit).unwrap(), bytes);
        }
    }

    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        const path = require('node:path');
        const { pathToFileURL } = require('node:url');
        async function main() {
            const input = fs.readFileSync(0);
            let offset = 0;
            function field() {
                assert.ok(offset + 4 <= input.length);
                const length = input.readUInt32LE(offset);
                offset += 4;
                assert.ok(offset + length <= input.length);
                const bytes = input.subarray(offset, offset + length);
                offset += length;
                return bytes;
            }
            const inferred = field().toString('utf8');
            const explicit = field().toString('utf8');
            const native = [field(), field(), field(), field()];
            const diagnostics = Array.from({length: 4}, () => ({
                source: field().toString('utf8'),
                message: field().toString('utf8'),
            }));
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const { default: init, Compiler } = await import(
                pathToFileURL(path.join(directory, 'was_compiler.js')).href
            );
            await init({ module_or_path: fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm')) });
            const compiler = new Compiler();
            try {
                compiler.registerHostFunction('sample', 'env', 'sample', ['number'], 'number');
                compiler.registerHostFunction('emit', 'env', 'emit', ['number'], 'void');
                compiler.registerHostFunction('unused', 'env', 'unused', [], 'void');
                compiler.setExports(['n', 'b', 's', 'a', 'v', 'choose', 'main']);
                function sameBytes(source, expected) {
                    const bytes = compiler.compile(source);
                    assert.ok(bytes instanceof Uint8Array);
                    assert.ok(WebAssembly.validate(bytes));
                    assert.deepEqual(Buffer.from(bytes), expected);
                }
                for (const level of [0, 1, 2, 3]) {
                    compiler.setOptimizationLevel(level);
                    sameBytes(inferred, native[level]);
                    sameBytes(explicit, native[level]);
                    for (const diagnostic of diagnostics) {
                        assert.throws(() => compiler.compile(diagnostic.source), error => {
                            assert.ok(error instanceof Error);
                            assert.equal(error.name, 'Error');
                            assert.equal(error.message, diagnostic.message);
                            return true;
                        });
                        sameBytes(inferred, native[level]);
                        sameBytes(explicit, native[level]);
                    }
                }
            } finally {
                compiler.free();
            }
        }
        main().catch(error => { console.error(error); process.exitCode = 1; });
    "#;
    let output = test_process::output(
        Command::new("node")
            .arg("-e")
            .arg(script)
            .arg(package)
            .current_dir(root),
        Some(&input),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "browser inference parity failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
