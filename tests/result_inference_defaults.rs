#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type, codegen,
    semantic::SemanticError,
    tokens::Span,
    typed_ast::{CallBindingOrigin, TypedExprKind, TypedStatementKind},
};
use wasmparser::{Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn error(source: &str) -> SemanticError {
    match Compiler::new().analyze(source) {
        Err(CompileError::Semantic(error)) => error,
        other => panic!("expected semantic error for {source}: {other:?}"),
    }
}

fn span(source: &str, fragment: &str) -> Span {
    let start = source.find(fragment).unwrap();
    Span::new(start, start + fragment.len())
}

fn compile_valid(source: &str) {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    let program = compiler.analyze(source).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
    }
}

#[test]
fn inferred_defaulted_callee_is_available_to_annotated_caller() {
    let source = "fn target(a: number, b: number = a + 1, c: number = b * 2) { return c; } fn main() -> number { return target(3); }";
    let program = Compiler::new().analyze(source).unwrap();
    assert_eq!(program.functions[0].return_type, Type::Number);
    assert_eq!(program.functions[0].params.len(), 3);
    compile_valid(source);
}

#[test]
fn inferred_caller_to_annotated_named_default_target_decodes_full_arity_and_locals() {
    let source = "fn target(a: number, b: number = a + 1, c: number = b * 2) -> number { return c; } fn main() { return target(c: 9, a: 3); }";
    let program = Compiler::new().analyze(source).unwrap();
    assert_eq!(program.functions[1].return_type, Type::Number);
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized call")
    };
    assert_eq!(
        sequence
            .bindings
            .iter()
            .map(|b| b.origin)
            .collect::<Vec<_>>(),
        [
            CallBindingOrigin::Explicit { argument: 0 },
            CallBindingOrigin::Explicit { argument: 1 },
            CallBindingOrigin::Default { formal: 1 },
        ]
    );
    assert_eq!(sequence.bindings[2].span, span(source, "a + 1"));
    for level in LEVELS {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let mut signatures = Vec::new();
        let mut bodies = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::TypeSection(reader) => {
                    signatures.extend(reader.into_iter_err_on_gc_types().map(Result::unwrap))
                }
                Payload::CodeSectionEntry(body) => bodies.push(body),
                _ => {}
            }
        }
        assert_eq!(signatures[0].params(), [ValType::F64; 3]);
        assert_eq!(signatures[0].results(), [ValType::F64]);
        assert_eq!(signatures[1].params(), []);
        assert_eq!(signatures[1].results(), [ValType::F64]);
        let locals: Vec<_> = bodies[1]
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(locals, [(1, ValType::F64); 3]);
        let ops: Vec<_> = bodies[1]
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert!(matches!(ops.as_slice(), [
            Operator::F64Const { value: nine }, Operator::LocalSet { local_index: 0 },
            Operator::F64Const { value: three }, Operator::LocalSet { local_index: 1 },
            Operator::LocalGet { local_index: 1 }, Operator::F64Const { value: one }, Operator::F64Add, Operator::LocalSet { local_index: 2 },
            Operator::LocalGet { local_index: 1 }, Operator::LocalGet { local_index: 2 }, Operator::LocalGet { local_index: 0 },
            Operator::Call { function_index: 0 }, Operator::Return, Operator::End,
        ] if nine.bits() == 9f64.to_bits() && three.bits() == 3f64.to_bits() && one.bits() == 1f64.to_bits()));
    }
}

#[test]
fn inferred_defaulted_signatures_match_explicit_bytes_for_every_concrete_type() {
    for (parameters, body, result, call, expected) in [
        ("x: number = 3", "return x;", "number", "f()", Type::Number),
        ("x: bool = true", "return x;", "bool", "f()", Type::Bool),
        (
            "x: string = \"é\"",
            "return x;",
            "string",
            "f()",
            Type::String,
        ),
        (
            "x: number[], y: number[] = x",
            "return y;",
            "number[]",
            "f([1])",
            Type::NumberArray,
        ),
        (
            "x: range, y: range = x",
            "return y;",
            "range",
            "f(0..3)",
            Type::Range,
        ),
        ("x: number = 1", "return;", "void", "f()", Type::Void),
    ] {
        let inferred = format!(
            "fn f({parameters}) {{ {body} }} fn main() -> {result} {{ {} }}",
            if expected == Type::Void {
                format!("{call}; return;")
            } else {
                format!("return {call};")
            }
        );
        let explicit = inferred.replacen(
            &format!("fn f({parameters}) {{"),
            &format!("fn f({parameters}) -> {result} {{"),
            1,
        );
        let mut compiler = Compiler::new();
        compiler.set_exports(if expected == Type::Range {
            ExportOptions::None
        } else {
            ExportOptions::Only(vec!["f".into(), "main".into()])
        });
        let program = compiler.analyze(&inferred).unwrap();
        assert_eq!(program.functions[0].return_type, expected);
        let mut baseline = None;
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile_source(&inferred).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(bytes, compiler.compile_source(&explicit).unwrap());
            assert_eq!(bytes, compiler.compile(&program).unwrap());
            if let Some(previous) = &baseline {
                assert_eq!(&bytes, previous);
            }
            baseline = Some(bytes);
        }
    }
}

#[test]
fn defaults_validate_before_scc_dependencies_and_probes_even_when_unused_or_supplied() {
    for graph in [
        "fn recur() { return recur(); }",
        "fn caller() { return target(); } fn target() { return 1; }",
        "fn bad() { if true { return 1; } return true; }",
    ] {
        let overflow = "9".repeat(400);
        for expression in [
            "absent()",
            "false && absent()",
            "missing",
            "true",
            overflow.as_str(),
        ] {
            let ty = "number";
            let source = format!(
                "{graph} fn unused(value: {ty} = {expression}) {{ return value; }} fn supplied() -> number {{ return unused(2); }}"
            );
            let actual = error(&source);
            assert!(
                match expression {
                    "absent()" | "false && absent()" =>
                        matches!(actual, SemanticError::InvalidDefaultExpression { .. }),
                    "missing" => matches!(actual, SemanticError::UndefinedVariable { .. }),
                    "true" => matches!(actual, SemanticError::TypeMismatch { .. }),
                    _ => matches!(actual, SemanticError::DefaultNumberLiteralOutOfRange { .. }),
                },
                "{source}: {actual:?}"
            );
            let start = source.find("value: number = ").unwrap() + "value: number = ".len();
            assert_eq!(
                actual.span(),
                Some(Span::new(start, start + expression.len()))
            );
        }
    }
}

#[test]
fn default_errors_keep_declaration_origins_and_explicit_errors_keep_argument_origins() {
    let source = "// é

fn caller() { return f(value: true); }

fn f(value: number = 1) -> number { return value; }";
    assert!(matches!(error(source), SemanticError::TypeMismatch { .. }));
    assert_eq!(error(source).span(), Some(span(source, "value: true")));
    let source = "// é

fn caller() { return f(value: 2); }

fn f(value: number = unknown) -> number { return value; }";
    assert!(matches!(
        error(source),
        SemanticError::UndefinedVariable { .. }
    ));
    assert_eq!(error(source).span(), Some(span(source, "unknown")));
}

#[test]
fn defaulted_calls_retain_conservative_lexical_recursion_and_shadowing() {
    for source in [
        "fn f(x: number = 1) { return 1; f(); }",
        "fn f(x: number = 1) { return false && f(x: 2); }",
        "fn f(x: number = 1) { return g(); } fn g(y: number = 2) -> number { return f(); }",
        "fn f(x: number = 1) { return 1; g(y: f()); } fn g(y: number, z: number = y) -> number { return z; }",
    ] {
        let actual = error(source);
        assert!(
            matches!(&actual, SemanticError::RecursiveResultAnnotationRequired { name, .. } if name == "f"),
            "{actual:?}"
        );
        assert_eq!(
            actual.span(),
            Some(span(source, source.split(" fn g").next().unwrap()))
        );
    }
    let source = "fn f(x: number = 1) { let f = 1; return f(x: 2); }";
    assert!(matches!(error(source), SemanticError::NotCallable { name, .. } if name == "f"));
    compile_valid(
        "fn f(x: number = 1) -> number { if x == 0 { return 0; } return f(x - 1); } fn caller() { return f(); }",
    );
}

#[test]
fn resolved_dependencies_include_defaulted_calls_and_skipped_explicit_arguments() {
    for body in [
        "return target();",
        "return target(value: 3);",
        "return 1; target();",
        "false && target() == 1; return 1;",
        "return consume(value: target());",
        "return consume(value: { return 1; target() });",
    ] {
        for target_first in [false, true] {
            let target = "fn target(value: number = 1) { return value; }";
            let caller = format!(
                "fn caller() {{ {body} }} fn consume(value: number, other: number = value) -> number {{ return other; }}"
            );
            let source = if target_first {
                format!("{target} {caller}")
            } else {
                format!("{caller} {target}")
            };
            let program = Compiler::new().analyze(&source).unwrap();
            assert_eq!(
                program.functions[usize::from(target_first)].return_type,
                Type::Number
            );
            compile_valid(&source);
        }
    }
}

#[test]
fn normalized_explicit_returns_constrain_only_reachable_caller_flow() {
    for body in [
        "return consume(value: { return 7; true });",
        "consume(value: { return 7; true }); return 8;",
        "loop { consume(value: { return 7; true }); }",
        "return loop { consume(value: { break 7; true }); break 8; };",
        "let n = 0; for i in [1, 2] { n += 1; consume(value: { continue; true }); } return n;",
    ] {
        let source = format!(
            "fn caller() {{ {body} }} fn consume(value: bool, other: bool = value) -> bool {{ return other; }}"
        );
        let program = Compiler::new().analyze(&source).unwrap();
        assert_eq!(program.functions[0].return_type, Type::Number, "{source}");
        compile_valid(&source);
    }
    let source = "fn caller() { consume(value: { return 7; true }, other: { return false; true }); } fn consume(value: bool, other: bool = value) -> bool { return other; }";
    assert!(matches!(error(source), SemanticError::TypeMismatch { .. }));
}

#[test]
fn defaulted_functions_keep_no_implicit_return_and_never_only_annotation_rules() {
    assert!(matches!(
        Compiler::new().analyze("fn f(x: number = 1) { x }"),
        Err(CompileError::Parse(_))
    ));
    for body in ["return loop {};", "loop {} return 1;", "return; return 1;"] {
        let source = format!("fn f(x: number = 1) {{ {body} }}");
        assert!(
            matches!(error(&source), SemanticError::CannotInferFunctionResult { name, .. } if name == "f")
        );
    }
    compile_valid(
        "fn f(x: number = 1) { loop {} } fn g(x: number = 1) -> string { return loop {}; }",
    );
    let source = "fn f(flag: bool = true) { if flag { return 1; } }";
    assert!(matches!(error(source), SemanticError::MissingReturn { .. }));
}

#[test]
fn forged_default_child_in_inferred_program_keeps_checked_lowering_origin() {
    let source = "fn target(a: number, b: number = a + 1) { return b; } fn main() -> number { return target(3); }";
    let mut program = Compiler::new().analyze(source).unwrap();
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &mut value.kind else {
        panic!("normalized")
    };
    let default_span = sequence.bindings[1].span;
    sequence.bindings[1].value.kind = TypedExprKind::Number("NaN".into());
    assert!(
        matches!(codegen::compile(&program), Err(CodegenError::InvalidNumberLiteral { span, .. }) if span == default_span)
    );
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert!(
            matches!(compiler.compile(&program), Err(CodegenError::InvalidNumberLiteral { span, .. }) if span == default_span)
        );
    }
}

fn run(source: &str, assertions: &str) {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "mark",
            "env",
            "mark",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .unwrap();
    compiler
        .register_host_function(
            "corrupt",
            "env",
            "corrupt",
            vec![Type::NumberArray],
            Type::Number,
        )
        .unwrap();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let mut baseline = None;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        if let Some(previous) = &baseline {
            assert_eq!(&bytes, previous);
        }
        baseline = Some(bytes.clone());
        let script = format!(
            r#"
            const assert = require("node:assert/strict");
            const events = [];
            let e;
            e = new WebAssembly.Instance(new WebAssembly.Module(require("node:fs").readFileSync(0)), {{ env: {{
                mark(id, value) {{
                    events.push(id);
                    return value;
                }},
                corrupt(handle) {{
                    events.push(99);
                    new DataView(e.memory.buffer).setUint32(handle + 4, 0xffffffff, true);
                    return 0;
                }}
            }} }}).exports;
            {assertions}
        "#
        );
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn inferred_caller_captures_scalars_and_observes_shared_handle_defaults_in_written_order() {
    run(
        r#"
        fn capture(a: number, b: number = a + 1, c: number = b * 2) -> number { return a * 100 + b * 10 + c; }
        fn inspect(xs: number[], trigger: number, copy: number[] = xs, n: number = len(copy), same: bool = xs == copy) -> number {
            copy[0] += 1;
            return xs[0] * 100 + n * 10 + if same { 1 } else { 0 };
        }
        fn text(a: string, b: string = a, same: bool = a == b) -> bool { return same; }
        fn ranges(a: range, b: range = a) -> bool { return 2 in b; }
        fn main() {
            let x = 3;
            let scalar = capture(c: { mark(1, 0); x = 9; 8 }, a: mark(2, x));
            let xs = [1, 2]; let alias = xs;
            let arrays = inspect(xs: xs, trigger: { alias[0] = 8; xs = [99]; mark(3, 0) });
            return scalar + arrays + alias[0] + xs[0] + if text("é") && ranges(0..3) { 1 } else { 0 };
        }
    "#,
        r#"
        assert.equal(e.main(), 2038);
        assert.deepEqual(events.splice(0), [1, 2, 3]);
        assert.equal(e.main(), 2038);
        assert.deepEqual(events, [1, 2, 3]);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn inferred_defaulted_callee_executes_dependent_defaults_per_invocation_with_raw_full_arity() {
    let source = r#"
        fn target(a: number, b: number = a + 1, c: number = b * 2) { return a * 100 + b * 10 + c; }
        fn main() -> number { return target(3) + target(c: 9, a: 2); }
    "#;
    for level in LEVELS {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let script = r#"
            const assert = require("node:assert/strict");
            const e = new WebAssembly.Instance(new WebAssembly.Module(require("node:fs").readFileSync(0))).exports;
            assert.equal(e.main(), 587);
            assert.equal(e.target.length, 3);
            assert.equal(e.target(3, 7, 8), 378);
            assert.equal(e.target(3, 0, 0), 300);
            assert.ok(Number.isNaN(e.target(3)));
        "#;
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn inferred_callers_preserve_return_break_continue_and_skip_later_effects() {
    for (body, expected, events) in [
        (
            "return 5 + consume(value: { mark(1, 0); return 77; true }, trigger: mark(99, 0));",
            77,
            "[1]",
        ),
        (
            "return 10 + loop { consume(value: { mark(2, 0); break 8; true }, trigger: mark(99, 0)); break 99; };",
            18,
            "[2]",
        ),
        (
            "let n = 0; for i in [1, 2, 3] { n += 1; consume(value: { mark(10 + i, 0); continue; true }, trigger: mark(99, 0)); } return n;",
            3,
            "[11, 12, 13]",
        ),
    ] {
        let source = format!(
            "fn main() {{ {body} }} fn consume(value: bool, trigger: number, copy: bool = value) -> bool {{ mark(98, 0); return copy; }}"
        );
        run(
            &source,
            &format!("assert.equal(e.main(), {expected}); assert.deepEqual(events, {events});"),
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn pure_defaults_can_trap_after_explicit_effects_without_folding_ieee_arithmetic() {
    run(
        r#"
        fn consume(xs: number[], trigger: number, n: number = len(xs)) -> number { return n; }
        fn main() { let xs = [1]; return consume(xs: xs, trigger: corrupt(xs)); }
    "#,
        r#"
        assert.throws(() => e.main(), WebAssembly.RuntimeError);
        assert.deepEqual(events, [99]);
    "#,
    );
    run(
        r#"
        fn capture(a: number, b: number = a, c: number = b) -> number { return a * 100 + b * 10 + c; }
        fn main() { let x = 7; let result = capture(a: x, c: { x = 9; mark(3, x) }); return result + x; }
    "#,
        r#"
        assert.equal(e.main(), 788);
        assert.deepEqual(events, [3]);
    "#,
    );
    for (default, assertion) in [
        ("-0", "assert.ok(Object.is(e.main(), -0));"),
        ("1 / 0", "assert.equal(e.main(), Infinity);"),
        ("0 / 0", "assert.ok(Number.isNaN(e.main()));"),
        ("-4 % 2", "assert.ok(Object.is(e.main(), -0));"),
        (
            "0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
            "assert.equal(e.main(), 0);",
        ),
    ] {
        let source = format!(
            "fn scalar(x: number = {default}) -> number {{ return x; }} fn main() {{ return scalar(); }}"
        );
        run(&source, assertion);
    }
}

#[test]
fn resolved_call_type_errors_keep_default_normalized_written_argument_origins() {
    for (body, origin, expected_operator) in [
        ("return consume(target());", "target()", None),
        (
            "return consume(target(), 0);",
            "return consume(target(), 0);",
            None,
        ),
        ("return consume(value: target());", "value: target()", None),
        (
            "return consume({ return 1; target() + 0 });",
            "{ return 1; target() + 0 }",
            Some("+"),
        ),
        (
            "return consume(false && target() == 1);",
            "false && target() == 1",
            Some("=="),
        ),
    ] {
        let source = format!(
            "fn caller() {{ {body} }} fn target() {{ return true; }} fn consume(value: number, other: number = 0) -> number {{ return value; }}"
        );
        let actual = error(&source);
        if let Some(expected_operator) = expected_operator {
            assert!(
                matches!(&actual, SemanticError::InvalidBinaryOperands { operator, left: Type::Bool, right: Type::Number, .. } if *operator == expected_operator),
                "{source}: {actual:?}"
            );
        } else {
            assert!(
                matches!(
                    actual,
                    SemanticError::TypeMismatch {
                        expected: Type::Number,
                        found: Type::Bool,
                        ..
                    }
                ),
                "{source}: {actual:?}"
            );
        }
        assert_eq!(actual.span(), Some(span(&source, origin)), "{source}");
    }
}
