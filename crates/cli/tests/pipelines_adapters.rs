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
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic,
    semantic::SemanticError, tokens::Span,
};
use wasmparser::{Parser, Payload, ValType, Validator};

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];
const SOURCE: &str = r#"
fn digits(a: number, b: number, c: number) -> number requires a > 0 {
    mark(90); return a * 100 + b * 10 + c;
}
fn identity(x: number) -> number { return x; }
fn progression(start: number, end: number) -> range { return start..=end by 2; }
fn total(xs: range) -> number { let n = 0; for x in xs { n += x; } return n; }
fn main() -> number { return mark(1) |> digits(mark(2), mark(3)) |> identity(); }
fn bare() -> number { return 5 |> identity; }
fn empty() -> number { return 5 |> identity(); }
fn ranges() -> number { return 1 |> progression(end: 5) |> total; }
fn defaults(a: number = 9, b: number = a + 1, c: number = b + 1) -> number
    requires a > 0 requires b > a requires c > b {
    return a * 100 + b * 10 + c;
}
fn named() -> number { return mark(1) |> digits(c: mark(3), b: mark(2)) |> identity; }
fn mixed() -> number { return mark(1) |> digits(mark(2), c: mark(3)); }
fn default_bare() -> number { return 2 |> defaults; }
fn default_empty() -> number { return 2 |> defaults(); }
fn default_named() -> number { return 2 |> defaults(c: 8); }
fn default_mixed() -> number { return 2 |> defaults(4, c: 8); }
fn default_chain() { return 2 |> defaults |> identity(); }
fn array_default(xs: number[], n: number = len(xs)) -> number { return n; }
fn arrays() -> number { return [1, 2, 3] |> array_default; }
fn text_default(s: string, same: string = s) -> bool { return s == same; }
fn strings() -> bool { return "é🦀" |> text_default; }
fn ordinary(a: number = 2, b: number = a + 1) -> number { return a * 10 + b; }
fn ordinary_calls() -> number { return ordinary() + ordinary(b: 4, a: 3); }
fn sink(a: number, b: number = a + 1) -> void
    requires a > 0 requires b > a { mark(a); mark(b); }
fn sink_wrapper() { 4 |> sink; }
fn sink_trap_wrapper() { mark(1) |> sink(b: mark(0)); mark(99); }
fn discarded(flag: bool) -> number {
    mark(1) /* é🦀 */ |> identity |> /* λ */ sink(
        b: mark(2)
    );
    (3 |> sink());
    { /* é🦀: discarded tail */ (4 |> sink) };
    if flag { 6 |> sink } else { 8 |> sink() }
    (if flag { 10 |> sink } else { 12 |> sink });
    match flag { true => 14 |> sink, false => 16 |> sink(), }
    { if flag { { (18 |> sink) } } else { 20 |> sink } };
    return 65;
}
fn discarded_never() -> number { (loop {}) |> sink; }
"#;
const SELECTED: &[&str] = &[
    "main",
    "bare",
    "empty",
    "ranges",
    "ordinary_calls",
    "named",
    "mixed",
    "defaults",
    "default_bare",
    "default_empty",
    "default_named",
    "default_mixed",
    "default_chain",
    "arrays",
    "strings",
    "sink",
    "sink_wrapper",
    "sink_trap_wrapper",
    "discarded",
    "discarded_never",
];

const ERRORS: &[(&str, &str)] = &[
    ("// é🦀\r\n\tfn f() { return 1 |> mark; }\r\n", "mark"),
    ("// é🦀\r\n\tfn f() { return [1] |> len(); }\r\n", "len()"),
    (
        "// é🦀\r\n\tfn f() { return 1 |> zero(); }\r\nfn zero() -> number { return 0; }",
        "zero()",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair; }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "pair",
    ),
    (
        "// é🦀\r\n\tfn f() { return (\"é🦀\") |> numeric; }\r\nfn numeric(a: number) -> number { return a; }",
        "(\"é🦀\")",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(\"é🦀\"); }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "\"é🦀\"",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> text |> numeric; }\r\nfn text(a: number) -> string { return \"é\"; }\r\nfn numeric(a: number) -> number { return a; }",
        "1 |> text",
    ),
    (
        "// é🦀\r\n\tfn f() { return (\"é🦀\" /* λ */\r\n\t) |> numeric; }\r\nfn numeric(a: number) -> number { return a; }",
        "(\"é🦀\" /* λ */\r\n\t)",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 /* λ */ |> text(\r\n\t) /* 🦀 */ |> numeric; }\r\nfn text(a: number) -> string { return \"é\"; }\r\nfn numeric(a: number) -> number { return a; }",
        "1 /* λ */ |> text(\r\n\t)",
    ),
    (
        "// é🦀\r\n\tfn f() { 1 |> discard; }\r\nfn discard(a: number, b: number = /* λ */ \"é🦀\") {}",
        "\"é🦀\"",
    ),
];
const FINAL_VOID: &str = "a final void stage requires a discarded result";
const NONFINAL_VOID: &str = "a nonfinal stage cannot return void";
const VOID_ERRORS: &[(&str, &str, &str)] = &[
    (
        "fn f() { return 1 |> discard; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { let x = 1 |> discard(); } fn discard(a: number) {}",
        "discard()",
        FINAL_VOID,
    ),
    (
        "fn f() { consume(1 |> discard); } fn discard(a: number) {} fn consume(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { (1 |> discard) + 2; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { if (1 |> discard) {} } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { [1 |> discard]; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { let xs = [1]; xs[1 |> discard]; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { let x = { 1 |> discard }; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { let x = if true { 1 |> discard } else { 2 |> discard }; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { let x = match true { true => 1 |> discard, false => 2 |> discard, }; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() -> number { return (loop {}) |> discard; } fn discard(a: number) {}",
        "discard",
        FINAL_VOID,
    ),
    (
        "fn f() { 1 |> discard |> identity; } fn discard(a: number) {} fn identity(a: number) -> number { return a; }",
        "discard",
        NONFINAL_VOID,
    ),
    (
        "fn f() -> number { (loop {}) |> discard() |> identity; } fn discard(a: number) {} fn identity(a: number) -> number { return a; }",
        "discard()",
        NONFINAL_VOID,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 /* λ */ |> discard(\r\n\t    b: 2 /* 🦀 */\r\n\t); }\r\nfn discard(a: number, b: number = a + 1) {}",
        "discard(\r\n\t    b: 2 /* 🦀 */\r\n\t)",
        FINAL_VOID,
    ),
    (
        "// é🦀\r\n\tfn f() { 1 /* λ */ |> discard(\r\n\t    b: 2 /* 🦀 */\r\n\t) |> identity; }\r\nfn discard(a: number, b: number = a + 1) {}\r\nfn identity(a: number) -> number { return a; }",
        "discard(\r\n\t    b: 2 /* 🦀 */\r\n\t)",
        NONFINAL_VOID,
    ),
];

#[derive(Clone, Copy, Debug)]
enum BindingError {
    Duplicate(&'static str),
    Unknown,
    Missing,
    PositionalAfterNamed,
    TooManyPositional,
    TypeMismatch,
}

const BINDING_ERRORS: &[(&str, &str, BindingError)] = &[
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(a: missing, b: 3); }\r\nfn pair(a: number = 9, b: number = a + 1) -> number { return a + b; }",
        "a: missing",
        BindingError::Duplicate("a"),
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(2, b: missing); }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "b: missing",
        BindingError::Duplicate("b"),
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(b: 2, b: missing); }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "b: missing",
        BindingError::Duplicate("b"),
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(unknown: missing); }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "unknown: missing",
        BindingError::Unknown,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(); }\r\nfn pair(a: number, b: number, c: number = b + 1) -> number { return a + b + c; }",
        "pair()",
        BindingError::Missing,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair; }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "pair",
        BindingError::Missing,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(c: missing); }\r\nfn pair(a: number, b: number, c: number = b + 1) -> number { return a + b + c; }",
        "pair(c: missing)",
        BindingError::Missing,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(b: missing, 3); }\r\nfn pair(a: number, b: number, c: number) -> number { return a + b + c; }",
        "3",
        BindingError::PositionalAfterNamed,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(2, missing); }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "missing",
        BindingError::TooManyPositional,
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> pair(b: \"é🦀\"); }\r\nfn pair(a: number, b: number) -> number { return a + b; }",
        "b: \"é🦀\"",
        BindingError::TypeMismatch,
    ),
    (
        "// é🦀\r\n\tfn f() -> number { return (loop {}) |> pair(a: 2); }\r\nfn pair(a: number, b: number = a + 1) -> number { return a + b; }",
        "a: 2",
        BindingError::Duplicate("a"),
    ),
    (
        "// é🦀\r\n\tfn f() { 1 |> pair(a: missing); }\r\nfn pair(a: number, b: number = a + 1) {}",
        "a: missing",
        BindingError::Duplicate("a"),
    ),
    (
        "// é🦀\r\n\tfn f() { 1 |> pair(b: \"é🦀\"); }\r\nfn pair(a: number, b: number = a + 1) {}",
        "b: \"é🦀\"",
        BindingError::TypeMismatch,
    ),
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
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
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "was-pipelines-adapters-{}-{timestamp}-{serial}",
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
    let error = compiler().compile_source(source).unwrap_err();
    assert!(matches!(error, CompileError::Semantic(_)), "{error:?}");
    let start = source.find(origin).unwrap();
    assert_eq!(
        error.span(),
        Some(Span::new(start, start + origin.len())),
        "{error}"
    );
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
    let visible_end = (start + origin.len()).min(line_end);
    let width = source[start..visible_end]
        .trim_end_matches('\r')
        .replace('\t', "    ")
        .chars()
        .count()
        .max(1);
    let expected = format!(
        "{path}:{line}:{column}: semantic error: {error}\n  |\n{line} | {excerpt}\n  | {}{}",
        " ".repeat(padding),
        "^".repeat(width)
    );
    assert_eq!(diagnostic::render(path, source, &error), expected);
    expected
}

fn diagnostic_cases(path: &str) -> Vec<(&'static str, String)> {
    let mut cases: Vec<_> = ERRORS
        .iter()
        .map(|(source, origin)| (*source, expected_diagnostic(path, source, origin)))
        .collect();
    for (source, origin, message) in VOID_ERRORS {
        let error = compiler().compile_source(source).unwrap_err();
        assert!(
            matches!(&error, CompileError::Semantic(SemanticError::InvalidPipeline { message: actual, .. }) if actual == message),
            "expected {message:?}, got {error:?}"
        );
        let start = source.find(origin).unwrap();
        let span = Span::new(start, start + origin.len());
        assert_eq!(
            error.to_string(),
            format!("Invalid pipeline: {message} at {span:?}")
        );
        cases.push((*source, expected_diagnostic(path, source, origin)));
    }
    for (source, origin, kind) in BINDING_ERRORS {
        let error = compiler().compile_source(source).unwrap_err();
        let CompileError::Semantic(semantic) = &error else {
            panic!("expected {kind:?}, got {error:?}");
        };
        let matches = match (kind, semantic) {
            (
                BindingError::Duplicate(formal),
                SemanticError::DuplicateArgument { name, argument, .. },
            ) => name == "pair" && argument == formal,
            (BindingError::Unknown, SemanticError::UnknownArgument { name, argument, .. }) => {
                name == "pair" && argument == "unknown"
            }
            (BindingError::Missing, SemanticError::MissingArgument { name, argument, .. }) => {
                name == "pair" && argument == "b"
            }
            (
                BindingError::PositionalAfterNamed,
                SemanticError::PositionalAfterNamed { name, .. },
            )
            | (
                BindingError::TooManyPositional,
                SemanticError::TooManyPositionalArguments { name, .. },
            ) => name == "pair",
            (BindingError::TypeMismatch, SemanticError::TypeMismatch { .. }) => true,
            _ => false,
        };
        assert!(matches, "expected {kind:?}, got {error:?}");
        cases.push((*source, expected_diagnostic(path, source, origin)));
    }
    cases
}

fn assert_full_arity_default_export(bytes: &[u8]) {
    let mut signatures = Vec::new();
    let mut functions = Vec::new();
    let mut exported = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.unwrap() {
            Payload::TypeSection(reader) => {
                signatures.extend(reader.into_iter_err_on_gc_types().map(Result::unwrap));
            }
            Payload::FunctionSection(reader) => {
                functions.extend(reader.into_iter().map(Result::unwrap));
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.unwrap();
                    if matches!(export.name, "defaults" | "sink" | "sink_wrapper") {
                        exported.push((export.name, export.index));
                    }
                }
            }
            _ => {}
        }
    }
    // SOURCE registers exactly one imported function, before every user function.
    assert_eq!(exported.len(), 3);
    for (name, index) in exported {
        let index = usize::try_from(index).unwrap() - 1;
        let signature = &signatures[usize::try_from(functions[index]).unwrap()];
        match name {
            "defaults" => {
                assert_eq!(signature.params(), [ValType::F64; 3]);
                assert_eq!(signature.results(), [ValType::F64]);
            }
            "sink" => {
                assert_eq!(signature.params(), [ValType::F64; 2]);
                assert!(signature.results().is_empty());
            }
            "sink_wrapper" => {
                assert!(signature.params().is_empty());
                assert!(signature.results().is_empty());
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn cli_pipeline_wasm_and_wat_equal_native_at_every_level_and_export_selection() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), SOURCE).unwrap();
    let mut native = compiler();
    let baseline = native
        .compile_source(SOURCE)
        .expect("pipeline source compilation");
    assert_full_arity_default_export(&baseline);
    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        native.set_exports(ExportOptions::Only(
            SELECTED.iter().map(|name| (*name).into()).collect(),
        ));
        let selected_bytes = native.compile_source(SOURCE).unwrap();
        assert_full_arity_default_export(&selected_bytes);
        let program = native.analyze(SOURCE).unwrap();
        assert_eq!(selected_bytes, native.compile(&program).unwrap());
        for names in [SELECTED, &["main"][..], &[][..]] {
            native.set_exports(ExportOptions::Only(
                names.iter().map(|name| (*name).into()).collect(),
            ));
            let bytes = native.compile_source(SOURCE).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            for (emit, destination) in [("wasm", "out.wasm"), ("wat", "out.wat")] {
                let mut args = vec![
                    "program.was",
                    flag,
                    "--emit",
                    emit,
                    "-o",
                    destination,
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
}

#[test]
fn cli_pipeline_errors_preserve_utf8_stage_argument_prefix_spans_and_output() {
    let scratch = Scratch::new();
    for (source, expected) in diagnostic_cases("bad.was") {
        fs::write(scratch.0.join("bad.was"), source).unwrap();
        for (flag, level) in LEVELS {
            let mut native = compiler();
            native.set_optimization_level(level);
            let error = native.compile_source(source).unwrap_err();
            assert_eq!(diagnostic::render("bad.was", source, &error), expected);
            fs::write(scratch.0.join("out.wasm"), b"keep existing output").unwrap();
            let output = scratch.cli(&[
                "bad.was",
                flag,
                "-o",
                "out.wasm",
                "--host",
                "mark=env.mark(number)->number",
            ]);
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
    // Recover using the same adapter and destination after every rejected program.
    fs::write(
        scratch.0.join("bad.was"),
        "fn f() { return 1 |> identity; } fn identity(x: number) -> number { return x; }",
    )
    .unwrap();
    assert!(scratch.cli(&["bad.was", "-o", "out.wasm"]).status.success());
    Validator::new()
        .validate_all(&fs::read(scratch.0.join("out.wasm")).unwrap())
        .unwrap();
}

#[test]
fn ordinary_calls_keep_named_defaults_hosts_builtins_zero_arity_and_void_support() {
    let source = r#"
        fn main() -> number {
            discard(1);
            return mark(pair(b: 4, a: 3)) + pair(2) + len([7]) + zero();
        }
        fn pair(a: number, b: number = a + 1) -> number { return a * 10 + b; }
        fn zero() -> number { return 5; }
        fn discard(a: number) {}
    "#;
    let scratch = Scratch::new();
    fs::write(scratch.0.join("ordinary.was"), source).unwrap();
    let mut native = compiler();
    native.set_exports(ExportOptions::Only(vec!["main".into()]));
    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        let expected = native.compile_source(source).unwrap();
        Validator::new().validate_all(&expected).unwrap();
        let program = native.analyze(source).unwrap();
        assert_eq!(expected, native.compile(&program).unwrap());
        let output = scratch.cli(&[
            "ordinary.was",
            flag,
            "-o",
            "out.wasm",
            "--export",
            "main",
            "--host",
            "mark=env.mark(number)->number",
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), expected);
    }
}

const VOID_ASSERTIONS: &str = r#"
    function assertDiscardedVoid(assert, e, events) {
        for (const [flag, expected] of [
            [1, [1,2,1,2,3,4,4,5,6,7,10,11,14,15,18,19]],
            [0, [1,2,1,2,3,4,4,5,8,9,12,13,16,17,20,21]]
        ]) {
            events.length = 0;
            assert.equal(e.discarded(flag), 65);
            assert.deepEqual(events, expected);
        }
        events.length = 0;
        assert.equal(e.sink.length, 2);
        assert.equal(e.sink(4, 5), undefined);
        assert.equal(e.sink_wrapper.length, 0);
        assert.equal(e.sink_wrapper(), undefined);
        assert.deepEqual(events, [4,5,4,5]);
        events.length = 0;
        assert.throws(() => e.sink(4), WebAssembly.RuntimeError);
        assert.throws(() => e.sink(0, 1), WebAssembly.RuntimeError);
        assert.throws(() => e.sink(2, 1), WebAssembly.RuntimeError);
        assert.deepEqual(events, []);
        assert.throws(() => e.sink_trap_wrapper(), WebAssembly.RuntimeError);
        assert.deepEqual(events, [1,0]);
        // Validate the divergent export, but never run it without guest fuel.
        assert.equal(typeof e.discarded_never, 'function');
    }
"#;

#[test]
#[ignore = "requires Node.js for bounded CLI-produced pipeline execution"]
fn cli_discarded_void_pipelines_execute_defaults_contracts_and_full_arity_wrappers() {
    let scratch = Scratch::new();
    let source = SOURCE.replace('\n', "\r\n");
    fs::write(scratch.0.join("program.was"), &source).unwrap();
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        const events = [];
        const bytes = fs.readFileSync(process.argv[1]);
        assert.ok(WebAssembly.validate(bytes));
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
            env: {mark(n) {events.push(n); return n;}}
        }).exports;
        assertDiscardedVoid(assert, e, events);
    "#;
    let script = [VOID_ASSERTIONS, script].concat();
    for (flag, level) in LEVELS {
        let mut native = compiler();
        native.set_optimization_level(level);
        let bytes = native.compile_source(&source).unwrap();
        assert_eq!(bytes, native.compile_source(SOURCE).unwrap());
        Validator::new().validate_all(&bytes).unwrap();
        assert_full_arity_default_export(&bytes);
        let mut args = vec![
            "program.was",
            flag,
            "-o",
            "out.wasm",
            "--host",
            "mark=env.mark(number)->number",
        ];
        for name in SELECTED {
            args.extend(["--export", name]);
        }
        let output = scratch.cli(&args);
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), bytes);
        let output = test_process::output(
            Command::new("node")
                .args(["-e", &script])
                .arg(scratch.0.join("out.wasm")),
            None,
            Duration::from_secs(15),
        );
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
