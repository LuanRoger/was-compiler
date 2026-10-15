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
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic,
    semantic::SemanticError, tokens::Span,
};
use wasmparser::Validator;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];
const SOURCE: &str = r#"
fn named(a: number, b: number = a + 1) -> number requires a > 0 { return a + b; }
fn main() -> number {
    let first = ((x: number) => named(a: x));
    let wrapper = () => { let nested = () => first(mark(2)); nested() };
    let sink = () => { mark(3); return; };
    let state = 0;
    let increment = () => { state = state + 1; };
    let invoke = () => { let nested = () => increment(); nested() };
    invoke(); sink(); return wrapper() + state;
}
fn text() -> string {
    let original = "hé😀";
    let make = () => { let nested = () => original; nested() };
    return make();
}
fn array() -> number[] { let make = () => [1, 2]; return make(); }
fn internal_range() -> range { let make = () => 1..=5 by 2; return make(); }
fn operations() -> number {
    let work = () => {
        let total = 0;
        for x in internal_range() { total += x; }
        total % 4
    };
    return work();
}
"#;
const SCALAR: &str = "fn main() -> number { let f = () => 7; let g = () => { let h = () => f(); h() }; return g(); }";
const CAPTURED_COMPOUNDS: &str = r#"
fn main() -> number {
    let n = 20;
    let array = [20];
    let alias = array;
    let rhs = () => {
        mark(2); n = 100; alias[0] = 200; array = [80]; 3
    };
    n += rhs(); mark(n);
    let original = array;
    array[mark(0)] %= rhs(); mark(original[0]);
    let update = () => { n -= { n = 300; mark(4) }; };
    update();
    return n + alias[0] + original[0] + array[0];
}
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
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
            "was-closures-adapters-{}-{timestamp}-{serial}",
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

#[derive(Debug)]
enum ErrorKind {
    Invalid(&'static str),
    Inference,
    Arity,
    Named,
    MissingReturn,
    ReadOnly,
}

const ERRORS: &[(&str, &str, ErrorKind)] = &[
    (
        "for x in [1] { let f = () => { x = 2; }; }",
        "x = 2;",
        ErrorKind::ReadOnly,
    ),
    (
        "for x in 1..2 { let f = () => { return; x += 2; }; }",
        "x += 2;",
        ErrorKind::ReadOnly,
    ),
    (
        "let f: number = (() => missing);",
        "f",
        ErrorKind::Invalid("a local callable binding cannot have a value type annotation"),
    ),
    (
        "(() => missing);",
        "() => missing",
        ErrorKind::Invalid("a closure must initialize a local callable binding"),
    ),
    (
        "let f = () => 1; let alias = f;",
        "let alias = f;",
        ErrorKind::Invalid(
            "a local callable binding cannot be used as a value; invoke it directly or use a named function",
        ),
    ),
    (
        "let f = () => 1; f = 2;",
        "f = 2;",
        ErrorKind::Invalid("a local callable binding cannot be reassigned"),
    ),
    ("let f = (() => loop {});", "loop {}", ErrorKind::Inference),
    (
        "let f = (x: number) => x; f(1, 2);",
        "f(1, 2)",
        ErrorKind::Arity,
    ),
    (
        "let f = (x: number) => x; f(x: missing);",
        "x: missing",
        ErrorKind::Named,
    ),
    (
        "let f = () => { if true { return 1; } let x = 2; };",
        "{ if true { return 1; } let x = 2; }",
        ErrorKind::MissingReturn,
    ),
];

fn expected_diagnostic(source: &str, origin: &str, kind: &ErrorKind) -> String {
    let error = compiler().analyze(source).unwrap_err();
    let CompileError::Semantic(semantic) = &error else {
        panic!("expected {kind:?}, got {error:?}");
    };
    let matches = match (kind, semantic) {
        (ErrorKind::Invalid(reason), SemanticError::InvalidClosure { reason: actual, .. }) => {
            assert_eq!(error.to_string(), format!("invalid closure: {reason}"));
            actual == reason
        }
        (ErrorKind::Inference, SemanticError::CannotInferClosureResult { .. }) => {
            assert_eq!(error.to_string(), "cannot infer a concrete closure result");
            true
        }
        (
            ErrorKind::Arity,
            SemanticError::ArgumentCountMismatch {
                name,
                expected: 1,
                found: 2,
                ..
            },
        )
        | (ErrorKind::Named, SemanticError::NamedArgumentsNotSupported { name, .. })
        | (ErrorKind::MissingReturn, SemanticError::MissingReturn { name, .. }) => {
            name == "<closure>"
        }
        (ErrorKind::ReadOnly, SemanticError::ReadOnlyVariable { name, .. }) => name == "x",
        _ => false,
    };
    assert!(matches, "expected {kind:?}, got {error:?}");
    let start = source.rfind(origin).unwrap();
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
    let width = origin.chars().count();
    let expected = format!(
        "bad.was:{line}:{column}: semantic error: {error}\n  |\n{line} | {excerpt}\n  | {}{}",
        " ".repeat(padding),
        "^".repeat(width)
    );
    assert_eq!(diagnostic::render("bad.was", source, &error), expected);
    expected
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_closure_wasm_and_wat_match_library_at_all_levels_and_export_selections() {
    let scratch = Scratch::new();
    let mut native = compiler();
    for (source, selections) in [
        (
            SOURCE,
            vec![
                vec!["main", "text", "array", "operations"],
                vec!["main"],
                vec![],
            ],
        ),
        (SCALAR, vec![vec!["main"], vec![]]),
        (CAPTURED_COMPOUNDS, vec![vec!["main"], vec![]]),
        (
            "fn main() -> number { let sum = 0; for x in 1..4 { let f = () => x; sum += f(); } for x in [4,5] { let f = () => x; sum += f(); } return sum; }",
            vec![vec!["main"], vec![]],
        ),
    ] {
        fs::write(scratch.0.join("program.was"), source).unwrap();
        for names in selections {
            native.set_exports(ExportOptions::Only(
                names.iter().map(|name| (*name).into()).collect(),
            ));
            let mut baseline = None;
            for (flag, level) in LEVELS {
                native.set_optimization_level(level);
                let program = native.analyze(source).unwrap();
                let bytes = native.compile(&program).unwrap();
                assert_eq!(native.compile_source(source).unwrap(), bytes);
                assert_eq!(native.compile(&program).unwrap(), bytes);
                Validator::new().validate_all(&bytes).unwrap();
                if let Some(expected) = &baseline {
                    assert_eq!(&bytes, expected);
                } else {
                    baseline = Some(bytes.clone());
                }
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
                    for name in &names {
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
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn cli_captured_compounds_preserve_rhs_effects_and_match_core_execution() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), CAPTURED_COMPOUNDS).unwrap();
    let mut native = compiler();
    native.set_exports(ExportOptions::Only(vec!["main".into()]));
    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        let expected = native.compile_source(CAPTURED_COMPOUNDS).unwrap();
        let output = scratch.cli(&[
            "program.was",
            flag,
            "--emit",
            "wasm",
            "-o",
            "out.wasm",
            "--host",
            "mark=env.mark(number)->number",
            "--export",
            "main",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        let bytes = fs::read(scratch.0.join("out.wasm")).unwrap();
        assert_eq!(bytes, expected);
        let script = r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const events = [];
            const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {mark(n) { events.push(n); return n; }}
            }).exports;
            for (let i = 0; i < 2; i++) {
                // The second rhs() resets n to 100; update stores 100 - 4 = 96.
                assert.equal(e.main(), 378);
                assert.deepEqual(events.splice(0), [2,23,0,2,2,4]);
            }
        "#;
        let output = test_process::output(
            Command::new("node").args(["-e", script]),
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
fn cli_capture_lifetime_survives_recursion_reentry_trap_and_memory_growth() {
    let source = r#"
        fn creator(seed: number, depth: number, fail: bool) -> number[] {
            let values = [seed, seed + 1];
            let finish = () => {
                values[1] += seed;
                if fail { values[2]; }
                values
            };
            retain(values);
            if depth > 0 { creator(seed + 10, depth - 1, false); }
            if seed == 1 { reenter(); }
            return finish();
        }
    "#;
    let scratch = Scratch::new();
    fs::write(scratch.0.join("program.was"), source).unwrap();
    let mut native = Compiler::new();
    native
        .register_host_function(
            "retain",
            "env",
            "retain",
            vec![Type::NumberArray],
            Type::Void,
        )
        .unwrap();
    native
        .register_host_function("reenter", "env", "reenter", vec![], Type::Void)
        .unwrap();
    native.set_exports(ExportOptions::Only(vec!["creator".into()]));

    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const retained = [];
        const initial = [];
        let e;
        let reentries = 0;
        let reentered;
        function readArray(handle) {
            handle >>>= 0;
            // Never retain a view across a guest call or host allocation.
            const view = new DataView(e.memory.buffer);
            const data = view.getUint32(handle, true);
            const length = view.getUint32(handle + 4, true);
            assert.equal(length, 2);
            return [view.getFloat64(data, true), view.getFloat64(data + 8, true)];
        }
        function growAndOverwriteAllocation() {
            const before = e.memory.buffer;
            const size = before.byteLength;
            const address = e.__alloc(131072) >>> 0;
            assert.ok(e.memory.buffer.byteLength > size, 'allocation must actually grow memory');
            assert.notEqual(e.memory.buffer, before);
            // Overwrite new storage to expose any reuse of retained descriptors/payloads.
            new Uint8Array(e.memory.buffer, address, 131072).fill(0xa5);
        }
        e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {env: {
            retain(handle) {
                retained.push(handle >>> 0);
                initial.push(readArray(handle));
            },
            reenter() {
                assert.equal(++reentries, 1);
                assert.deepEqual(retained.map(readArray), [[1,2], [11,23], [21,43]]);
                reentered = e.creator(100, 1, 0) >>> 0;
                growAndOverwriteAllocation();
            }
        }}).exports;

        const returned = e.creator(1, 2, 0) >>> 0;
        assert.equal(reentries, 1);
        assert.equal(returned, retained[0]);
        assert.equal(reentered, retained[3]);
        assert.deepEqual(retained.map(readArray), [[1,3], [11,23], [21,43], [100,201], [110,221]]);

        // The helper writes before a bounded out-of-bounds trap; neither is rolled back.
        assert.throws(() => e.creator(7, 1, 1), WebAssembly.RuntimeError);
        assert.deepEqual(readArray(retained[5]), [7,15]);
        assert.deepEqual(readArray(retained[6]), [17,35]);
        const later = e.creator(40, 0, 0) >>> 0;
        assert.equal(later, retained[7]);
        growAndOverwriteAllocation();
        assert.equal(new Set(retained).size, 8);
        assert.deepEqual(initial, [
            [1,2], [11,12], [21,22], [100,101], [110,111], [7,8], [17,18], [40,41]
        ]);
        assert.deepEqual(retained.map(readArray), [
            [1,3], [11,23], [21,43], [100,201], [110,221], [7,15], [17,35], [40,81]
        ]);
        assert.deepEqual(readArray(returned), [1,3]);
        assert.deepEqual(readArray(reentered), [100,201]);
        assert.deepEqual(readArray(later), [40,81]);
    "#;

    for (flag, level) in LEVELS {
        native.set_optimization_level(level);
        let expected = native.compile_source(source).unwrap();
        let output = scratch.cli(&[
            "program.was",
            flag,
            "--emit",
            "wasm",
            "-o",
            "out.wasm",
            "--host",
            "retain=env.retain(number[])->void",
            "--host",
            "reenter=env.reenter()->void",
            "--export",
            "creator",
        ]);
        assert!(
            output.status.success(),
            "{level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        let bytes = fs::read(scratch.0.join("out.wasm")).unwrap();
        assert_eq!(bytes, expected, "{level:?}: CLI/core byte parity");
        Validator::new().validate_all(&bytes).unwrap();
        let output = test_process::output(
            Command::new("node").args(["-e", script]),
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
fn cli_semantic_errors_keep_focused_utf8_crlf_spans_and_existing_outputs() {
    let scratch = Scratch::new();
    for (body, origin, kind) in ERRORS {
        let source = format!("// Unicode é😀\r\nfn main() {{\r\n\t/*é😀*/ {body}\r\n}}\r\n");
        let expected = expected_diagnostic(&source, origin, kind);
        fs::write(scratch.0.join("bad.was"), &source).unwrap();
        for (flag, level) in LEVELS {
            let mut native = compiler();
            native.set_optimization_level(level);
            for error in [
                native.analyze(&source).unwrap_err(),
                native.compile_source(&source).unwrap_err(),
            ] {
                assert_eq!(diagnostic::render("bad.was", &source, &error), expected);
            }
            for destination in ["out.wasm", "out.wat"] {
                fs::write(scratch.0.join(destination), b"keep existing output").unwrap();
                let output = scratch.cli(&["bad.was", flag, "-o", destination]);
                assert_eq!(output.status.code(), Some(1));
                assert!(output.stdout.is_empty());
                assert_eq!(
                    String::from_utf8(output.stderr).unwrap(),
                    format!("{expected}\n")
                );
                assert_eq!(
                    fs::read(scratch.0.join(destination)).unwrap(),
                    b"keep existing output"
                );
            }
            let mut entries: Vec<_> = fs::read_dir(&scratch.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            entries.sort();
            assert_eq!(entries, ["bad.was", "out.wasm", "out.wat"]);
        }
    }
    fs::write(scratch.0.join("bad.was"), SCALAR).unwrap();
    assert!(scratch.cli(&["bad.was", "-o", "out.wasm"]).status.success());
    Validator::new()
        .validate_all(&fs::read(scratch.0.join("out.wasm")).unwrap())
        .unwrap();
}

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js and pkg freshly rebuilt from current source"]
fn rebuilt_browser_closure_bytes_wat_execution_and_diagnostics_match_native() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "rebuild pkg before browser tests"
        );
    }
    let names = ["main", "text", "array", "operations"];
    let mut input = Vec::new();
    field(&mut input, SOURCE.as_bytes());
    field(&mut input, names.join(",").as_bytes());
    let mut native = compiler();
    for (_, level) in LEVELS {
        native.set_optimization_level(level);
        for selected in [&names[..], &["main"][..], &[][..]] {
            native.set_exports(ExportOptions::Only(
                selected.iter().map(|name| (*name).into()).collect(),
            ));
            let bytes = native.compile_source(SOURCE).unwrap();
            field(&mut input, &bytes);
            field(
                &mut input,
                wasmprinter::print_bytes(&bytes).unwrap().as_bytes(),
            );
        }
    }
    field(
        &mut input,
        &u32::try_from(ERRORS.len()).unwrap().to_le_bytes(),
    );
    for (body, origin, kind) in ERRORS {
        let source = format!("// Unicode é😀\r\nfn main() {{\r\n\t/*é😀*/ {body}\r\n}}\r\n");
        let expected = expected_diagnostic(&source, origin, kind).replace("bad.was:", "<browser>:");
        field(&mut input, source.as_bytes());
        field(&mut input, expected.as_bytes());
    }
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs'), path = require('node:path');
        const {pathToFileURL} = require('node:url');
        async function main() {
            const input = fs.readFileSync(0); let offset = 0;
            function field() {
                const n = input.readUInt32LE(offset); offset += 4;
                const bytes = input.subarray(offset, offset+n); offset += n; return bytes;
            }
            const source = field().toString('utf8'), names = field().toString('utf8').split(',');
            const expected = Array.from({length:4}, () => Array.from({length:3},
                () => ({bytes:field(), wat:field().toString('utf8')})));
            const count = field().readUInt32LE(0);
            const errors = Array.from({length:count}, () => ({source:field().toString('utf8'), message:field().toString('utf8')}));
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const {default:init, Compiler} = await import(pathToFileURL(path.join(directory, 'was_compiler.js')).href);
            await init({module_or_path:fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm'))});
            const compiler = new Compiler();
            try {
                compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'number');
                for (let level = 0; level < 4; level++) {
                    compiler.setOptimizationLevel(level);
                    for (const [index, selected] of [names, ['main'], []].entries()) {
                        compiler.setExports(selected);
                        const bytes = compiler.compile(source);
                        assert.ok(WebAssembly.validate(bytes));
                        assert.deepEqual(Buffer.from(bytes), expected[level][index].bytes);
                        assert.equal(compiler.compileWat(source), expected[level][index].wat);
                    }
                    compiler.setExports(names);
                    const events = [];
                    const e = new WebAssembly.Instance(new WebAssembly.Module(compiler.compile(source)),
                        {env:{mark(n) {events.push(n); return n;}}}).exports;
                    assert.equal(e.main(), 6); assert.deepEqual(events.splice(0), [3,2]);
                    assert.equal(e.operations(), 1);
                    function contents(h, text = false) {
                        const v = new DataView(e.memory.buffer);
                        const p = v.getUint32(h >>> 0, true), n = v.getUint32((h >>> 0)+4, true);
                        return text ? Buffer.from(e.memory.buffer,p,n).toString('utf8') :
                            Array.from({length:n}, (_, i) => v.getFloat64(p+8*i,true));
                    }
                    assert.equal(contents(e.text(), true), 'hé😀');
                    assert.deepEqual(contents(e.array()), [1,2]);
                    assert.deepEqual(Object.keys(e).sort(), ['__alloc','array','main','memory','operations','text']);
                    for (const error of errors) {
                        for (const method of ['compile','compileWat']) {
                            assert.throws(() => compiler[method](error.source), e => e instanceof Error && e.message === error.message);
                        }
                    }
                    assert.deepEqual(Buffer.from(compiler.compile(source)), expected[level][0].bytes);
                }
            } finally {compiler.free();}
        }
        main().catch(error => {console.error(error); process.exitCode=1;});
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
}
