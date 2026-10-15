#[path = "../benches/support/corpus.rs"]
mod corpus;

#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{Compiler, CompilerOptions, OptimizationLevel};
use wasmparser::Validator;

#[test]
fn corpus_is_deterministic_valid_and_optimization_invariant() {
    for workload in corpus::CASES {
        let source = corpus::source(workload);
        assert_eq!(source, corpus::source(workload));
        let expected = Compiler::new().compile_source(&source).unwrap();
        Validator::new().validate_all(&expected).unwrap();

        for optimization_level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let compiler = Compiler::with_options(CompilerOptions { optimization_level });
            let typed = compiler.analyze(&source).unwrap();
            assert_eq!(compiler.compile(&typed).unwrap(), expected, "{workload}");
        }
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn benchmark_workloads_execute_against_independent_expected_results() {
    let mut cases = Vec::new();
    for workload in corpus::CASES {
        let source = corpus::source(workload);
        let bytes = Compiler::new().compile_source(&source).unwrap();
        cases.push(format!("[\"{workload}\", {bytes:?}]"));
    }
    let input = format!("[{}]", cases.join(","));

    let script = r#"
const assert = require('node:assert/strict');
const cases = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
for (const [name, bytes] of cases) {
    const [family, size] = name.split('_');
    const n = Number(size);
    let expected;
    if (family === 'scalar') {
        expected = n * (n - 1) / 2 * 1.5 - n * 0.25;
    } else if (family === 'calls') {
        expected = Math.min(n, 8) ** 2 - Math.max(n - 8, 0);
    } else {
        expected = 0;
        for (let k = 1; k <= n; k++) {
            // Compound assignment retains its pre-call value across RHS writes.
            const before = expected;
            expected += 1 + 6 * k;
            expected += 2;
            expected += 3;
            expected = before + expected;
        }
    }
    const instance = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array(bytes)));
    assert.equal(instance.exports.main(), expected, name);
}
console.log('all nine benchmark workload results passed');
"#;
    let output = test_process::output(
        Command::new("node").args(["-e", script]),
        Some(input.as_bytes()),
        Duration::from_secs(30),
    );
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("all nine benchmark workload results passed")
    );
}

#[test]
fn workload_sizes_increase_source_and_module_sizes() {
    for workloads in corpus::CASES.as_chunks::<3>().0 {
        let mut previous = (0, 0);
        for &workload in workloads {
            let source = corpus::source(workload);
            let module = Compiler::new().compile_source(&source).unwrap();
            assert!(source.len() > previous.0, "{workload}");
            assert!(module.len() > previous.1, "{workload}");
            previous = (source.len(), module.len());
        }
    }
}
