use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use was_compiler::{Compiler, CompilerOptions, OptimizationLevel, Type};

#[path = "../../../tests/support/process.rs"]
mod test_process;

const SOURCE: &str = include_str!("../../../tests/fixtures/abi/memory.was");
const GOLDEN: &[u8] = include_bytes!("../../../tests/fixtures/abi/memory.wasm");

fn compiler(level: OptimizationLevel) -> Compiler {
    let mut compiler = Compiler::with_options(CompilerOptions {
        optimization_level: level,
    });

    for (name, ty) in [
        (
            "echo_array",
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
        ("echo_string", Type::String),
        ("echo_bool", Type::Bool),
    ] {
        compiler
            .register_host_function(name, "env", name, vec![ty], ty)
            .unwrap();
    }

    compiler
}

#[test]
fn cli_and_library_match_the_pre_refactor_memory_fixture() {
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "was-compiler-abi-{}-{timestamp}",
        std::process::id()
    )));
    fs::create_dir(&scratch.0).unwrap();
    fs::write(scratch.0.join("memory.was"), SOURCE).unwrap();
    let output = test_process::output(
        Command::new(env!("CARGO_BIN_EXE_was-compiler"))
            .current_dir(&scratch.0)
            .args([
                "memory.was",
                "-o",
                "memory.wasm",
                "--host",
                "echo_array=env.echo_array(number[])->number[]",
                "--host",
                "echo_string=env.echo_string(string)->string",
                "--host",
                "echo_bool=env.echo_bool(bool)->bool",
            ]),
        None,
        Duration::from_secs(15),
    );

    assert!(output.status.success(), "CLI failed: {output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let bytes = fs::read(scratch.0.join("memory.wasm")).unwrap();
    assert_eq!(bytes, GOLDEN);
    assert_eq!(
        bytes,
        compiler(OptimizationLevel::None)
            .compile_source(SOURCE)
            .unwrap()
    );
}
