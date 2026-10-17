#[path = "../../../tests/support/memory_corpus.rs"]
mod corpus;

#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{Compiler, CompilerOptions, OptimizationLevel};
use wasmparser::{ExternalKind, Parser, Payload, Validator};

fn execute(mode: &str) {
    let cases = corpus::cases()
        .into_iter()
        .map(|case| {
            let bytes = Compiler::new().compile_source(&case.source).unwrap();
            format!(
                "[\"{}\",{:?}, {bytes:?}]",
                case.name,
                case.source.as_bytes()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let input = format!("[{cases}]");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/support/memory_measure.cjs"
    );
    let output = test_process::output(
        Command::new("node").args([script, mode]),
        Some(input.as_bytes()),
        Duration::from_secs(60),
    );
    assert!(output.status.success(), "{output:?}");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(String::from_utf8_lossy(&output.stdout).contains("all 17 memory workloads passed"));
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn memory_workloads_execute_with_independent_results_and_allocation_checks() {
    execute("verify");
}

#[test]
#[ignore = "manual bounded M0 allocation baseline; requires Node.js"]
fn record_memory_markers() {
    execute("marker");
}

#[test]
#[ignore = "manual bounded M0 capacity baseline; requires Node.js"]
fn record_memory_capacity() {
    execute("capacity");
}

#[test]
#[ignore = "manual bounded M0 guest timing baseline; requires Node.js"]
fn record_memory_guest_timing() {
    execute("timing");
}

#[test]
fn bounded_memory_workloads_compile_deterministically_at_the_public_seam() {
    for case in corpus::cases() {
        let compiler = Compiler::new();
        let typed = compiler.analyze(&case.source).unwrap();
        let bytes = compiler.compile(&typed).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let mut exports = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::ExportSection(section) => {
                    exports.extend(section.into_iter().map(|export| {
                        let export = export.unwrap();
                        (export.name.to_owned(), export.kind)
                    }));
                }
                Payload::MemorySection(section) => {
                    let memories = section.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
                    assert_eq!(memories.len(), 1);
                    assert_eq!(memories[0].initial, 1);
                    assert_eq!(memories[0].maximum, Some(65535));
                }
                Payload::ImportSection(section) => assert_eq!(section.count(), 0),
                Payload::StartSection { .. } => panic!("workloads must have no startup effects"),
                _ => {}
            }
        }
        exports.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            exports,
            [
                ("__alloc".into(), ExternalKind::Func),
                ("main".into(), ExternalKind::Func),
                ("memory".into(), ExternalKind::Memory),
            ],
            "no profiling exports: {}",
            case.name
        );
        assert_eq!(bytes, compiler.compile_source(&case.source).unwrap());
        let mut enabled_bytes = None;
        for optimization_level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let selected = Compiler::with_options(CompilerOptions { optimization_level });
            let selected_bytes = selected.compile_source(&case.source).unwrap();
            Validator::new().validate_all(&selected_bytes).unwrap();
            if optimization_level == OptimizationLevel::None {
                assert_eq!(selected_bytes, bytes, "{}", case.name);
            } else if let Some(expected) = &enabled_bytes {
                assert_eq!(&selected_bytes, expected, "{}", case.name);
            } else {
                enabled_bytes = Some(selected_bytes.clone());
            }
            assert_eq!(selected.analyze(&case.source).unwrap(), typed);
            assert_eq!(selected.compile(&typed).unwrap(), selected_bytes);
            assert_eq!(
                selected.compile_source(&case.source).unwrap(),
                selected_bytes
            );
        }
    }
}
