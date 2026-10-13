#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type, codegen,
    semantic::SemanticError,
    typed_ast::{FunctionId, FunctionRef, TypedExprKind, TypedStatementKind},
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

#[test]
fn forward_chain_resolves_concrete_results() {
    let program = Compiler::new()
        .analyze("fn first() { return second(); } fn second() { return third(); } fn third() { return 42; }")
        .unwrap();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|function| function.return_type)
            .collect::<Vec<_>>(),
        [Type::Number; 3]
    );
}

#[test]
fn diamonds_join_never_with_concrete_results_in_either_declaration_order() {
    let declarations = [
        "fn root(flag: bool) { return if flag { left(flag) } else { right(flag) }; }",
        "fn left(flag: bool) { return if flag { leaf() } else { loop {} }; }",
        "fn right(flag: bool) { return loop { break leaf(); }; }",
        "fn leaf() { return 7; }",
    ];
    for source in [
        declarations.join("\n"),
        declarations
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"),
    ] {
        let mut compiler = Compiler::new();
        let program = compiler.analyze(&source).unwrap();
        assert!(
            program
                .functions
                .iter()
                .all(|function| function.return_type == Type::Number)
        );
        for level in LEVELS {
            compiler.set_optimization_level(level);
            Validator::new()
                .validate_all(&compiler.compile(&program).unwrap())
                .unwrap();
        }
    }
}

#[test]
fn chain_to_void_infers_void_without_inventing_never_signatures() {
    let program = Compiler::new()
        .analyze("fn first() { second(); } fn second() { third(); return; } fn third() { loop {} }")
        .unwrap();
    assert!(
        program
            .functions
            .iter()
            .all(|function| function.return_type == Type::Void)
    );
    Validator::new()
        .validate_all(&Compiler::new().compile(&program).unwrap())
        .unwrap();
}

#[test]
fn recursive_omitted_results_remain_rejected_even_with_concrete_anchors() {
    for source in [
        "fn first() { return second(); } fn second() { return first(); }",
        "fn first() { return second(); } fn second() -> number { return first(); }",
        "fn first() { return 1; first(); }",
        "fn first() { return true || second(); } fn second() { return first(); }",
    ] {
        assert!(
            matches!(
                Compiler::new().analyze(source),
                Err(CompileError::Semantic(SemanticError::RecursiveResultAnnotationRequired { name, .. })) if name == "first"
            ),
            "{source}"
        );
    }
}

#[test]
fn resolved_chains_still_check_unreachable_returns_against_concrete_results() {
    let source = "fn first() { return second(); return false; } fn second() { return 1; }";
    assert!(matches!(
        Compiler::new().analyze(source),
        Err(CompileError::Semantic(SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        }))
    ));
}

#[derive(Debug, Default)]
struct Decoded {
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    calls: Vec<Vec<u32>>,
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
                        panic!("function import")
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
            Payload::CodeSectionEntry(body) => {
                let mut reader = body.get_operators_reader().unwrap();
                let mut calls = Vec::new();
                while !reader.eof() {
                    if let Operator::Call { function_index } = reader.read().unwrap() {
                        calls.push(function_index);
                    }
                }
                decoded.calls.push(calls);
            }
            _ => {}
        }
    }
    decoded
}

fn host_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("sample", "env", "sample", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler
}

#[test]
fn scheduling_preserves_owned_source_order_call_ids_import_snapshot_and_layout() {
    let mut compiler = host_compiler();
    let program = {
        let source = String::from(
            "fn first(x: number) { return second(x); } fn second(x: number) { return third(x); } fn third(x: number) { return sample(x); }",
        );
        compiler.analyze(&source).unwrap()
    };
    let snapshot = program.clone();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.id, function.return_type))
            .collect::<Vec<_>>(),
        [
            ("first", FunctionId(0), Type::Number),
            ("second", FunctionId(1), Type::Number),
            ("third", FunctionId(2), Type::Number),
        ]
    );
    for (caller, callee) in [(0, 1), (1, 2)] {
        let TypedStatementKind::Return(Some(value)) = &program.functions[caller].body[0].kind
        else {
            panic!("return")
        };
        assert!(
            matches!(&value.kind, TypedExprKind::Call { func: FunctionRef::User(FunctionId(id)), .. } if *id == callee)
        );
        assert_eq!(value.ty, Type::Number);
    }
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    let baseline = compiler.compile(&program).unwrap();
    let decoded = decode(&baseline);
    assert_eq!(
        decoded.imports,
        [
            ("env".into(), "sample".into(), 0),
            ("env".into(), "unused".into(), 1)
        ]
    );
    assert_eq!(
        decoded.signatures,
        [(vec![ValType::F64], vec![ValType::F64]), (vec![], vec![])]
    );
    assert_eq!(decoded.functions, [0, 0, 0]);
    assert_eq!(decoded.calls, [vec![3], vec![4], vec![0]]);
    assert_eq!(
        decoded.exports,
        [
            ("first".into(), ExternalKind::Func, 2),
            ("second".into(), ExternalKind::Func, 3),
            ("third".into(), ExternalKind::Func, 4)
        ]
    );
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), baseline);
        assert_eq!(program, snapshot);
    }
}

const PARITY_SOURCE: &str = r#"
    fn text() /*string*/ { return text_leaf(); }
    fn array() /*number[]*/ { return array_leaf(); }
    fn ranges() /*range*/ { return range_leaf(); }
    fn main() /*number*/ { return left() + right(); }
    fn left() /*number*/ { return leaf(); }
    fn right() /*number*/ { return leaf() + 1; }
    fn leaf() /*number*/ { let a = array(); let total = a[0]; for i in ranges() { total += i; } if text() == "é" { total += 1; } return total; }
    fn text_leaf() /*string*/ { return "é"; }
    fn array_leaf() /*number[]*/ { return [4, 8]; }
    fn range_leaf() /*range*/ { return 1..=5 by 2; }
"#;

fn explicit_source() -> String {
    let mut source = PARITY_SOURCE.to_owned();
    for ty in ["number", "string", "number[]", "range"] {
        source = source.replace(&format!("/*{ty}*/"), &format!("-> {ty}"));
    }
    source
}

#[test]
fn chains_have_explicit_bytes_and_concrete_descriptor_signatures_at_all_levels() {
    let mut compiler = Compiler::new();
    let inferred = compiler.analyze(PARITY_SOURCE).unwrap();
    let explicit = compiler.analyze(&explicit_source()).unwrap();
    assert_eq!(
        inferred
            .functions
            .iter()
            .map(|function| function.return_type)
            .collect::<Vec<_>>(),
        [
            Type::String,
            Type::NumberArray,
            Type::Range,
            Type::Number,
            Type::Number,
            Type::Number,
            Type::Number,
            Type::String,
            Type::NumberArray,
            Type::Range
        ]
    );
    assert_eq!(
        compiler.compile(&inferred),
        Err(CodegenError::RangeExport {
            function: "ranges".into()
        })
    );
    for exports in [
        ExportOptions::Only(vec!["text".into(), "array".into(), "main".into()]),
        ExportOptions::None,
    ] {
        compiler.set_exports(exports.clone());
        let baseline = compiler.compile(&explicit).unwrap();
        let decoded = decode(&baseline);
        assert!(decoded.imports.is_empty());
        for (index, result) in [
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::F64,
            ValType::F64,
            ValType::F64,
            ValType::F64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                decoded.signatures[usize::try_from(decoded.functions[index]).unwrap()],
                (vec![], vec![result]),
                "function {index}"
            );
        }
        assert!(
            decoded
                .exports
                .contains(&("memory".into(), ExternalKind::Memory, 0))
        );
        let functions = decoded
            .exports
            .iter()
            .filter(|(_, kind, _)| *kind == ExternalKind::Func)
            .cloned()
            .collect::<Vec<_>>();
        match exports {
            ExportOptions::None => {
                assert_eq!(functions, [("__alloc".into(), ExternalKind::Func, 10)])
            }
            ExportOptions::Only(_) => assert_eq!(
                functions,
                [
                    ("text".into(), ExternalKind::Func, 0),
                    ("array".into(), ExternalKind::Func, 1),
                    ("main".into(), ExternalKind::Func, 3),
                    ("__alloc".into(), ExternalKind::Func, 10)
                ]
            ),
            ExportOptions::All => unreachable!(),
        }
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&inferred).unwrap(), baseline, "{level:?}");
            assert_eq!(compiler.compile(&explicit).unwrap(), baseline, "{level:?}");
            assert_eq!(
                compiler.compile_source(PARITY_SOURCE).unwrap(),
                baseline,
                "{level:?}"
            );
        }
    }
}

#[test]
fn checked_backend_rejects_forged_chain_signatures_and_hidden_body_mismatches() {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    let program = compiler
        .analyze("fn first() { return second(); } fn second() { return 1; return 2; }")
        .unwrap();
    let mut signature = program.clone();
    signature.functions[0].return_type = Type::Bool;
    let signature_error = CodegenError::TypeMismatch {
        function: "first".into(),
        expected: Type::Bool,
        found: Type::Number,
        span: signature.functions[0].body[0].span,
    };
    let mut body = program.clone();
    let span = body.functions[1].body[1].span;
    let TypedStatementKind::Return(Some(value)) = &mut body.functions[1].body[1].kind else {
        panic!("return")
    };
    value.kind = TypedExprKind::Bool(false);
    value.ty = Type::Bool.into();
    let body_error = CodegenError::TypeMismatch {
        function: "second".into(),
        expected: Type::Number,
        found: Type::Bool,
        span,
    };
    for (forged, error) in [(&signature, signature_error), (&body, body_error)] {
        assert_eq!(
            codegen::compile_with_exports(forged, &ExportOptions::None).unwrap_err(),
            error
        );
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(forged).unwrap_err(), error);
        }
    }
    let mut metadata = program;
    let span = metadata.functions[0].body[0].span;
    let TypedStatementKind::Return(Some(value)) = &mut metadata.functions[0].body[0].kind else {
        panic!("return")
    };
    value.ty = Type::Bool.into();
    assert_eq!(
        codegen::compile(&metadata),
        Err(CodegenError::TypeMismatch {
            function: "first".into(),
            expected: Type::Number,
            found: Type::Bool,
            span
        })
    );
}

fn run_node(bytes: &[u8], assertions: &str) {
    Validator::new().validate_all(bytes).unwrap();
    let script = format!(
        r#"
        const assert = require('node:assert/strict');
        const events = [];
        const bytes = require('node:fs').readFileSync(0);
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {{env: {{
            sample(value) {{ events.push(value); return value; }},
            unused() {{ throw new Error('unused import executed'); }}
        }} }}).exports;
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
fn node_forward_chain_and_diamond_preserve_values_and_effect_order() {
    let source = "fn main() { return left() + right(); } fn left() { return chain(2); } fn right() { return chain(3); } fn chain(x: number) { return leaf(x); } fn leaf(x: number) { return sample(x) * 10; }";
    let mut compiler = host_compiler();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(
            &compiler.compile_source(source).unwrap(),
            "assert.equal(e.main(), 50); assert.deepEqual(events, [2, 3]);",
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_descriptor_and_internal_range_chains_execute_bounded_diamond() {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec![
        "main".into(),
        "array".into(),
        "text".into(),
    ]));
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(
            &compiler.compile_source(PARITY_SOURCE).unwrap(),
            r#"
            assert.equal(e.main(), 29);
            const first = e.array(), second = e.array();
            assert.notEqual(first, second);
            let view = new DataView(e.memory.buffer);
            const p = view.getUint32(first, true), q = view.getUint32(second, true);
            assert.equal(view.getUint32(first + 4, true), 2);
            assert.equal(view.getFloat64(p, true), 4);
            view.setFloat64(p, 99, true);
            assert.equal(view.getFloat64(q, true), 4);
            const text = e.text();
            view = new DataView(e.memory.buffer);
            const ptr = view.getUint32(text, true), length = view.getUint32(text + 4, true);
            assert.equal(length, 2);
            assert.equal(new TextDecoder('utf-8', {fatal: true}).decode(new Uint8Array(e.memory.buffer, ptr, length)), 'é');
            assert.equal(e.ranges, undefined);
            assert.deepEqual(events, []);
        "#,
        );
    }
}
