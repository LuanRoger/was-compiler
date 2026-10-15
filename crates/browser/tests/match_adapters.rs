#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::PathBuf, process::Command, time::Duration};
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

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js and pkg freshly rebuilt by the integrator with WP9"]
fn rebuilt_browser_match_bytes_wat_diagnostics_and_reserved_names_match_native() {
    let package = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "rebuild pkg before running browser match tests"
        );
    }
    let mut input = Vec::new();
    field(&mut input, SOURCE.as_bytes());
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
    for (source, origin) in ERRORS {
        field(&mut input, source.as_bytes());
        field(
            &mut input,
            expected_diagnostic("<browser>", source, origin).as_bytes(),
        );
    }
    for name in ["match", "_"] {
        let error = native
            .register_host_function(name, "env", "f", vec![], Type::Number)
            .unwrap_err();
        field(
            &mut input,
            format!("host registration error: {error}").as_bytes(),
        );
    }
    for source in [
        "fn match() {}",
        "fn f(_: number) {}",
        "fn f() { let _ = 1; }",
    ] {
        let source = format!("// é🦀\r\n\t{source}\r\n");
        let error = Compiler::new().compile_source(&source).unwrap_err();
        field(&mut input, source.as_bytes());
        field(
            &mut input,
            diagnostic::render("<browser>", &source, &error).as_bytes(),
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
            const source = field().toString('utf8');
            const native = Array.from({length: 4}, () => ({bytes: field(), wat: field().toString('utf8')}));
            const diagnostics = Array.from({length: 5}, () => ({source: field().toString('utf8'), message: field().toString('utf8')}));
            const registration = [field().toString('utf8'), field().toString('utf8')];
            diagnostics.push(...Array.from({length: 3}, () => ({source: field().toString('utf8'), message: field().toString('utf8')})));
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const {default: init, Compiler} = await import(pathToFileURL(path.join(directory, 'was_compiler.js')).href);
            await init({module_or_path: fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm'))});
            const compiler = new Compiler();
            try {
                for (const [i, name] of ['match', '_'].entries()) {
                    assert.throws(() => compiler.registerHostFunction(name, 'env', 'f', [], 'number'),
                        error => { assert.equal(error.message, registration[i]); return true; });
                }
                compiler.registerHostFunction('mark', 'match', '_', ['number'], 'number');
                function same(expected) {
                    const bytes = compiler.compile(source);
                    assert.ok(WebAssembly.validate(bytes));
                    assert.deepEqual(Buffer.from(bytes), expected.bytes);
                    assert.equal(compiler.compileWat(source), expected.wat);
                }
                for (let level = 0; level < 4; level++) {
                    compiler.setOptimizationLevel(level); same(native[level]);
                    for (const diagnostic of diagnostics) {
                        for (const method of ['compile', 'compileWat']) {
                            assert.throws(() => compiler[method](diagnostic.source), error => {
                                assert.ok(error instanceof Error); assert.equal(error.name, 'Error');
                                assert.equal(error.message, diagnostic.message); return true;
                            });
                            same(native[level]);
                        }
                    }
                    const events = [];
                    const e = new WebAssembly.Instance(new WebAssembly.Module(compiler.compile(source)), {
                        match: {_(n) {events.push(n); return n;}}
                    }).exports;
                    assert.equal(e.main(1), 21); assert.deepEqual(events.splice(0), [1]);
                    assert.equal(e.main(1.25), 43); assert.deepEqual(events.splice(0), [1.25]);
                    assert.equal(e.main(NaN), 54); assert.ok(Number.isNaN(events.splice(0)[0]));
                    assert.equal(e.boolean(-7), 1); assert.equal(e.boolean(0), 2);
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
