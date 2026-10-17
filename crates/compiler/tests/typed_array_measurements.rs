#[path = "../../../tests/support/typed_array_measure_corpus.rs"]
mod corpus;

#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{Compiler, CompilerOptions, OptimizationLevel};
use wasmparser::{ExternalKind, Parser, Payload, Validator};

fn execute(mode: &str, cases: Vec<corpus::Case>) {
    let input = cases
        .into_iter()
        .map(|case| {
            let bytes = Compiler::new().compile_source(&case.source).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            format!("[\"{}\",{:?},{bytes:?}]", case.name, case.source.as_bytes())
        })
        .collect::<Vec<_>>()
        .join(",");
    let input = format!("[{input}]");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/support/typed_array_measure.cjs"
    );
    let output = test_process::output(
        Command::new("node").args([script, mode]),
        Some(input.as_bytes()),
        Duration::from_secs(60),
    );
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{stdout}");
    assert!(stdout.contains("typed-array measurements passed"));
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn typed_array_workloads_execute_with_independent_results_and_allocation_checks() {
    execute("verify", corpus::cases());
}

#[test]
#[ignore = "manual bounded M4 marker measurement; requires Node.js"]
fn record_typed_array_markers() {
    execute("marker", corpus::cases());
}

#[test]
#[ignore = "manual bounded M4 capacity measurement; requires Node.js"]
fn record_typed_array_capacity() {
    execute("capacity", corpus::cases());
}

#[test]
#[ignore = "manual bounded M4 guest timing measurement; requires Node.js"]
fn record_typed_array_guest_timing() {
    execute("timing", corpus::cases());
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn descriptor_bytes_independently_confirm_slot_widths_and_shared_string_handles() {
    let cases = [
        ("number", "[1,0,1]"),
        ("bool", "[true,false,true]"),
        ("string", "[\"é\",\"x\",\"é\"]"),
    ]
    .into_iter()
    .map(|(element, literal)| corpus::Case {
        name: element.into(),
        source: format!("fn main() -> {element}[] {{ return {literal}; }}"),
    })
    .collect();
    execute("layout", cases);
}

#[test]
fn bounded_typed_array_workloads_use_public_compile_and_normal_memory_exports() {
    let cases = corpus::cases();
    assert_eq!(cases.len(), 48);
    for case in cases {
        let compiler = Compiler::new();
        let typed = compiler.analyze(&case.source).unwrap();
        let bytes = compiler.compile(&typed).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(bytes, compiler.compile_source(&case.source).unwrap());
        let mut exports = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::ExportSection(section) => {
                    exports.extend(section.into_iter().map(|export| {
                        let export = export.unwrap();
                        (export.name.to_owned(), export.kind)
                    }));
                }
                Payload::ImportSection(section) => assert_eq!(section.count(), 0),
                Payload::StartSection { .. } => panic!("no start effects"),
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
            ]
        );
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
