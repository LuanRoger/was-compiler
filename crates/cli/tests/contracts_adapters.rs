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
use wasmparser::Validator;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];
const SOURCE: &str = r#"
fn main() { return target(c: mark(9), a: mark(3)) + target(2); }
fn target(a: number, b: number = a + 1, c: number = b * 2)
    requires a > 0 requires b == a + 1 requires c > b {
    mark(10); return a * 100 + b * 10 + c;
}
fn reject(n: number = 1) -> number requires false { mark(99); return n; }
fn count(xs: number[], n: number = len(xs)) requires n == len(xs) { return n; }
fn recursive(n: number) -> number requires n >= 0 {
    if n == 0 { return 0; } return n + recursive(n - 1);
}
"#;
const SELECTED: &[&str] = &["main", "target", "reject", "count", "recursive"];
#[derive(Clone, Copy)]
enum DiagnosticKind {
    TypeMismatch,
    UndefinedVariable,
    InvalidContract,
    NumberLiteralOutOfRange,
}

const ERRORS: [(&str, &str, &str, DiagnosticKind); 8] = [
    (
        "// é🦀\r\n\tfn f(x: number) requires x + 1 {}\r\n",
        "x + 1",
        "Type mismatch",
        DiagnosticKind::TypeMismatch,
    ),
    (
        "// é🦀\r\n\tfn f(x: number) requires missing > x {}\r\n",
        "missing > x",
        "Undefined variable 'missing'",
        DiagnosticKind::UndefinedVariable,
    ),
    (
        "// é🦀\r\n\tfn f() requires false && other() {}\r\nfn other() -> bool { return true; }",
        "false && other()",
        "only positional builtin len calls are allowed",
        DiagnosticKind::InvalidContract,
    ),
    (
        "// é🦀\r\n\tfn f(xs: number[]) requires true || xs[0] > 0 {}\r\n",
        "true || xs[0] > 0",
        "indexing is not allowed",
        DiagnosticKind::InvalidContract,
    ),
    (
        "// é🦀\r\n\tfn f(xs: number[]) requires len(xs: xs) > 0 {}\r\n",
        "len(xs: xs) > 0",
        "only positional builtin len calls are allowed",
        DiagnosticKind::InvalidContract,
    ),
    (
        "// é🦀\r\n\tfn f(x: number = missing_default) requires missing_contract {}\r\n",
        "missing_default",
        "Undefined variable 'missing_default'",
        DiagnosticKind::UndefinedVariable,
    ),
    (
        "\tfn f() /* é🦀 */ requires \"é🦀\" {}\r\n",
        "\"é🦀\"",
        "Type mismatch",
        DiagnosticKind::TypeMismatch,
    ),
    (
        "\tfn f() -> number /* é🦀 */ requires true { return 99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999; }\r\n",
        "return 99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999;",
        "outside the finite f64 range",
        DiagnosticKind::NumberLiteralOutOfRange,
    ),
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("idle", "requires", "requires", vec![], Type::Void)
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
            "was-contracts-adapters-{}-{timestamp}-{serial}",
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

fn expected_diagnostic(
    path: &str,
    source: &str,
    origin: &str,
    message: &str,
    kind: DiagnosticKind,
) -> String {
    let error = Compiler::new().compile_source(source).unwrap_err();
    use was_compiler::semantic::SemanticError;

    let stage = match (kind, &error) {
        (
            DiagnosticKind::TypeMismatch,
            CompileError::Semantic(SemanticError::TypeMismatch { .. }),
        )
        | (
            DiagnosticKind::UndefinedVariable,
            CompileError::Semantic(SemanticError::UndefinedVariable { .. }),
        )
        | (
            DiagnosticKind::InvalidContract,
            CompileError::Semantic(SemanticError::InvalidContractExpression { .. }),
        ) => "semantic",
        (
            DiagnosticKind::NumberLiteralOutOfRange,
            CompileError::Codegen(was_compiler::CodegenError::NumberLiteralOutOfRange { .. }),
        ) => "code generation",
        _ => panic!("unexpected diagnostic variant: {error}"),
    };
    assert!(error.to_string().contains(message), "{error}");
    let start = source.find(origin).unwrap();
    assert_eq!(error.span(), Some(Span::new(start, start + origin.len())));
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |i| start + i);
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
    let expected = format!(
        "{path}:{line}:{column}: {stage} error: {error}\n  |\n{line} | {excerpt}\n  | {}{}",
        " ".repeat(padding),
        "^".repeat(origin.chars().count())
    );
    assert_eq!(diagnostic::render(path, source, &error), expected);
    expected
}

#[test]
fn cli_contract_bytes_and_wat_match_native_at_every_level_and_export_selection() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), SOURCE).unwrap();
    let mut compiler = compiler();
    let baseline = compiler.compile_source(SOURCE).unwrap();
    for (flag, level) in LEVELS {
        compiler.set_optimization_level(level);
        compiler.set_exports(ExportOptions::Only(
            SELECTED.iter().map(|n| (*n).into()).collect(),
        ));
        assert_eq!(compiler.compile_source(SOURCE).unwrap(), baseline);
        for names in [SELECTED, &["main"][..], &[][..]] {
            compiler.set_exports(ExportOptions::Only(
                names.iter().map(|n| (*n).into()).collect(),
            ));
            let expected = compiler.compile_source(SOURCE).unwrap();
            Validator::new().validate_all(&expected).unwrap();
            for (emit, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
                let mut args = vec![
                    "program.was",
                    flag,
                    "--emit",
                    emit,
                    "-o",
                    destination,
                    "--host",
                    "idle=requires.requires()->void",
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
                let artifact = fs::read(scratch.0.join(destination)).unwrap();
                if emit == "wasm" {
                    assert_eq!(artifact, expected);
                } else {
                    assert_eq!(
                        String::from_utf8(artifact).unwrap(),
                        wasmprinter::print_bytes(&expected).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn cli_contract_diagnostics_keep_utf8_crlf_origins_destinations_and_recover() {
    let scratch = Scratch::new();
    for (source, origin, message, kind) in ERRORS {
        fs::write(scratch.0.join("program.was"), source).unwrap();
        let expected = expected_diagnostic("program.was", source, origin, message, kind);
        for (flag, level) in LEVELS {
            let mut native = Compiler::new();
            native.set_optimization_level(level);
            assert_eq!(
                diagnostic::render(
                    "program.was",
                    source,
                    &native.compile_source(source).unwrap_err()
                ),
                expected
            );
            for (emit, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
                fs::write(scratch.0.join(destination), b"existing artifact").unwrap();
                let before = scratch.entries();
                let output = scratch.cli(&["program.was", flag, "--emit", emit, "-o", destination]);
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
            let valid = "fn main(n: number = 1) requires n > 0 { return n; }";
            fs::write(scratch.0.join("recovered.was"), valid).unwrap();
            let output = scratch.cli(&["recovered.was", flag, "-o", "recovered.wasm"]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            assert!(output.stderr.is_empty());
            assert_eq!(
                fs::read(scratch.0.join("recovered.wasm")).unwrap(),
                native.compile_source(valid).unwrap()
            );
        }
    }
}

#[test]
fn requires_is_reserved_only_for_host_language_names_in_cli_and_native() {
    let scratch = Scratch::new();
    let source = "fn main() requires true { return 1; }";
    fs::write(scratch.0.join("program.was"), source).unwrap();
    fs::write(scratch.0.join("program.wasm"), b"existing artifact").unwrap();
    let mut native = Compiler::new();
    let error = native
        .register_host_function("requires", "env", "f", vec![], Type::Void)
        .unwrap_err();
    let output = scratch.cli(&[
        "program.was",
        "-o",
        "program.wasm",
        "--host",
        "requires=env.f()->void",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&error.to_string()));
    assert_eq!(
        fs::read(scratch.0.join("program.wasm")).unwrap(),
        b"existing artifact"
    );
    native
        .register_host_function("idle", "requires", "requires", vec![], Type::Void)
        .unwrap();
    let output = scratch.cli(&[
        "program.was",
        "-o",
        "program.wasm",
        "--host",
        "idle=requires.requires()->void",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(scratch.0.join("program.wasm")).unwrap(),
        native.compile_source(source).unwrap()
    );
}
