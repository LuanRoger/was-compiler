#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use was_compiler::{Compiler, OptimizationLevel, Type, diagnostic, tokens::Span};
use wasmparser::Validator;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];
const SOURCE: &str = r#"
fn main(x: number) { return 10 + (match mark(x) { 1 => 11, 1 => 22, 0.5..=2.5 => 33, _ => 44 }); }
fn text(s: string) { return match s { "é🦀" => "hit", _ => "miss" }; }
fn boolean(b: bool) { return match b { true => 1, false => 2 }; }
"#;
const ERRORS: &[(&str, &str)] = &[
    (
        "// é🦀\r\n\tfn f(n: number) { return match n { true => 1, _ => 2 }; }\r\n",
        "true",
    ),
    (
        "// é🦀\r\n\tfn f(b: bool) { return match b { true => 1, false => \"é\" }; }\r\n",
        "false => \"é\"",
    ),
    (
        "// é🦀\r\n\tfn f() { return match true { true => 1, false => missing }; }\r\n",
        "false => missing",
    ),
    (
        "// é🦀\r\n\tfn f(x: number = match true { _ => 1 }) {}\r\n",
        "match true { _ => 1 }",
    ),
    (
        "// é🦀\r\n\tfn f() requires match true { _ => true } {}\r\n",
        "match true { _ => true }",
    ),
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "match", "_", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "was-match-adapters-{}-{timestamp}-{serial}",
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
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn expected_diagnostic(path: &str, source: &str, origin: &str) -> String {
    let error = Compiler::new().compile_source(source).unwrap_err();
    let start = source.find(origin).unwrap();
    assert_eq!(
        error.span(),
        Some(Span::new(start, start + origin.len())),
        "{error}"
    );
    let rendered = diagnostic::render(path, source, &error);
    assert!(rendered.contains("semantic error:"), "{rendered}");
    assert!(rendered.contains("2 |     fn f"), "{rendered}");
    rendered
}

#[test]
fn cli_match_wasm_and_wat_equal_library_at_all_levels() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), SOURCE).unwrap();
    let mut native = compiler();
    let baseline = native.compile_source(SOURCE).unwrap();
    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        let bytes = native.compile_source(SOURCE).unwrap();
        assert_eq!(bytes, baseline);
        Validator::new().validate_all(&bytes).unwrap();
        for (emit, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
            let output = scratch.cli(&[
                "program.was",
                flag,
                "--emit",
                emit,
                "-o",
                destination,
                "--host",
                "mark=match._(number)->number",
            ]);
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
                assert_eq!(artifact, bytes);
            } else {
                assert_eq!(
                    String::from_utf8(artifact).unwrap(),
                    wasmprinter::print_bytes(&bytes).unwrap()
                );
            }
        }
    }
}

#[test]
fn cli_match_diagnostics_preserve_pattern_arm_utf8_origins_and_existing_output() {
    let scratch = Scratch::new();
    for (source, origin) in ERRORS {
        let expected = expected_diagnostic("bad.was", source, origin);
        for (flag, _) in LEVELS {
            fs::write(scratch.0.join("bad.was"), source).unwrap();
            fs::write(scratch.0.join("out.wasm"), b"keep existing output").unwrap();
            let output = scratch.cli(&["bad.was", flag, "-o", "out.wasm"]);
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!("{expected}\n")
            );
            assert_eq!(
                fs::read(scratch.0.join("out.wasm")).unwrap(),
                b"keep existing output"
            );
            let mut entries: Vec<_> = fs::read_dir(&scratch.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            entries.sort();
            assert_eq!(entries, ["bad.was", "out.wasm"]);
        }
    }
    fs::write(
        scratch.0.join("bad.was"),
        "fn main() { return match true { _ => 7 }; }",
    )
    .unwrap();
    assert!(scratch.cli(&["bad.was", "-o", "out.wasm"]).status.success());
    Validator::new()
        .validate_all(&fs::read(scratch.0.join("out.wasm")).unwrap())
        .unwrap();
}

#[test]
fn match_and_wildcard_are_reserved_only_as_source_names_in_native_and_cli() {
    let scratch = Scratch::new();
    let source = "fn main() { return _temp(); }";
    fs::write(scratch.0.join("program.was"), source).unwrap();
    let mut native = Compiler::new();
    for name in ["match", "_"] {
        let error = native
            .register_host_function(name, "env", "f", vec![], Type::Number)
            .unwrap_err();
        fs::write(scratch.0.join("out.wasm"), b"unchanged").unwrap();
        let host = format!("{name}=env.f()->number");
        let output = scratch.cli(&["program.was", "-o", "out.wasm", "--host", &host]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(&error.to_string()));
        assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), b"unchanged");
        for source in [
            format!("fn {name}() {{}}"),
            format!("fn f({name}: number) {{}}"),
            format!("fn f() {{ let {name} = 1; }}"),
        ] {
            let source = format!("// é🦀\r\n\t{source}\r\n");
            let error = Compiler::new().compile_source(&source).unwrap_err();
            fs::write(scratch.0.join("reserved.was"), &source).unwrap();
            for (flag, _) in LEVELS {
                let output = scratch.cli(&["reserved.was", flag, "-o", "out.wasm"]);
                assert_eq!(output.status.code(), Some(1));
                assert!(output.stdout.is_empty());
                assert_eq!(
                    String::from_utf8(output.stderr).unwrap(),
                    format!("{}\n", diagnostic::render("reserved.was", &source, &error))
                );
                assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), b"unchanged");
            }
        }
    }
    native
        .register_host_function("_temp", "match", "_", vec![], Type::Number)
        .unwrap();
    let output = scratch.cli(&[
        "program.was",
        "-o",
        "out.wasm",
        "--host",
        "_temp=match._()->number",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(scratch.0.join("out.wasm")).unwrap(),
        native.compile_source(source).unwrap()
    );
}
