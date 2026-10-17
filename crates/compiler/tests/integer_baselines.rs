#[path = "../../../tests/support/integer_corpus.rs"]
mod corpus;

#[path = "../../../tests/support/process.rs"]
mod test_process;

use was_compiler::{Compiler, CompilerOptions, OptimizationLevel};
use wasmparser::{Operator, Parser, Payload, Validator};

#[test]
fn integer_workloads_preserve_the_unoptimized_public_compilation_contract() {
    for case in corpus::cases() {
        let compiler = corpus::compiler(OptimizationLevel::None);
        let typed = compiler.analyze(&case.source).unwrap();
        let bytes = compiler.compile(&typed).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(bytes, compiler.compile_source(&case.source).unwrap());
        assert_eq!(
            case.source,
            corpus::cases()
                .into_iter()
                .find(|c| c.name == case.name)
                .unwrap()
                .source
        );
        let mut floating_constants = 0;
        let mut floating_arithmetic = 0;
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::StartSection { .. } => panic!("unexpected startup effects"),
                Payload::CodeSectionEntry(body) => {
                    for op in body.get_operators_reader().unwrap() {
                        match op.unwrap() {
                            Operator::F64Const { .. } => floating_constants += 1,
                            Operator::F64Add | Operator::F64Sub => floating_arithmetic += 1,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(
            floating_constants > 0,
            "{} retains f64 constants",
            case.name
        );
        if ["literal_", "locals_", "compare_", "negate_"]
            .iter()
            .any(|p| {
                case.name.starts_with(p)
                    && (case.name.ends_with("_1") || case.name.ends_with("_32"))
            })
        {
            assert!(
                floating_arithmetic > 0,
                "{} has baseline f64 arithmetic",
                case.name
            );
        }
        for optimization_level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let selected = corpus::compiler(optimization_level);
            assert_eq!(
                selected.compile_source(&case.source).unwrap(),
                bytes,
                "{}",
                case.name
            );
            assert_eq!(selected.compile(&typed).unwrap(), bytes, "{}", case.name);
        }
        assert_eq!(*Compiler::new().options(), CompilerOptions::default());
    }
}

fn execute(mode: &str, levels: &[OptimizationLevel]) {
    let mut records = Vec::new();
    for &level in levels {
        for case in corpus::cases() {
            let bytes = corpus::compiler(level)
                .compile_source(&case.source)
                .unwrap();
            records.push(format!(
                r#"["{}","{level:?}",{:?},{bytes:?}]"#,
                case.name,
                case.source.as_bytes()
            ));
        }
    }
    let input = format!("[{}]", records.join(","));
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/support/integer_measure.cjs"
    );
    let output = test_process::output(
        std::process::Command::new("node").args([script, mode]),
        Some(input.as_bytes()),
        std::time::Duration::from_secs(60),
    );
    assert!(output.status.success(), "{output:?}");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(String::from_utf8_lossy(&output.stdout).contains("integer workloads passed"));
}

#[test]
#[ignore = "requires Node.js 24+ on PATH"]
fn execute_integer_workloads() {
    execute(
        "verify",
        &[
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ],
    );
}

#[test]
#[ignore = "manual bounded baseline; requires Node.js 24+"]
fn record_integer_guest_timing() {
    execute("timing", &[corpus::measurement_level()]);
}
