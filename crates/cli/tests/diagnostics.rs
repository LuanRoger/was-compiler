use std::{path::Path, process::Command, time::Duration};

#[path = "../../../tests/support/process.rs"]
mod test_process;

const FIXTURES: [(&str, &str, &str); 4] = [
    (
        "parse.was",
        include_str!("../../../tests/fixtures/diagnostics/parse.was"),
        include_str!("../../../tests/fixtures/diagnostics/parse.stderr"),
    ),
    (
        "semantic.was",
        include_str!("../../../tests/fixtures/diagnostics/semantic.was"),
        include_str!("../../../tests/fixtures/diagnostics/semantic.stderr"),
    ),
    (
        "codegen.was",
        include_str!("../../../tests/fixtures/diagnostics/codegen.was"),
        include_str!("../../../tests/fixtures/diagnostics/codegen.stderr"),
    ),
    (
        "unlocated.was",
        include_str!("../../../tests/fixtures/diagnostics/unlocated.was"),
        include_str!("../../../tests/fixtures/diagnostics/unlocated.stderr"),
    ),
];

#[test]
fn cli_diagnostics_match_pre_refactor_text_and_exit_status_exactly() {
    let directory =
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("tests/fixtures/diagnostics");
    for (name, _, expected) in FIXTURES {
        let mut command = Command::new(env!("CARGO_BIN_EXE_was-compiler"));
        command.current_dir(&directory).arg(name);
        if name == "unlocated.was" {
            command.args(["--export", "missing"]);
        }
        let output = test_process::output(&mut command, None, Duration::from_secs(15));

        assert_eq!(output.status.code(), Some(1), "fixture {name}: {output:?}");
        assert!(output.stdout.is_empty(), "fixture {name}: {output:?}");
        assert_eq!(output.stderr, expected.as_bytes(), "fixture {name}");
    }
}
