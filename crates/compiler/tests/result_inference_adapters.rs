#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{CodegenError, Compiler, ExportOptions, OptimizationLevel, Type};
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

#[test]
fn inferred_defaulted_analysis_retains_the_host_snapshot_and_private_range_export_policy() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let expected = compiler.compile(&program).unwrap();
    compiler
        .register_host_function("later", "env", "later", vec![], Type::Void)
        .unwrap();
    for (_, level) in LEVELS {
        compiler.set_optimization_level(level);
        let selected_bytes = compiler.compile(&program).unwrap();
        wasmparser::Validator::new()
            .validate_all(&selected_bytes)
            .unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(selected_bytes, expected);
        }
        let expected = selected_bytes;
        assert_eq!(compiler.compile(&program).unwrap(), expected);
        assert_ne!(compiler.compile_source(SOURCE).unwrap(), expected);
        compiler.set_exports(ExportOptions::All);
        assert!(
            matches!(compiler.compile(&program), Err(CodegenError::RangeExport { function, .. }) if function == "ranges")
        );
        compiler.set_exports(ExportOptions::Only(vec!["make".into()]));
        assert!(
            matches!(compiler.compile(&program), Err(CodegenError::RangeExport { function, .. }) if function == "make")
        );
        compiler.set_exports(ExportOptions::None);
        Validator::new()
            .validate_all(&compiler.compile(&program).unwrap())
            .unwrap();
        compiler.set_exports(ExportOptions::Only(
            SELECTED.iter().map(|name| (*name).into()).collect(),
        ));
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_inferred_defaulted_exports_remain_full_arity_and_calls_apply_defaults() {
    let mut compiler = compiler();
    for (_, level) in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let script = r#"
            const assert = require('node:assert/strict');
            const events = [];
            const bytes = require('node:fs').readFileSync(0);
            const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                app: { unused() { throw new Error('unused import executed'); } },
                env: { mark(n) { events.push(n); return n; } },
            }).exports;
            assert.equal(e.main(), 747);
            assert.deepEqual(events, [9, 3]);
            assert.equal(e.target(3, 4, 9), 349);
            assert.ok(Number.isNaN(e.target(3)));
            assert.equal(e.inferred(12), 12);
            assert.ok(Number.isNaN(e.inferred()));
            assert.equal(e.flag(0), 0);
            assert.equal(e.flag(1), 1);
        "#;
        let output = test_process::output(
            Command::new("node").args(["-e", script]),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
