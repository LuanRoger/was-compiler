#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::PathBuf, process::Command, time::Duration};
use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic,
    tokens::Span,
};
use wasmparser::Validator;

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

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js ESM and pkg rebuilt by the integrator from current WP7 + released WP6"]
fn node_rebuilt_browser_inference_defaults_bytes_diagnostics_recovery_and_range_remedies() {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
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
    for (_, level) in LEVELS {
        compiler.set_optimization_level(level);
        compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
        field(&mut input, &compiler.compile_source(SOURCE).unwrap());
        compiler.set_exports(ExportOptions::None);
        field(&mut input, &compiler.compile_source(SOURCE).unwrap());
    }

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
            const selections = Array.from({length: 4}, () => ({mainOnly: field(), none: field()}));
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
                    const events = [];
                    const e = new WebAssembly.Instance(new WebAssembly.Module(compiler.compile(inferred)), {
                        app: { unused() { throw new Error('unused import executed'); } },
                        env: { mark(n) { events.push(n); return n; } },
                    }).exports;
                    assert.equal(e.main(), 747);
                    assert.deepEqual(events, [9, 3]);
                    assert.equal(e.target.length, 3);
                    assert.equal(e.target(3, 4, 9), 349);
                    assert.ok(Number.isNaN(e.target(3)));
                    for (const diagnostic of diagnostics) {
                        fails(diagnostic.source, diagnostic.message);
                        same(inferred, native[level]); same(explicit, native[level]);
                    }
                    compiler.exportAll(); fails(inferred, rangeAll);
                    compiler.setExports(['main']); same(inferred, selections[level].mainOnly); same(explicit, selections[level].mainOnly);
                    compiler.setExports(['make']); fails(inferred, rangeSelected);
                    compiler.exportNone(); same(inferred, selections[level].none); same(explicit, selections[level].none);
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
