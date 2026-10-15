#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{
    CompileError, Compiler, ExportOptions, OptimizationLevel, Type, semantic::SemanticError,
};
use wasmparser::Validator;

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const PARITY_SOURCE: &str = r#"
    fn n(x: number) /*number*/ { return sample(x) + 1; }
    fn b(x: bool) /*bool*/ { return !x; }
    fn s() /*string*/ { return "é"; }
    fn a() /*number[]*/ { return [1, 2]; }
    fn v() /*void*/ { emit(3); return; }
    fn r(start: number) /*range*/ { return start..=5 by 2; }
    fn choose(flag: bool) /*number*/ {
        if flag { return { let x = 7; x }; }
        return loop { break 9; };
    }
    fn main() -> number { let total = 0; for i in r(1) { total += i; } return total; }
"#;

fn parity_compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(
        ["n", "b", "s", "a", "v", "choose", "main"]
            .map(String::from)
            .to_vec(),
    ));
    compiler
        .register_host_function("sample", "env", "sample", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("emit", "env", "emit", vec![Type::Number], Type::Void)
        .unwrap();
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler
}

fn annotated_parity_source() -> String {
    let mut source = PARITY_SOURCE.to_owned();
    for ty in ["number", "bool", "string", "number[]", "void", "range"] {
        source = source.replace(&format!("/*{ty}*/"), &format!("-> {ty}"));
    }
    source
}

#[test]
#[ignore = "requires Node.js with ESM support and a freshly rebuilt browser package in pkg"]
fn node_rebuilt_browser_result_inference_matches_native_bytes_diagnostics_and_recovery() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let package = root.join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "missing {}; rebuild the isolated browser package before this ignored test",
            package.join(name).display()
        );
    }

    // Length-prefixed fields keep native bytes and exact UTF-8 diagnostics out of
    // command-line quoting and avoid writing temporary fixtures or adding serde.
    let mut input = Vec::new();
    let mut field = |bytes: &[u8]| {
        let length = u32::try_from(bytes.len()).unwrap();
        input.extend_from_slice(&length.to_le_bytes());
        input.extend_from_slice(bytes);
    };
    let explicit = annotated_parity_source();
    field(PARITY_SOURCE.as_bytes());
    field(explicit.as_bytes());
    let mut compiler = parity_compiler();
    let mut expected = Vec::with_capacity(LEVELS.len());
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(PARITY_SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(compiler.compile_source(&explicit).unwrap(), bytes);
        field(&bytes);
        expected.push(bytes);
    }

    let cases = [
        (
            "// é\r\n\tfn recurse() {\r\n\treturn recurse();\r\n}\r\n",
            "fn recurse() {\r\n\treturn recurse();\r\n}",
            "<browser>:2:2: semantic error:",
        ),
        (
            "// é\r\nfn callee() { return true; }\r\nfn caller() {\r\n\treturn callee() + 1;\r\n}\r\n",
            "return callee() + 1;",
            "<browser>:4:2: semantic error:",
        ),
        (
            "// é\r\n\tfn mixed(flag: bool) {\r\n\tif flag { return 1; }\r\n\treturn true;\r\n}\r\n",
            "fn mixed(flag: bool) {\r\n\tif flag { return 1; }\r\n\treturn true;\r\n}",
            "<browser>:2:2: semantic error:",
        ),
        (
            "// é\r\n\tfn spin() {\r\n\treturn loop {};\r\n}\r\n",
            "fn spin() {\r\n\treturn loop {};\r\n}",
            "<browser>:2:2: semantic error:",
        ),
    ];
    for (index, (source, excerpt, header)) in cases.into_iter().enumerate() {
        let error = compiler.compile_source(source).unwrap_err();
        match (index, &error) {
            (
                0,
                CompileError::Semantic(SemanticError::RecursiveResultAnnotationRequired {
                    name,
                    ..
                }),
            ) => {
                assert_eq!(name, "recurse");
            }
            (
                1,
                CompileError::Semantic(SemanticError::InvalidBinaryOperands {
                    operator,
                    left: Type::Bool,
                    right: Type::Number,
                    ..
                }),
            ) if *operator == "+" => {}
            (
                2,
                CompileError::Semantic(SemanticError::IncompatibleInferredReturns {
                    expected: Type::Number,
                    found: Type::Bool,
                    ..
                }),
            ) => {}
            (3, CompileError::Semantic(SemanticError::CannotInferFunctionResult { name, .. })) => {
                assert_eq!(name, "spin");
            }
            _ => panic!("unexpected native diagnostic for {source}: {error:?}"),
        }
        let span = error.span().unwrap();
        assert_eq!(&source[span.start()..span.end()], excerpt);
        let message = was_compiler::diagnostic::render("<browser>", source, &error);
        assert!(message.starts_with(header), "{message}");
        assert!(!message.contains('\r'));
        assert!(!message.contains('\t'));
        assert!(message.contains('^'));
        field(source.as_bytes());
        field(message.as_bytes());
        for (level, bytes) in LEVELS.into_iter().zip(&expected) {
            compiler.set_optimization_level(level);
            let current = compiler.compile_source(source).unwrap_err();
            assert_eq!(
                was_compiler::diagnostic::render("<browser>", source, &current),
                message
            );
            assert_eq!(&compiler.compile_source(PARITY_SOURCE).unwrap(), bytes);
            assert_eq!(&compiler.compile_source(&explicit).unwrap(), bytes);
        }
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
                const length = input.readUInt32LE(offset);
                offset += 4;
                assert.ok(offset + length <= input.length);
                const bytes = input.subarray(offset, offset + length);
                offset += length;
                return bytes;
            }
            const inferred = field().toString('utf8');
            const explicit = field().toString('utf8');
            const native = [field(), field(), field(), field()];
            const diagnostics = Array.from({length: 4}, () => ({
                source: field().toString('utf8'),
                message: field().toString('utf8'),
            }));
            assert.equal(offset, input.length);
            const directory = process.argv[1];
            const { default: init, Compiler } = await import(
                pathToFileURL(path.join(directory, 'was_compiler.js')).href
            );
            await init({ module_or_path: fs.readFileSync(path.join(directory, 'was_compiler_bg.wasm')) });
            const compiler = new Compiler();
            try {
                compiler.registerHostFunction('sample', 'env', 'sample', ['number'], 'number');
                compiler.registerHostFunction('emit', 'env', 'emit', ['number'], 'void');
                compiler.registerHostFunction('unused', 'env', 'unused', [], 'void');
                compiler.setExports(['n', 'b', 's', 'a', 'v', 'choose', 'main']);
                function sameBytes(source, expected) {
                    const bytes = compiler.compile(source);
                    assert.ok(bytes instanceof Uint8Array);
                    assert.ok(WebAssembly.validate(bytes));
                    assert.deepEqual(Buffer.from(bytes), expected);
                }
                for (const level of [0, 1, 2, 3]) {
                    compiler.setOptimizationLevel(level);
                    sameBytes(inferred, native[level]);
                    sameBytes(explicit, native[level]);
                    for (const diagnostic of diagnostics) {
                        assert.throws(() => compiler.compile(diagnostic.source), error => {
                            assert.ok(error instanceof Error);
                            assert.equal(error.name, 'Error');
                            assert.equal(error.message, diagnostic.message);
                            return true;
                        });
                        sameBytes(inferred, native[level]);
                        sameBytes(explicit, native[level]);
                    }
                }
            } finally {
                compiler.free();
            }
        }
        main().catch(error => { console.error(error); process.exitCode = 1; });
    "#;
    let output = test_process::output(
        Command::new("node")
            .arg("-e")
            .arg(script)
            .arg(package)
            .current_dir(root),
        Some(&input),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "browser inference parity failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
