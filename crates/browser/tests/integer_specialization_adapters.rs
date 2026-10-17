#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::Path, process::Command, time::Duration};
use was_compiler::{Compiler, OptimizationLevel};
use wasmparser::Validator;

#[test]
#[ignore = "requires Node.js and freshly rebuilt browser bindings in pkg"]
fn rebuilt_browser_integer_specialization_bytes_and_wat_match_library_at_every_level() {
    let source = "fn main() -> number { return (10 + 20) - 3; }";
    let mut compiler = Compiler::new();
    let baseline = compiler.compile_source(source).unwrap();
    let mut native = Vec::new();
    let mut native_wat = Vec::new();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(bytes, baseline);
        } else {
            assert_ne!(
                bytes, baseline,
                "{level:?} must specialize literal addition/subtraction"
            );
        }
        native_wat.push(wasmprinter::print_bytes(&bytes).unwrap());
        native.push(bytes);
    }

    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let package = root.join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "rebuild browser bindings before this test"
        );
    }
    let script = format!(
        r#"
        const assert = require('node:assert/strict');
        const path = require('node:path');
        const fs = require('node:fs/promises');
        const {{ pathToFileURL }} = require('node:url');
        (async () => {{
            const directory = process.argv[1];
            const {{ default: init, Compiler }} = await import(pathToFileURL(path.join(directory, 'was_compiler.js')).href);
            await init({{ module_or_path: await fs.readFile(path.join(directory, 'was_compiler_bg.wasm')) }});
            const native = {native:?};
            const nativeWat = {native_wat:?};
            const compiler = new Compiler();
            try {{
                assert.deepEqual(Array.from(compiler.compile({source:?})), native[0]);
                for (const level of [0, 1, 2, 3, 0]) {{
                    compiler.setOptimizationLevel(level);
                    const bytes = compiler.compile({source:?});
                    assert.deepEqual(Array.from(bytes), native[level], `level ${{level}}`);
                    assert.equal(compiler.compileWat({source:?}), nativeWat[level]);
                    assert.ok(WebAssembly.validate(bytes));
                    const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));
                    assert.equal(instance.exports.main(), 27);
                    if (level > 0) {{
                        assert.notDeepEqual(Array.from(bytes), native[0]);
                        assert.deepEqual(Array.from(bytes), native[1]);
                    }}
                    assert.deepEqual(Array.from(compiler.compile({source:?})), native[level]);
                }}
            }} finally {{ compiler.free(); }}
        }})().catch(error => {{ console.error(error); process.exitCode = 1; }});
        "#
    );
    let output = test_process::output(
        Command::new("node")
            .arg("-e")
            .arg(script)
            .arg(package)
            .current_dir(root),
        None,
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
