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
fn digits(a: number, b: number, c: number) -> number requires a > 0 {
    mark(90); return a * 100 + b * 10 + c;
}
fn identity(x: number) -> number { return x; }
fn progression(start: number, end: number) -> range { return start..=end by 2; }
fn total(xs: range) -> number { let n = 0; for x in xs { n += x; } return n; }
fn main() -> number { return mark(1) |> digits(mark(2), mark(3)) |> identity(); }
fn bare() -> number { return 5 |> identity; }
fn empty() -> number { return 5 |> identity(); }
fn ranges() -> number { return 1 |> progression(5) |> total; }
fn ordinary(a: number = 2, b: number = a + 1) -> number { return a * 10 + b; }
fn ordinary_calls() -> number { return ordinary() + ordinary(b: 4, a: 3); }
"#;
const SELECTED: &[&str] = &["main", "bare", "empty", "ranges", "ordinary_calls"];

// #25 deliberately tests full positional, value-producing stages only. Ordinary
// named/default calls remain supported; pipeline stages must not inherit them.
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
        "// é🦀\r\n\tfn f() { return 1 |> defaults(); }\r\nfn defaults(a: number, b: number = 2) -> number { return a + b; }",
        "defaults()",
    ),
    (
        "// é🦀\r\n\tfn f() { 1 |> discard(); }\r\nfn discard(a: number) {}",
        "discard()",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> discard; }\r\nfn discard(a: number) {}",
        "discard",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> defaults; }\r\nfn defaults(a: number, b: number = 2) -> number { return a + b; }",
        "defaults",
    ),
    (
        "// é🦀\r\n\tfn f() { return 1 |> discard |> identity; }\r\nfn discard(a: number) {}\r\nfn identity(a: number) -> number { return a; }",
        "discard",
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
];
const NAMED_STAGE_ERRORS: &[&str] = &[
    "fn f() { return 1 |> pair(b: 2); } fn pair(a: number, b: number) -> number { return a + b; }",
    "fn f() { return 1 |> pair(a: 2, b: 3); } fn pair(a: number, b: number) -> number { return a + b; }",
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
    let expected = format!(
        "{path}:{line}:{column}: semantic error: {error}\n  |\n{line} | {excerpt}\n  | {}{}",
        " ".repeat(padding),
        "^".repeat(origin.chars().count())
    );
    assert_eq!(diagnostic::render(path, source, &error), expected);
    expected
}

#[test]
fn cli_pipeline_wasm_and_wat_equal_native_at_every_level_and_export_selection() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), SOURCE).unwrap();
    let mut native = compiler();
    let baseline = native
        .compile_source(SOURCE)
        .expect("pipeline source compilation");
    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        native.set_exports(ExportOptions::Only(
            SELECTED.iter().map(|name| (*name).into()).collect(),
        ));
        assert_eq!(native.compile_source(SOURCE).unwrap(), baseline);
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
    for (source, origin) in ERRORS {
        let expected = expected_diagnostic("bad.was", source, origin);
        fs::write(scratch.0.join("bad.was"), source).unwrap();
        for (flag, _) in LEVELS {
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
fn named_stage_arguments_are_rejected_without_changing_ordinary_named_defaults() {
    let scratch = Scratch::new();
    let ordinary = "fn main() { return pair(b: 4, a: 3) + pair(2); } fn pair(a: number, b: number = a + 1) -> number { return a * 10 + b; }";
    let expected = Compiler::new().compile_source(ordinary).unwrap();
    for (flag, level) in LEVELS {
        for source in NAMED_STAGE_ERRORS {
            fs::write(scratch.0.join("program.was"), source).unwrap();
            let mut native = Compiler::new();
            native.set_optimization_level(level);
            let error = native.compile_source(source).unwrap_err();
            // An invalid-token error at |> is not evidence that named stage
            // arguments have been rejected by the implemented pipeline grammar.
            let start = error.span().expect("named stage diagnostic span").start();
            assert!(start > source.find("|>").unwrap() + 1, "{error:?}");
            fs::write(scratch.0.join("out.wasm"), b"unchanged").unwrap();
            let output = scratch.cli(&["program.was", flag, "-o", "out.wasm"]);
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!("{}\n", diagnostic::render("program.was", source, &error))
            );
            assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), b"unchanged");
        }
        fs::write(scratch.0.join("program.was"), ordinary).unwrap();
        let output = scratch.cli(&["program.was", flag, "-o", "out.wasm"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), expected);
    }
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
    let baseline = native.compile_source(source).unwrap();
    Validator::new().validate_all(&baseline).unwrap();
    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        assert_eq!(native.compile_source(source).unwrap(), baseline);
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
        assert_eq!(fs::read(scratch.0.join("out.wasm")).unwrap(), baseline);
    }
}

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js and pkg freshly rebuilt from the integrated pipeline source"]
fn rebuilt_browser_pipeline_bytes_wat_execution_and_diagnostics_match_native() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "rebuild pkg before running pipeline browser tests"
        );
    }
    let mut input = Vec::new();
    field(&mut input, SOURCE.as_bytes());
    field(&mut input, SELECTED.join(",").as_bytes());
    let mut native = compiler();
    for (_, level) in LEVELS {
        native.set_optimization_level(level);
        for names in [SELECTED, &["main"][..], &[][..]] {
            native.set_exports(ExportOptions::Only(
                names.iter().map(|name| (*name).into()).collect(),
            ));
            let bytes = native.compile_source(SOURCE).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            field(&mut input, &bytes);
            field(
                &mut input,
                wasmprinter::print_bytes(&bytes).unwrap().as_bytes(),
            );
        }
    }
    field(
        &mut input,
        u32::try_from(ERRORS.len() + NAMED_STAGE_ERRORS.len())
            .unwrap()
            .to_le_bytes()
            .as_slice(),
    );
    for (source, origin) in ERRORS {
        field(&mut input, source.as_bytes());
        field(
            &mut input,
            expected_diagnostic("<browser>", source, origin).as_bytes(),
        );
    }
    for source in NAMED_STAGE_ERRORS {
        let error = compiler().compile_source(source).unwrap_err();
        assert!(
            error.span().unwrap().start() > source.find("|>").unwrap() + 1,
            "{error:?}"
        );
        field(&mut input, source.as_bytes());
        field(
            &mut input,
            diagnostic::render("<browser>", source, &error).as_bytes(),
        );
    }
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        const path = require('node:path');
        const {pathToFileURL} = require('node:url');
        async function main() {
            const input = fs.readFileSync(0); let offset = 0;
            function field() {
                assert.ok(offset + 4 <= input.length);
                const length = input.readUInt32LE(offset); offset += 4;
                assert.ok(offset + length <= input.length);
                const bytes = input.subarray(offset, offset + length); offset += length; return bytes;
            }
            const source = field().toString('utf8'), selected = field().toString('utf8').split(',');
            const native = Array.from({length: 4}, () => Array.from({length: 3},
                () => ({bytes: field(), wat: field().toString('utf8')})));
            const count = field().readUInt32LE(0);
            const diagnostics = Array.from({length: count},
                () => ({source: field().toString('utf8'), message: field().toString('utf8')}));
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const {default: init, Compiler} = await import(pathToFileURL(path.join(directory, 'was_compiler.js')).href);
            await init({module_or_path: fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm'))});
            const compiler = new Compiler();
            try {
                compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'number');
                function same(expected) {
                    const bytes = compiler.compile(source);
                    assert.ok(WebAssembly.validate(bytes));
                    assert.deepEqual(Buffer.from(bytes), expected.bytes);
                    assert.equal(compiler.compileWat(source), expected.wat);
                    return bytes;
                }
                for (let level = 0; level < 4; level++) {
                    compiler.setOptimizationLevel(level);
                    for (const [index, names] of [selected, ['main'], []].entries()) {
                        compiler.setExports(names); same(native[level][index]);
                    }
                    compiler.setExports(selected);
                    for (const diagnostic of diagnostics) {
                        for (const method of ['compile', 'compileWat']) {
                            assert.throws(() => compiler[method](diagnostic.source), error => {
                                assert.ok(error instanceof Error); assert.equal(error.name, 'Error');
                                assert.equal(error.message, diagnostic.message); return true;
                            });
                            same(native[level][0]);
                        }
                    }
                    const events = [];
                    const e = new WebAssembly.Instance(new WebAssembly.Module(same(native[level][0])), {
                        env: {mark(n) {events.push(n); return n;}}
                    }).exports;
                    assert.equal(e.main(), 123); assert.deepEqual(events, [1,2,3,90]);
                    assert.equal(e.bare(), 5); assert.equal(e.empty(), 5);
                    assert.equal(e.ranges(), 9); assert.equal(e.ordinary_calls(), 57);
                    assert.ok(!('progression' in e)); assert.ok(!('total' in e));
                }
            } finally {compiler.free();}
        }
        main().catch(error => {console.error(error); process.exitCode = 1;});
    "#;
    let output = test_process::output(
        Command::new("node").args(["-e", script]).arg(package),
        Some(&input),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
