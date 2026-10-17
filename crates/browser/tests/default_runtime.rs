#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, OptimizationLevel};

#[test]
#[ignore = "requires Node.js and freshly rebuilt browser bindings in pkg"]
fn rebuilt_browser_defaults_match_native_bytes_origins_raw_arity_and_recovery() {
    let source = "fn main() -> number { return f(c: 9, a: 3); } fn f(a: number, b: number = a + 1, c: number = b * 2) -> number { return a * 100 + b * 10 + c; } fn omitted() -> number { return f(2); }";
    let native = [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ]
    .map(|level| {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        compiler.compile_source(source).unwrap()
    });
    let cases = [
        "// é\r\nfn f(a: number, b: number = missing) -> number { return b; }",
        "// é\r\nfn f(a: number, b: number = 1) -> number { return b; }\r\nfn main() -> number {\r\n\treturn f(b: true, a: 1);\r\n}",
        "fn f(a: number, b: number = a[0]) -> number { return b; }",
        "fn f(a: number, b: number = 1) -> number { return b; } fn main() -> number { return f(b: 2); }",
        "fn f(a: number = 1) -> number { return a; } fn main() -> number { return f(a: 2, a: 3); }",
        "fn f(a: number = 1) -> number { return a; } fn main() -> number { return f(unknown: 2); }",
        "fn f(a: number, b: number = 1) -> number { return b; } fn main() -> number { return f(a: 2, 3); }",
        "fn f(xs: number[], n: number = len(xs: xs)) -> number { return n; }",
    ];
    let mut checks = String::new();
    for case in cases {
        let error = Compiler::new().compile_source(case).unwrap_err();
        let message = was_compiler::diagnostic::render("<browser>", case, &error);
        checks.push_str(&format!(
            "for (const method of ['compile', 'compileWat']) {{ assert.throws(() => compiler[method]({case:?}), error => {{ assert.equal(error.message, {message:?}); return true; }}); }}\n"
        ));
    }
    let package = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("pkg");
    for file in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(file).is_file(),
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
            const compiler = new Compiler();
            const native = {native:?};
            try {{
                for (let level = 0; level < 4; level++) {{
                    compiler.setOptimizationLevel(level);
                    assert.deepEqual(Array.from(compiler.compile({source:?})), native[level]);
                    const e = new WebAssembly.Instance(new WebAssembly.Module(compiler.compile({source:?}))).exports;
                    assert.equal(e.main(), 349);
                    assert.equal(e.omitted(), 236);
                    assert.equal(e.f.length, 3);
                    assert.equal(e.f(3, 8, 5), 385);
                    assert.equal(e.f(3, 0, 0), 300);
                    {checks}
                    assert.deepEqual(Array.from(compiler.compile({source:?})), native[level]);
                }}
            }} finally {{ compiler.free(); }}
        }})().catch(error => {{ console.error(error); process.exitCode = 1; }});
    "#
    );
    let output = test_process::output(
        Command::new("node").arg("-e").arg(script).arg(package),
        None,
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
