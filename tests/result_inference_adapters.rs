#[path = "support/process.rs"]
mod test_process;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic,
    tokens::Span,
};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef, ValType, Validator};

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];
const SOURCE: &str = r#"
fn main() {
    let xs = [1, 2];
    return target(c: mark(9), a: mark(3)) + target(2) + target(1, b: 5) + count(xs);
}
fn target(a: number, b: number = a + 1, c: number = b * 2) -> number {
    return a * 100 + b * 10 + c;
}
fn count(xs: number[], n: number = len(xs)) -> number { return n; }
fn inferred(n: number = 7) { return n; }
fn flag(b: bool = true) { return b; }
fn text(s: string = "é🦀") { return s; }
fn ranges(r: range, copy: range = r) { return copy; }
fn make() { return 0..=4 by 2; }
"#;
const SELECTED: &[&str] = &["main", "target", "count", "inferred", "flag", "text"];

fn explicit_source() -> String {
    SOURCE
        .replace("fn main() {", "fn main() -> number {")
        .replace(
            "fn inferred(n: number = 7) {",
            "fn inferred(n: number = 7) -> number {",
        )
        .replace(
            "fn flag(b: bool = true) {",
            "fn flag(b: bool = true) -> bool {",
        )
        .replace(
            "fn text(s: string = \"é🦀\") {",
            "fn text(s: string = \"é🦀\") -> string {",
        )
        .replace(
            "fn ranges(r: range, copy: range = r) {",
            "fn ranges(r: range, copy: range = r) -> range {",
        )
        .replace("fn make() {", "fn make() -> range {")
}

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("idle", "app", "unused", vec![], Type::Void)
        .unwrap();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    compiler.set_exports(ExportOptions::Only(
        SELECTED.iter().map(|name| (*name).into()).collect(),
    ));
    compiler
}

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
            "was-inference-adapters-{}-{timestamp}-{serial}",
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

#[test]
fn cli_inferred_defaults_match_explicit_library_bytes_and_full_arity_at_every_level() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), SOURCE).unwrap();
    let mut compiler = compiler();
    let baseline = compiler.compile_source(SOURCE).unwrap();
    let mut signatures = Vec::new();
    let mut function_types = Vec::new();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    for payload in Parser::new(0).parse_all(&baseline) {
        match payload.unwrap() {
            Payload::TypeSection(reader) => {
                signatures.extend(reader.into_iter_err_on_gc_types().map(Result::unwrap))
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.unwrap();
                    let TypeRef::Func(index) = import.ty else {
                        panic!("function import")
                    };
                    imports.push((import.module.to_owned(), import.name.to_owned(), index));
                }
            }
            Payload::FunctionSection(reader) => {
                function_types.extend(reader.into_iter().map(Result::unwrap))
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
        imports
            .iter()
            .map(|(module, name, _)| (module.as_str(), name.as_str()))
            .collect::<Vec<_>>(),
        [("app", "unused"), ("env", "mark")]
    );
    assert_eq!(
        exports
            .iter()
            .filter(|(name, kind, _)| *kind == ExternalKind::Func && name != "__alloc")
            .map(|(name, _, index)| (name.as_str(), *index))
            .collect::<Vec<_>>(),
        [
            ("main", 2),
            ("target", 3),
            ("count", 4),
            ("inferred", 5),
            ("flag", 6),
            ("text", 7)
        ]
    );
    assert!(exports.contains(&("memory".into(), ExternalKind::Memory, 0)));
    for (index, params, result) in [
        (0, vec![], ValType::F64),
        (1, vec![ValType::F64; 3], ValType::F64),
        (2, vec![ValType::I32, ValType::F64], ValType::F64),
        (3, vec![ValType::F64], ValType::F64),
        (4, vec![ValType::I32], ValType::I32),
        (5, vec![ValType::I32], ValType::I32),
        (6, vec![ValType::I32; 2], ValType::I32),
        (7, vec![], ValType::I32),
    ] {
        let signature = &signatures[usize::try_from(function_types[index]).unwrap()];
        assert_eq!(signature.params(), params);
        assert_eq!(signature.results(), [result]);
    }
    for (flag, level) in LEVELS {
        compiler.set_optimization_level(level);
        for (selection, names) in [
            (
                ExportOptions::Only(SELECTED.iter().map(|name| (*name).into()).collect()),
                SELECTED,
            ),
            (ExportOptions::None, &[][..]),
        ] {
            compiler.set_exports(selection);
            let expected = compiler.compile_source(SOURCE).unwrap();
            Validator::new().validate_all(&expected).unwrap();
            assert_eq!(
                compiler.compile_source(&explicit_source()).unwrap(),
                expected
            );
            let mut args = vec![
                "program.was",
                flag,
                "-o",
                "program.wasm",
                "--host",
                "idle=app.unused()->void",
                "--host",
                "mark=env.mark(number)->number",
            ];
            if names.is_empty() {
                args.push("--no-exports");
            }
            for name in names {
                args.extend(["--export", name]);
            }
            let output = scratch.cli(&args);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            assert!(output.stderr.is_empty());
            assert_eq!(fs::read(scratch.0.join("program.wasm")).unwrap(), expected);
        }
    }
    assert_eq!(
        scratch.entries(),
        [PathBuf::from("program.was"), PathBuf::from("program.wasm")]
    );
}

#[test]
fn inferred_defaulted_analysis_retains_the_host_snapshot_and_private_range_export_policy() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let expected = compiler.compile(&program).unwrap();
    compiler
        .register_host_function("later", "env", "later", vec![], Type::Void)
        .unwrap();
    for (_, level) in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), expected);
        assert_ne!(compiler.compile_source(SOURCE).unwrap(), expected);
        compiler.set_exports(ExportOptions::All);
        assert!(
            matches!(compiler.compile(&program), Err(CodegenError::RangeExport { function, .. }) if function == "ranges")
        );
        compiler.set_exports(ExportOptions::Only(vec!["make".into()]));
        assert!(
            matches!(compiler.compile(&program), Err(CodegenError::RangeExport { function, .. }) if function == "make")
        );
        compiler.set_exports(ExportOptions::None);
        Validator::new()
            .validate_all(&compiler.compile(&program).unwrap())
            .unwrap();
        compiler.set_exports(ExportOptions::Only(
            SELECTED.iter().map(|name| (*name).into()).collect(),
        ));
    }
}

// Origins refer to written source, not instantiated call-site defaults.
const ERRORS: [(&str, &str, &str); 8] = [
    (
        "fn main() { return missing_body; }\r\n\t/* é🦀 */ fn hidden(n: number = missing_default) {}\r\n",
        "missing_default",
        "Undefined variable 'missing_default'",
    ),
    (
        "fn target(n: number = 1) -> number { return n; }\r\nfn main() {\r\n\t/* é🦀 */ return target(n: missing_call);\r\n}\r\n",
        "n: missing_call",
        "Undefined variable 'missing_call'",
    ),
    (
        "// é🦀\r\n\tfn mixed(b: bool = true) { if b { return 1; } return false; }\r\n",
        "fn mixed(b: bool = true) { if b { return 1; } return false; }",
        "Incompatible inferred returns",
    ),
    (
        "// é🦀\r\n\tfn recurse(n: number = 1) { return recurse(); }\r\n",
        "fn recurse(n: number = 1) { return recurse(); }",
        "Recursive function 'recurse'",
    ),
    (
        "// é🦀\r\n\tfn first(n: number = 1) { return second(n: n); }\r\nfn second(n: number = 1) { return n; }\r\n",
        "return second(n: n);",
        "callee 'second' has an unresolved result",
    ),
    (
        "// é🦀\r\n\tfn first(n: number = 1) { return second(n); }\r\nfn second(n: number = 1) -> number { return first(n); }\r\n",
        "fn first(n: number = 1) { return second(n); }",
        "Recursive function 'first'",
    ),
    (
        "// é🦀\r\n\tfn spin(n: number = 1) { return loop {}; }\r\n",
        "fn spin(n: number = 1) { return loop {}; }",
        "requires an explicit result annotation",
    ),
    (
        "fn caller() { return consume(target()); } fn target() { return 1; } fn consume(a: number, b: number = 0) -> number { return a; }",
        "target()",
        "callee 'target' has an unresolved result",
    ),
];

fn assert_origin(source: &str, origin: &str, message: &str, error: &CompileError) {
    let start = source.find(origin).unwrap();
    assert_eq!(error.span(), Some(Span::new(start, start + origin.len())));
    assert!(error.to_string().contains(message), "{error}");
    assert!(matches!(error, CompileError::Semantic(_)));
}

fn expected_diagnostic(path: &str, source: &str, origin: &str, error: &CompileError) -> String {
    let start = source.find(origin).unwrap();
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |index| start + index);
    let line = source[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let column = source[line_start..start].chars().count() + 1;
    let padding = source[line_start..start]
        .replace('\t', "    ")
        .chars()
        .count();
    let excerpt = source[line_start..line_end]
        .trim_end_matches('\r')
        .replace('\t', "    ");
    format!(
        "{path}:{line}:{column}: semantic error: {error}\n  |\n{line} | {excerpt}\n  | {}{}",
        " ".repeat(padding),
        "^".repeat(origin.chars().count())
    )
}

#[test]
fn cli_combined_diagnostics_keep_original_unicode_crlf_tab_origins_and_output_files() {
    let scratch = Scratch::new();
    for (source, origin, message) in ERRORS {
        fs::write(scratch.0.join("program.was"), source).unwrap();
        for (flag, level) in LEVELS {
            let mut compiler = Compiler::new();
            compiler.set_optimization_level(level);
            let error = compiler.compile_source(source).unwrap_err();
            assert_origin(source, origin, message, &error);
            let expected = expected_diagnostic("program.was", source, origin, &error);
            assert_eq!(diagnostic::render("program.was", source, &error), expected);
            for (format, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
                fs::write(scratch.0.join(destination), b"existing artifact").unwrap();
                let before = scratch.entries();
                let output =
                    scratch.cli(&["program.was", flag, "--emit", format, "-o", destination]);
                assert_eq!(output.status.code(), Some(1));
                assert!(output.stdout.is_empty());
                assert_eq!(
                    String::from_utf8(output.stderr).unwrap(),
                    format!("{expected}\n")
                );
                assert_eq!(
                    fs::read(scratch.0.join(destination)).unwrap(),
                    b"existing artifact"
                );
                assert_eq!(scratch.entries(), before);
            }
        }
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_inferred_defaulted_exports_remain_full_arity_and_calls_apply_defaults() {
    let mut compiler = compiler();
    for (_, level) in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let script = r#"
            const assert = require('node:assert/strict');
            const events = [];
            const bytes = require('node:fs').readFileSync(0);
            const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                app: { unused() { throw new Error('unused import executed'); } },
                env: { mark(n) { events.push(n); return n; } },
            }).exports;
            assert.equal(e.main(), 747);
            assert.deepEqual(events, [9, 3]);
            assert.equal(e.target(3, 4, 9), 349);
            assert.ok(Number.isNaN(e.target(3)));
            assert.equal(e.inferred(12), 12);
            assert.ok(Number.isNaN(e.inferred()));
            assert.equal(e.flag(0), 0);
            assert.equal(e.flag(1), 1);
        "#;
        let output = test_process::output(
            Command::new("node").args(["-e", script]),
            Some(&bytes),
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

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js ESM and pkg rebuilt by the integrator from current WP7 + released WP6"]
fn node_rebuilt_browser_inference_defaults_bytes_diagnostics_recovery_and_range_remedies() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let package = root.join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "integrator must rebuild {} before this test",
            package.join(name).display()
        );
    }
    // Length-prefixed fields avoid command-line quoting, fixture writes and serde.
    let mut input = Vec::new();
    field(&mut input, SOURCE.as_bytes());
    field(&mut input, explicit_source().as_bytes());
    let mut compiler = compiler();
    for (_, level) in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(compiler.compile_source(&explicit_source()).unwrap(), bytes);
        field(&mut input, &bytes);
    }
    for (source, origin, message) in ERRORS {
        let error = compiler.compile_source(source).unwrap_err();
        assert_origin(source, origin, message, &error);
        let expected = expected_diagnostic("<browser>", source, origin, &error);
        for (_, level) in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(
                diagnostic::render(
                    "<browser>",
                    source,
                    &compiler.compile_source(source).unwrap_err()
                ),
                expected
            );
        }
        field(&mut input, source.as_bytes());
        field(&mut input, expected.as_bytes());
    }
    for selection in [ExportOptions::All, ExportOptions::Only(vec!["make".into()])] {
        compiler.set_exports(selection);
        let error = compiler.compile_source(SOURCE).unwrap_err();
        assert!(matches!(
            &error,
            CompileError::Codegen(CodegenError::RangeExport { .. })
        ));
        let message = format!(
            "{}\nSelect only compatible functions with setExports([\"main\"]), or use exportNone() to keep all user functions internal.",
            diagnostic::render("<browser>", SOURCE, &error)
        );
        field(&mut input, message.as_bytes());
    }
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    field(&mut input, &compiler.compile_source(SOURCE).unwrap());
    compiler.set_exports(ExportOptions::None);
    field(&mut input, &compiler.compile_source(SOURCE).unwrap());

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
                const length = input.readUInt32LE(offset); offset += 4;
                assert.ok(offset + length <= input.length);
                const bytes = input.subarray(offset, offset + length); offset += length;
                return bytes;
            }
            const inferred = field().toString('utf8');
            const explicit = field().toString('utf8');
            const native = [field(), field(), field(), field()];
            const diagnostics = Array.from({length: 8}, () => ({source: field().toString('utf8'), message: field().toString('utf8')}));
            const rangeAll = field().toString('utf8');
            const rangeSelected = field().toString('utf8');
            const mainOnly = field();
            const none = field();
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const { default: init, Compiler } = await import(pathToFileURL(path.join(directory, 'was_compiler.js')).href);
            await init({ module_or_path: fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm')) });
            const compiler = new Compiler();
            try {
                compiler.registerHostFunction('idle', 'app', 'unused', [], 'void');
                compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'number');
                const selected = ['main', 'target', 'count', 'inferred', 'flag', 'text'];
                function same(source, expected) {
                    const bytes = compiler.compile(source);
                    assert.ok(bytes instanceof Uint8Array);
                    assert.ok(WebAssembly.validate(bytes));
                    assert.deepEqual(Buffer.from(bytes), expected);
                }
                function fails(source, message) {
                    assert.throws(() => compiler.compile(source), error => {
                        assert.ok(error instanceof Error);
                        assert.equal(error.name, 'Error');
                        assert.equal(error.message, message);
                        return true;
                    });
                }
                for (const level of [0, 1, 2, 3]) {
                    compiler.setOptimizationLevel(level);
                    compiler.setExports(selected);
                    same(inferred, native[level]); same(explicit, native[level]);
                    for (const diagnostic of diagnostics) {
                        fails(diagnostic.source, diagnostic.message);
                        same(inferred, native[level]); same(explicit, native[level]);
                    }
                    compiler.exportAll(); fails(inferred, rangeAll);
                    compiler.setExports(['main']); same(inferred, mainOnly); same(explicit, mainOnly);
                    compiler.setExports(['make']); fails(inferred, rangeSelected);
                    compiler.exportNone(); same(inferred, none); same(explicit, none);
                    compiler.setExports(selected); same(inferred, native[level]);
                }
            } finally { compiler.free(); }
        }
        main().catch(error => { console.error(error); process.exitCode = 1; });
    "#;
    let output = test_process::output(
        Command::new("node")
            .args(["-e", script])
            .arg(package)
            .current_dir(root),
        Some(&input),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "browser inference/default parity failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
