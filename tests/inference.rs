#[path = "support/process.rs"]
mod test_process;

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type, typed_ast::TypedStatementKind,
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const ANNOTATED: &str = r#"
fn main(input: number) -> number {
    let n: number = 10;
    let flag: bool = true;
    let text: string = "é🦀";
    let values: number[] = [n, input];
    let empty: number[] = [];
    let expression: number = -(n + input) / 2;
    let forward: number = later(expression);
    let hosted: number = host_number(forward);
    let length: number = len(values);
    let indexed: number = values[0];
    let alias: number[] = values;
    let copy: bool = flag;
    if copy { let n: number = n + 1; alias[1] = n; }
    while indexed < length { indexed = indexed + 1; }
    return hosted + indexed + len(empty);
}
fn boolean() -> bool { let value: bool = host_bool(); return value; }
fn text() -> string { let value: string = host_string(); return value; }
fn items() -> number[] { let value: number[] = host_array(); return value; }
fn later(value: number) -> number { return value * 2; }
"#;

fn inferred(source: &str) -> String {
    source
        .replace(": number[] =", " =")
        .replace(": number =", " =")
        .replace(": bool =", " =")
        .replace(": string =", " =")
}

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    for (name, params, result) in [
        ("host_number", vec![Type::Number], Type::Number),
        ("host_bool", vec![], Type::Bool),
        ("host_string", vec![], Type::String),
        ("host_array", vec![], Type::NumberArray),
    ] {
        compiler
            .register_host_function(name, "env", name, params, result)
            .unwrap();
    }
    compiler
}

#[test]
fn inference_preserves_raw_bytes_at_every_level_and_export_surface() {
    let inferred = inferred(ANNOTATED);
    let mut compiler = compiler();
    let program = compiler.analyze(&inferred).unwrap();
    let original = program.clone();

    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["items".into(), "main".into()]),
    ] {
        compiler.set_exports(exports.clone());
        compiler.set_optimization_level(OptimizationLevel::None);
        let expected = compiler.compile_source(ANNOTATED).unwrap();
        Validator::new().validate_all(&expected).unwrap();

        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile_source(&inferred).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(bytes, expected, "{level:?}, {exports:?}");
            assert_eq!(compiler.compile_source(ANNOTATED).unwrap(), expected);
            assert_eq!(compiler.compile(&program).unwrap(), expected);
            assert_eq!(program, original);
        }
    }
}

#[derive(Default)]
struct Decoded {
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    functions: Vec<u32>,
    locals: Vec<Vec<ValType>>,
    local_sets: Vec<Vec<u32>>,
    numbers: Vec<Vec<f64>>,
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
            Payload::FunctionSection(section) => {
                decoded
                    .functions
                    .extend(section.into_iter().map(Result::unwrap));
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    decoded
                        .exports
                        .push((export.name.into(), export.kind, export.index));
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut locals = Vec::new();
                for local in body.get_locals_reader().unwrap() {
                    let (count, ty) = local.unwrap();
                    locals.extend(std::iter::repeat_n(ty, usize::try_from(count).unwrap()));
                }
                let mut sets = Vec::new();
                let mut numbers = Vec::new();
                for operator in body.get_operators_reader().unwrap() {
                    match operator.unwrap() {
                        Operator::LocalSet { local_index } => sets.push(local_index),
                        Operator::F64Const { value } => numbers.push(f64::from_bits(value.bits())),
                        _ => {}
                    }
                }
                decoded.locals.push(locals);
                decoded.local_sets.push(sets);
                decoded.numbers.push(numbers);
            }
            _ => {}
        }
    }
    decoded
}

#[test]
fn inferred_locals_keep_f64_numbers_i32_handles_and_independent_slot_layout() {
    let source = r#"
        fn main(seed: number, gate: bool) -> number {
            let n = 10;
            let flag = true;
            let text = "é";
            let values = [n];
            let empty = [];
            return seed + n + len(values) + len(empty);
        }
        fn boolean() -> bool { let value = false; return value; }
        fn text() -> string { let value = "é"; return value; }
        fn items() -> number[] { let value = []; return value; }
    "#;
    let mut compiler = Compiler::new();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let decoded = decode(&compiler.compile_source(source).unwrap());
        for (index, (params, results)) in [
            (vec![ValType::F64, ValType::I32], vec![ValType::F64]),
            (vec![], vec![ValType::I32]),
            (vec![], vec![ValType::I32]),
            (vec![], vec![ValType::I32]),
        ]
        .into_iter()
        .enumerate()
        {
            let type_index = usize::try_from(decoded.functions[index]).unwrap();
            assert_eq!(decoded.signatures[type_index], (params, results));
        }
        // Two parameters precede five declarations and two fresh-array temporaries.
        assert_eq!(
            decoded.locals[0],
            [
                ValType::F64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32
            ]
        );
        assert_eq!(decoded.local_sets[0], [2, 3, 4, 7, 5, 8, 6]);
        assert_eq!(decoded.numbers[0], [10.0]);
        assert_eq!(decoded.locals[1], [ValType::I32]);
        assert_eq!(decoded.locals[2], [ValType::I32]);
        assert_eq!(decoded.locals[3], [ValType::I32, ValType::I32]);
        for (index, name) in ["main", "boolean", "text", "items"].into_iter().enumerate() {
            assert!(decoded.exports.contains(&(
                name.into(),
                ExternalKind::Func,
                u32::try_from(index).unwrap()
            )));
        }
        assert!(
            decoded
                .exports
                .contains(&("memory".into(), ExternalKind::Memory, 0))
        );
        assert!(
            decoded
                .exports
                .contains(&("__alloc".into(), ExternalKind::Func, 4))
        );
    }
}

#[test]
fn forged_inferred_declaration_types_still_fail_backend_checks_when_unexported_or_unreachable() {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    for body in [
        "let value = 10;",
        "return; let value = 10;",
        "if false { let value = 10; }",
    ] {
        let source = format!("fn hidden() -> void {{ {body} }} fn main() -> void {{}}");
        let mut program = compiler.analyze(&source).unwrap();
        let statement = program.functions[0].body.last_mut().unwrap();
        let (ty, span) = match &mut statement.kind {
            TypedStatementKind::Let { ty, .. } => (ty, statement.span),
            TypedStatementKind::If { then, .. } => {
                let span = then[0].span;
                let TypedStatementKind::Let { ty, .. } = &mut then[0].kind else {
                    panic!("expected nested declaration");
                };
                (ty, span)
            }
            _ => panic!("expected declaration or conditional"),
        };
        *ty = Type::Bool;

        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(
                compiler.compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "hidden".into(),
                    expected: Type::Bool,
                    found: Type::Number,
                    span,
                })
            );
        }
    }
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "was-compiler-inference-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create inference test directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_inferred_initializers_run_once_in_order_with_aliases_shadowing_and_control_flow() {
    let source = r#"
        fn main() -> number {
            let n = mark(1) + mark(2);
            let values = [mark(3), mark(4)];
            let alias = values;
            let fresh = [3, 4];
            fresh[0] = 99;
            let i = 0;
            while i < 2 {
                let n = n + mark(5 + i);
                alias[i] = n;
                i = i + 1;
            }
            if true {
                let n = n + mark(7);
                let text = "inner";
                alias[0] = n;
            } else {
                let n = mark(99);
                alias[0] = n;
            }
            return n * 100 + values[0] * 10 + values[1] + fresh[0];
        }
        fn fresh() -> number[] { let values = [1, 2]; return values; }
    "#;
    let scratch = Scratch::new();
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        for (const file of process.argv.slice(1)) {
            const events = [];
            const e = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(file)), {
                env: { mark(value) { events.push(value); return value; } },
            }).exports;
            for (let run = 0; run < 2; run++) {
                events.length = 0;
                assert.equal(e.main(), 508);
                assert.deepEqual(events, [1, 2, 3, 4, 5, 6, 7]);
            }
            const a = e.fresh(), b = e.fresh();
            assert.notEqual(a, b);
            const view = new DataView(e.memory.buffer);
            const dataA = view.getUint32(a, true), dataB = view.getUint32(b, true);
            assert.notEqual(dataA, dataB);
            assert.equal(view.getUint32(a + 4, true), 2);
            assert.equal(view.getUint32(b + 4, true), 2);
            view.setFloat64(dataA, 9, true);
            assert.equal(view.getFloat64(dataB, true), 1);
            assert.equal(view.getFloat64(dataB + 8, true), 2);
        }
    "#;
    let mut command = Command::new("node");
    command.arg("-e").arg(script);
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    for (index, level) in LEVELS.into_iter().enumerate() {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let path = scratch.0.join(format!("level-{index}.wasm"));
        fs::write(&path, bytes).unwrap();
        command.arg(path);
    }
    let output = test_process::output(&mut command, None, Duration::from_secs(30));
    assert!(
        output.status.success(),
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
