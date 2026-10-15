#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type};
use wasmparser::{Parser, Payload, Validator};

const SOURCE: &str = include_str!("../../../tests/fixtures/emitter/control.was");
const GOLDEN: &[u8] = include_bytes!("../../../tests/fixtures/emitter/control.wasm");
const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("marker", "env", "marker", vec![Type::Number], Type::Number)
        .expect("register marker first");
    compiler
        .register_host_function("signal", "env", "signal", vec![Type::Bool], Type::Bool)
        .expect("register signal second");
    compiler
}

#[test]
fn native_emitter_matches_pre_refactor_binary_at_all_levels_without_mutating_typed_input() {
    let mut compiler = compiler();
    assert_eq!(compiler.exports(), &ExportOptions::All);
    assert_eq!(
        compiler.options().optimization_level,
        OptimizationLevel::None
    );
    let program = compiler.analyze(SOURCE).expect("analyze control fixture");
    let original = program.clone();
    Validator::new()
        .validate_all(GOLDEN)
        .expect("valid pre-refactor fixture");

    for level in LEVELS {
        compiler.set_optimization_level(level);
        for _ in 0..2 {
            let bytes = compiler.compile(&program).expect("emit typed fixture");
            assert_eq!(bytes, GOLDEN, "typed compilation at {level:?}");
            Validator::new()
                .validate_all(&bytes)
                .expect("valid native emission");
            assert_eq!(program, original, "typed input changed at {level:?}");
        }
        assert_eq!(
            compiler
                .compile_source(SOURCE)
                .expect("compile source fixture"),
            GOLDEN,
            "source compilation at {level:?}"
        );
        assert_eq!(compiler.analyze(SOURCE).unwrap(), original);
        assert_eq!(program, original);
    }
}

fn non_export_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    Parser::new(0)
        .parse_all(bytes)
        .map(|payload| payload.expect("parse emitted module"))
        .filter_map(|payload| {
            let (id, range) = payload.as_section()?;
            (id != 7).then(|| {
                let start = usize::try_from(range.start).expect("section start fits usize");
                let end = usize::try_from(range.end).expect("section end fits usize");
                (id, bytes[start..end].to_vec())
            })
        })
        .collect()
}

#[test]
fn export_selection_preserves_pre_refactor_non_export_sections_at_all_levels() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let original = program.clone();
    let expected_sections = non_export_sections(GOLDEN);

    for level in LEVELS {
        compiler.set_optimization_level(level);
        for (selection, expected_functions) in [
            (ExportOptions::None, vec![]),
            (ExportOptions::Only(vec!["main".into()]), vec!["main"]),
            (
                ExportOptions::All,
                vec!["main", "forward", "even", "odd", "enabled"],
            ),
        ] {
            compiler.set_exports(selection);
            let bytes = compiler.compile(&program).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(non_export_sections(&bytes), expected_sections, "{level:?}");
            let mut functions = Vec::new();
            for payload in Parser::new(0).parse_all(&bytes) {
                if let Payload::ExportSection(exports) = payload.unwrap() {
                    for export in exports {
                        let export = export.unwrap();
                        if export.kind == wasmparser::ExternalKind::Func
                            && program.functions.iter().any(|f| f.name == export.name)
                        {
                            functions.push(export.name);
                        }
                    }
                }
            }
            assert_eq!(functions, expected_functions, "{level:?}");
            assert_eq!(program, original);
        }
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_emitter_preserves_control_flow_temporaries_bool_normalization_and_host_effect_order() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let original = program.clone();
    let script = r#"
'use strict';
const assert = require('node:assert/strict');
const bytes = require('node:fs').readFileSync(0);
const effects = [];
const {exports: e} = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
    env: {
        marker(value) {
            effects.push(['marker', value]);
            return value;
        },
        signal(value) {
            assert.ok(value === 0 || value === 1, `noncanonical bool argument: ${value}`);
            effects.push(['signal', value]);
            return value !== 0 ? 7 : 0;
        }
    }
});
for (const value of [10, -4, 0, 0.5]) {
    effects.length = 0;
    assert.equal(e.main(value), value + 19);
    assert.deepEqual(effects, [
        ['marker', value], ['marker', 1], ['marker', 2], ['marker', 3],
        ['signal', 1], ['marker', 1], ['marker', 4],
        ['signal', 0],
        ['signal', 1], ['marker', 3], ['marker', 4],
        ['signal', 1], ['marker', 3], ['marker', 4],
        ['marker', 5], ['marker', 6]
    ]);
}
for (const [value, even, odd] of [[-2, 1, 0], [0, 1, 0], [1, 0, 1], [2, 1, 0], [5, 0, 1], [6, 1, 0]]) {
    effects.length = 0;
    assert.equal(e.forward(value), even);
    assert.equal(e.even(value), even);
    assert.equal(e.odd(value), odd);
    assert.deepEqual(effects, []);
}
for (const value of [0, 1, 7, -7]) {
    effects.length = 0;
    assert.equal(e.enabled(value), value === 0 ? 0 : 1);
    assert.deepEqual(effects, value === 0
        ? [['signal', 1]]
        : [['signal', 0], ['signal', 1]]);
}
"#;

    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(15),
        );
        assert!(
            output.status.success(),
            "Node execution at {level:?}:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "unexpected stdout at {level:?}");
        assert!(
            output.stderr.is_empty(),
            "unexpected stderr at {level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(program, original);
    }
}
