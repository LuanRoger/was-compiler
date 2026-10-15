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
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic, tokens::Span,
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
    return left() + right() + target(1, b: 5) + count(xs);
}
fn target(a: number, b: number = a + 1, c: number = b * 2) {
    return a * 100 + b * 10 + c;
}
fn count(xs: number[], n: number = len(xs)) { return n; }
fn inferred(n: number = 7) { return n; }
fn flag(b: bool = true) { return b; }
fn text(s: string = "é🦀") { return s; }
fn ranges(r: range, copy: range = r) { return copy; }
fn make() { return 0..=4 by 2; }
fn left() { return target(c: mark(9), a: mark(3)); }
fn right() { return target(2); }
"#;
const SELECTED: &[&str] = &["main", "target", "count", "inferred", "flag", "text"];

fn explicit_source() -> String {
    SOURCE
        .replace("fn main() {", "fn main() -> number {")
        .replace(
            "fn target(a: number, b: number = a + 1, c: number = b * 2) {",
            "fn target(a: number, b: number = a + 1, c: number = b * 2) -> number {",
        )
        .replace(
            "fn count(xs: number[], n: number = len(xs)) {",
            "fn count(xs: number[], n: number = len(xs)) -> number {",
        )
        .replace("fn left() {", "fn left() -> number {")
        .replace("fn right() {", "fn right() -> number {")
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
        (8, vec![], ValType::F64),
        (9, vec![], ValType::F64),
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
        "// é🦀\r\n\tfn first(n: number = 1) { return consume(n: second(n: n)); }\r\nfn second(n: number = 1) { return true; }\r\nfn consume(n: number) -> number { return n; }\r\n",
        "n: second(n: n)",
        "Type mismatch",
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
        "fn caller() { return consume(target()); } fn target() { return true; } fn consume(a: number, b: number = 0) -> number { return a; }",
        "target()",
        "Type mismatch",
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
