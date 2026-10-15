#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::PathBuf, process::Command, time::Duration};
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

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js and pkg freshly rebuilt by the integrator with WP8"]
fn rebuilt_browser_contract_bytes_diagnostics_raw_calls_and_recovery_match_native() {
    let package = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "rebuild pkg before running browser contract tests"
        );
    }
    let mut input = Vec::new();
    field(&mut input, SOURCE.as_bytes());
    field(
        &mut input,
        &u32::try_from(ERRORS.len()).unwrap().to_le_bytes(),
    );
    let mut native = compiler();
    for (_, level) in LEVELS {
        native.set_optimization_level(level);
        let bytes = native.compile_source(SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        field(&mut input, &bytes);
        field(
            &mut input,
            wasmprinter::print_bytes(&bytes).unwrap().as_bytes(),
        );
    }
    for (source, origin, message, kind) in ERRORS {
        field(&mut input, source.as_bytes());
        field(
            &mut input,
            expected_diagnostic("<browser>", source, origin, message, kind).as_bytes(),
        );
    }
    let registration_error = native
        .register_host_function("requires", "env", "f", vec![], Type::Void)
        .unwrap_err();
    field(
        &mut input,
        format!("host registration error: {registration_error}").as_bytes(),
    );
    for names in [&["main"][..], &[][..]] {
        native.set_exports(ExportOptions::Only(
            names.iter().map(|n| (*n).into()).collect(),
        ));
        field(&mut input, &native.compile_source(SOURCE).unwrap());
    }
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        const path = require('node:path');
        const {pathToFileURL} = require('node:url');
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
            const source = field().toString('utf8');
            const diagnosticCount = field().readUInt32LE(0);
            const native = Array.from({length: 4}, () => ({bytes: field(), wat: field().toString('utf8')}));
            const diagnostics = Array.from({length: diagnosticCount}, () => ({source: field().toString('utf8'), message: field().toString('utf8')}));
            const registration = field().toString('utf8');
            const mainOnly = field(), none = field();
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const {default: init, Compiler} = await import(pathToFileURL(path.join(directory, 'was_compiler.js')).href);
            await init({module_or_path: fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm'))});
            const compiler = new Compiler();
            try {
                assert.throws(() => compiler.registerHostFunction('requires', 'env', 'f', [], 'void'),
                    error => { assert.equal(error.message, registration); return true; });
                compiler.registerHostFunction('idle', 'requires', 'requires', [], 'void');
                compiler.registerHostFunction('mark', 'env', 'mark', ['number'], 'number');
                const selected = ['main', 'target', 'reject', 'count', 'recursive'];
                function same(expected) {
                    const bytes = compiler.compile(source);
                    assert.ok(WebAssembly.validate(bytes));
                    assert.deepEqual(Buffer.from(bytes), expected.bytes);
                    assert.equal(compiler.compileWat(source), expected.wat);
                }
                for (let level = 0; level < 4; level++) {
                    compiler.setOptimizationLevel(level); compiler.setExports(selected);
                    same(native[level]);
                    for (const diagnostic of diagnostics) {
                        for (const method of ['compile', 'compileWat']) {
                            compiler.exportAll();
                            assert.throws(() => compiler[method](diagnostic.source), error => {
                                assert.ok(error instanceof Error); assert.equal(error.name, 'Error');
                                assert.equal(error.message, diagnostic.message); return true;
                            });
                            compiler.setExports(selected);
                            same(native[level]);
                        }
                    }
                    const events = [];
                    const e = new WebAssembly.Instance(new WebAssembly.Module(compiler.compile(source)), {
                        requires: {requires() {throw new Error('unused import executed');}},
                        env: {mark(n) {events.push(n); return n;}}
                    }).exports;
                    assert.equal(e.main(), 585); assert.deepEqual(events.splice(0), [9, 3, 10, 10]);
                    assert.equal(e.target.length, 3);
                    assert.equal(e.target(3, 4, 9), 349); assert.deepEqual(events.splice(0), [10]);
                    for (const args of [[3, 0, 9], [3], [-1, 0, 9]]) {
                        assert.throws(() => e.target(...args), WebAssembly.RuntimeError);
                    }
                    assert.throws(() => e.reject(1), WebAssembly.RuntimeError);
                    assert.equal(e.recursive(4), 10);
                    assert.throws(() => e.recursive(-1), WebAssembly.RuntimeError);
                    assert.deepEqual(events, []);
                    compiler.setExports(['main']);
                    assert.deepEqual(Buffer.from(compiler.compile(source)), mainOnly);
                    compiler.exportNone();
                    assert.deepEqual(Buffer.from(compiler.compile(source)), none);
                    compiler.setExports(selected); same(native[level]);
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
