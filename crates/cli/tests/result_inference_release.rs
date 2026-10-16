#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type, codegen,
    diagnostic, parser::ParseError, semantic::SemanticError, tokens::Span,
};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef, ValType, Validator};

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "was-inference-release-{}-{timestamp}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn cli(&self, args: &[&str]) -> Output {
        test_process::output(
            Command::new(env!("CARGO_BIN_EXE_was-compiler"))
                .current_dir(&self.0)
                .args(args),
            None,
            Duration::from_secs(15),
        )
    }

    fn entries(&self) -> Vec<PathBuf> {
        let mut entries: Vec<_> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| PathBuf::from(entry.unwrap().file_name()))
            .collect();
        entries.sort();
        entries
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn span(source: &str, origin: &str) -> Span {
    let start = source.find(origin).unwrap();
    Span::new(start, start + origin.len())
}

// The CLI must preserve the core error, not merely fail on the same input.
fn assert_rejection(source: &str, expected: CompileError) {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), source).unwrap();
    for (flag, level) in LEVELS {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        assert_eq!(compiler.analyze(source).unwrap_err(), expected, "{flag}");
        assert_eq!(
            compiler.compile_source(source).unwrap_err(),
            expected,
            "{flag}"
        );
        let rendered = diagnostic::render("program.was", source, &expected);
        let stage = match expected {
            CompileError::Parse(_) => "parse error:",
            CompileError::Semantic(_) => "semantic error:",
            CompileError::Codegen(_) => panic!("analysis rejection must precede lowering"),
        };
        assert!(rendered.contains(stage), "{rendered}");
        for (format, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
            fs::write(scratch.0.join(destination), b"previous artifact").unwrap();
            let before = scratch.entries();
            let output = scratch.cli(&["program.was", flag, "--emit", format, "-o", destination]);
            assert_eq!(output.status.code(), Some(1), "{source}");
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!("{rendered}\n")
            );
            assert_eq!(
                fs::read(scratch.0.join(destination)).unwrap(),
                b"previous artifact"
            );
            assert_eq!(scratch.entries(), before);
        }
        // A failed analysis must not poison the adapter's subsequent use.
        Validator::new()
            .validate_all(
                &compiler
                    .compile_source("fn recovered() { return 7; }")
                    .unwrap(),
            )
            .unwrap();
    }
}

#[test]
fn cli_and_core_reject_hidden_recursive_components_before_body_errors() {
    for (body, bridge) in [
        ("return 1; bridge();", "return hidden();"),
        ("return false && bridge();", "return hidden();"),
        ("return bridge();", "return true; hidden();"),
    ] {
        let function = format!("fn hidden() {{ {body} }}");
        let source = format!(
            "// é🦀\r\nfn bad() {{ return missing; }}\r\n\t{function}\r\nfn bridge() -> bool {{ {bridge} }}\r\nfn later() {{ return later(); }}"
        );
        assert_rejection(
            &source,
            SemanticError::RecursiveResultAnnotationRequired {
                name: "hidden".into(),
                span: span(&source, &function),
            }
            .into(),
        );
    }
}

#[test]
fn cli_and_core_keep_lexical_shadowing_out_of_the_recursion_graph() {
    for (function, origin) in [
        (
            "fn shadow(shadow: number) { return shadow(); }",
            "return shadow();",
        ),
        (
            "fn shadow() { let shadow = 1; return shadow(); }",
            "return shadow();",
        ),
        (
            "fn shadow() { for shadow in [] { shadow(); } return 1; }",
            "shadow();",
        ),
        (
            "fn shadow() { return { let shadow = 1; shadow() }; }",
            "{ let shadow = 1; shadow() }",
        ),
    ] {
        let source = format!("// é🦀\r\n\t{function}\r\n");
        assert_rejection(
            &source,
            SemanticError::NotCallable {
                name: "shadow".into(),
                span: span(&source, origin),
            }
            .into(),
        );
    }
}

#[test]
fn cli_and_core_keep_incompatible_incomplete_and_never_only_result_origins() {
    for (body, expected, found) in [
        ("if b { return true; } return 1;", Type::Bool, Type::Number),
        ("if b { return; } return 1;", Type::Void, Type::Number),
        ("if b { return 1; } return;", Type::Number, Type::Void),
        (
            "if b { return \"é\"; } return [];",
            Type::String,
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
    ] {
        let function = format!("fn choose(b: bool = true) {{ {body} }}");
        let source = format!("// é🦀\r\n\t{function}\r\n");
        assert_rejection(
            &source,
            SemanticError::IncompatibleInferredReturns {
                expected,
                found,
                span: span(&source, &function),
            }
            .into(),
        );
    }
    for body in ["if b { return 1; }", "for x in [1] { return x; }"] {
        let function = format!("fn incomplete(b: bool = true) {{ {body} }}");
        let source = format!("// é🦀\r\n\t{function}\r\n");
        assert_rejection(
            &source,
            SemanticError::MissingReturn {
                name: "incomplete".into(),
                span: span(&source, &function),
            }
            .into(),
        );
    }
    for body in ["loop {} return 1;", "return; return 1;"] {
        let function = format!("fn spin() {{ {body} }}");
        let source = format!("// é🦀\r\n\t{function}\r\n");
        assert_rejection(
            &source,
            SemanticError::CannotInferFunctionResult {
                name: "spin".into(),
                span: span(&source, &function),
            }
            .into(),
        );
    }
    let source = "// é🦀\r\n\tfn tail() { leaf(n: 3) }\r\nfn leaf(n: number = 1) { return n; }";
    let start = source.find(" }\r\n").unwrap() + 1;
    assert_rejection(
        source,
        ParseError::UnexpectedToken {
            expected: ";",
            found: "}",
            span: Span::new(start, start + 1),
        }
        .into(),
    );
}

#[test]
fn cli_and_core_progress_through_default_graph_dependency_and_final_check_errors() {
    let mut source = String::from(
        "// é🦀\r\nfn final_body() -> number { return false; }\r\nfn root() { return leaf(n: true); }\r\nfn leaf(n: number = 1) { return n; }\r\nfn recur() { return recur(); }\r\n\tfn unused(n: number = missing_default) { return n; }",
    );
    assert_rejection(
        &source,
        SemanticError::UndefinedVariable {
            name: "missing_default".into(),
            span: span(&source, "missing_default"),
        }
        .into(),
    );
    source = source.replace("missing_default", "2");
    assert_rejection(
        &source,
        SemanticError::RecursiveResultAnnotationRequired {
            name: "recur".into(),
            span: span(&source, "fn recur() { return recur(); }"),
        }
        .into(),
    );
    source = source.replace("fn recur() {", "fn recur() -> number {");
    assert_rejection(
        &source,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: span(&source, "n: true"),
        }
        .into(),
    );
    source = source.replace("n: true", "n: 4");
    assert_rejection(
        &source,
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span: span(&source, "return false;"),
        }
        .into(),
    );

    let overflow = "9".repeat(400);
    for supplied in ["", "fn supplied() { return unused(n: 4); }"] {
        let source = format!(
            "// é🦀\r\nfn root() {{ return leaf(); }} fn leaf() {{ return 1; }} fn recur() {{ return recur(); }}\r\n\tfn unused(n: number = {overflow}) {{ return n; }} {supplied}"
        );
        assert_rejection(
            &source,
            SemanticError::DefaultNumberLiteralOutOfRange {
                literal: overflow.clone(),
                span: span(&source, &overflow),
            }
            .into(),
        );
        let source = source.replace(&overflow, "leaf()");
        assert_rejection(
            &source,
            SemanticError::InvalidDefaultExpression {
                reason: "only positional builtin len calls are allowed",
                span: {
                    let start = source.rfind("leaf()").unwrap();
                    Span::new(start, start + "leaf()".len())
                },
            }
            .into(),
        );
    }
}

#[test]
fn cli_compile_and_direct_codegen_keep_hidden_literal_errors_at_lowering() {
    let overflow = "9".repeat(400);
    for body in [
        format!("return 1; return {overflow};"),
        format!("return false && {overflow} == 0;"),
        format!("return leaf(n: {overflow});"),
    ] {
        let source = format!(
            "// é🦀\r\nfn main() {{ return 7; }}\r\n\tfn hidden() {{ {body} }}\r\nfn leaf(n: number = 1) {{ return n; }}"
        );
        let origin = if body.contains("n:") {
            format!("n: {overflow}")
        } else if body.starts_with("return 1;") {
            format!("return {overflow};")
        } else {
            body.clone()
        };
        let expected = CodegenError::NumberLiteralOutOfRange {
            function: "hidden".into(),
            literal: overflow.clone(),
            span: span(&source, &origin),
        };
        let scratch = Scratch::new();
        fs::write(scratch.0.join("program.was"), &source).unwrap();
        for (flag, level) in LEVELS {
            let mut compiler = Compiler::new();
            compiler.set_optimization_level(level);
            let program = compiler.analyze(&source).unwrap();
            assert_eq!(codegen::compile(&program).unwrap_err(), expected);
            for exports in [
                ExportOptions::All,
                ExportOptions::Only(vec!["main".into()]),
                ExportOptions::None,
            ] {
                compiler.set_exports(exports.clone());
                assert_eq!(compiler.compile(&program).unwrap_err(), expected);
                let wrapped = CompileError::Codegen(compiler.compile(&program).unwrap_err());
                assert_eq!(compiler.compile_source(&source).unwrap_err(), wrapped);
                let mut args = vec!["program.was", flag, "-o", "program.wasm"];
                match exports {
                    ExportOptions::All => {}
                    ExportOptions::Only(_) => args.extend(["--export", "main"]),
                    ExportOptions::None => args.push("--no-exports"),
                }
                fs::write(scratch.0.join("program.wasm"), b"previous artifact").unwrap();
                let before = scratch.entries();
                let output = scratch.cli(&args);
                assert_eq!(output.status.code(), Some(1));
                assert!(output.stdout.is_empty());
                assert_eq!(
                    String::from_utf8(output.stderr).unwrap(),
                    format!("{}\n", diagnostic::render("program.was", &source, &wrapped))
                );
                assert_eq!(
                    fs::read(scratch.0.join("program.wasm")).unwrap(),
                    b"previous artifact"
                );
                assert_eq!(scratch.entries(), before);
            }
        }
    }
}

const ABI_SOURCE: &str = r#"
fn main() { return left() + right(); }
fn left() { return number_leaf(c: mark(9), a: mark(3)); }
fn right() { return number_leaf(2); }
fn number_leaf(a: number, b: number = a + 1, c: number = b * 2) { return a * 100 + b * 10 + c; }
fn flag(x: bool, copy: bool = x) { return flag_leaf(x: copy); }
fn text(x: string, copy: string = x) { return text_leaf(x: copy); }
fn array(x: number[], copy: number[] = x) { return array_leaf(x: copy); }
fn flag_leaf(x: bool, copy: bool = x) { return copy; }
fn text_leaf(x: string, copy: string = x) { return copy; }
fn array_leaf(x: number[], copy: number[] = x) { copy[0] += 1; return copy; }
fn host_flag(x: bool) { let result = flag(x: raw()); return observe(x) && result; }
fn private() { return 0..=4 by 2; }
"#;
const HOSTS: &[&str] = &[
    "idle=app.unused()->void",
    "mark=env.mark(number)->number",
    "raw=env.raw()->bool",
    "observe=env.observe(bool)->bool",
];
const SELECTED: &[&str] = &["main", "number_leaf", "flag", "text", "array", "host_flag"];

fn abi_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    for (name, module, field, params, result) in [
        ("idle", "app", "unused", vec![], Type::Void),
        ("mark", "env", "mark", vec![Type::Number], Type::Number),
        ("raw", "env", "raw", vec![], Type::Bool),
        ("observe", "env", "observe", vec![Type::Bool], Type::Bool),
    ] {
        compiler
            .register_host_function(name, module, field, params, result)
            .unwrap();
    }
    compiler.set_exports(ExportOptions::Only(
        SELECTED.iter().map(|name| (*name).into()).collect(),
    ));
    compiler
}

fn abi_args(flag: &str) -> Vec<&str> {
    let mut args = vec!["program.was", flag, "-o", "program.wasm"];
    for host in HOSTS {
        args.extend(["--host", host]);
    }
    for name in SELECTED {
        args.extend(["--export", name]);
    }
    args
}

#[test]
fn cli_descriptor_chains_keep_source_order_full_arity_snapshot_and_private_exports() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), ABI_SOURCE).unwrap();
    let mut compiler = abi_compiler();
    let program = compiler.analyze(ABI_SOURCE).unwrap();
    let baseline = compiler.compile(&program).unwrap();
    let mut signatures = Vec::new();
    let mut functions = Vec::new();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    for payload in Parser::new(0).parse_all(&baseline) {
        match payload.unwrap() {
            Payload::TypeSection(reader) => {
                signatures.extend(reader.into_iter_err_on_gc_types().map(Result::unwrap))
            }
            Payload::FunctionSection(reader) => {
                functions.extend(reader.into_iter().map(Result::unwrap))
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.unwrap();
                    assert!(matches!(import.ty, TypeRef::Func(_)));
                    imports.push((import.module.to_owned(), import.name.to_owned()));
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.unwrap();
                    exports.push((export.name.to_owned(), export.kind, export.index));
                }
            }
            _ => {}
        }
    }
    assert_eq!(
        imports,
        [
            ("app".into(), "unused".into()),
            ("env".into(), "mark".into()),
            ("env".into(), "raw".into()),
            ("env".into(), "observe".into())
        ]
    );
    assert_eq!(
        exports,
        [
            ("main".into(), ExternalKind::Func, 4),
            ("number_leaf".into(), ExternalKind::Func, 7),
            ("flag".into(), ExternalKind::Func, 8),
            ("text".into(), ExternalKind::Func, 9),
            ("array".into(), ExternalKind::Func, 10),
            ("host_flag".into(), ExternalKind::Func, 14),
            ("memory".into(), ExternalKind::Memory, 0),
            ("__alloc".into(), ExternalKind::Func, 16),
        ]
    );
    for (index, params, result) in [
        (3, vec![ValType::F64; 3], ValType::F64),
        (4, vec![ValType::I32; 2], ValType::I32),
        (5, vec![ValType::I32; 2], ValType::I32),
        (6, vec![ValType::I32; 2], ValType::I32),
    ] {
        let signature = &signatures[usize::try_from(functions[index]).unwrap()];
        assert_eq!(signature.params(), params);
        assert_eq!(signature.results(), [result]);
    }
    compiler
        .register_host_function("later", "env", "later", vec![], Type::Void)
        .unwrap();
    for (flag, level) in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), baseline);
        assert_ne!(compiler.compile_source(ABI_SOURCE).unwrap(), baseline);
        let output = scratch.cli(&abi_args(flag));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        let bytes = fs::read(scratch.0.join("program.wasm")).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(bytes, baseline);
        let expected = CodegenError::RangeExport {
            function: "private".into(),
        };
        assert_eq!(codegen::compile(&program).unwrap_err(), expected);
        compiler.set_exports(ExportOptions::All);
        assert_eq!(compiler.compile(&program).unwrap_err(), expected);
        let mut args = vec!["program.was", flag, "-o", "program.wasm"];
        for host in HOSTS {
            args.extend(["--host", host]);
        }
        let output = scratch.cli(&args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let wrapped = CompileError::Codegen(expected);
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "{}\nSelect only compatible functions with --export NAME, or use --no-exports to keep all user functions internal.\n",
                diagnostic::render("program.was", ABI_SOURCE, &wrapped)
            )
        );
        assert_eq!(fs::read(scratch.0.join("program.wasm")).unwrap(), baseline);
        compiler.set_exports(ExportOptions::None);
        Validator::new()
            .validate_all(&compiler.compile(&program).unwrap())
            .unwrap();
        args.push("--no-exports");
        let output = scratch.cli(&args);
        assert!(output.status.success());
        assert_eq!(
            fs::read(scratch.0.join("program.wasm")).unwrap(),
            codegen::compile_with_exports(&program, &ExportOptions::None).unwrap()
        );
        compiler.set_exports(ExportOptions::Only(
            SELECTED.iter().map(|name| (*name).into()).collect(),
        ));
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_executes_cli_inferred_default_chains_with_raw_descriptor_and_boolean_host_abi() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), ABI_SOURCE).unwrap();
    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync('program.wasm');
        const module = new WebAssembly.Module(bytes);
        assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
            [['app', 'unused'], ['env', 'mark'], ['env', 'raw'], ['env', 'observe']]);
        assert.throws(() => new WebAssembly.Instance(module, {env: {}}), TypeError);
        const events = [], observed = [];
        let raw = -9;
        const e = new WebAssembly.Instance(module, {
            app: {unused() { throw Error('unused import executed'); }},
            env: {
                mark(n) { events.push(n); return n; },
                raw() { return raw; },
                observe(b) { observed.push(b); return b ? 23 : 0; },
            },
        }).exports;
        assert.equal(e.main(), 585);
        assert.deepEqual(events, [9, 3]);
        assert.equal(e.number_leaf.length, 3);
        assert.equal(e.number_leaf(3, 4, 9), 349);
        assert.ok(Number.isNaN(e.number_leaf(3)));
        for (const name of ['flag', 'text', 'array']) assert.equal(e[name].length, 2);
        assert.equal(e.flag(1, 0), 0);
        assert.equal(e.flag(0, 1), 1);
        assert.equal(e.flag(1), 0); // Raw exports do not supply source defaults.
        for (const x of [0, -7, 42]) assert.equal(e.host_flag(x), x === 0 ? 0 : 1);
        assert.deepEqual(observed, [0, 1, 1]);
        raw = 0;
        assert.equal(e.host_flag(42), 0);
        assert.deepEqual(observed, [0, 1, 1, 1]);
        function descriptor(payload, length) {
            const h = e.__alloc(8);
            const view = new DataView(e.memory.buffer);
            view.setUint32(h, payload, true);
            view.setUint32(h + 4, length, true);
            return h;
        }
        const utf8 = new TextEncoder().encode('é🦀');
        const p = e.__alloc(utf8.length);
        new Uint8Array(e.memory.buffer, p, utf8.length).set(utf8);
        const text = descriptor(p, utf8.length);
        const other = descriptor(p, utf8.length);
        assert.equal(e.text(text, other), other);
        const returned = e.text(text, other);
        const view = new DataView(e.memory.buffer);
        assert.equal(view.getUint32(returned + 4, true), 6);
        assert.equal(new TextDecoder().decode(new Uint8Array(e.memory.buffer,
            view.getUint32(returned, true), 6)), 'é🦀');
        assert.throws(() => e.text(text), WebAssembly.RuntimeError);
        const payload = e.__alloc(16);
        const numbers = new DataView(e.memory.buffer);
        numbers.setFloat64(payload, 4, true);
        numbers.setFloat64(payload + 8, -0, true);
        const array = descriptor(payload, 2);
        assert.equal(e.array(array, array), array);
        assert.equal(new DataView(e.memory.buffer).getFloat64(payload, true), 5);
        assert.ok(Object.is(new DataView(e.memory.buffer).getFloat64(payload + 8, true), -0));
        assert.equal(e.array(array, array), array);
        assert.equal(new DataView(e.memory.buffer).getFloat64(payload, true), 6);
        assert.throws(() => e.array(array), WebAssembly.RuntimeError);
        assert.equal(e.private, undefined);
    "#;
    for (flag, _) in LEVELS {
        let output = scratch.cli(&abi_args(flag));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = test_process::output(
            Command::new("node")
                .current_dir(&scratch.0)
                .args(["-e", script]),
            None,
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
