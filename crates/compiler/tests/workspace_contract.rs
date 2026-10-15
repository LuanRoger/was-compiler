#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::Path, process::Command, time::Duration};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[test]
fn core_production_dependency_graph_is_adapter_free() {
    let output = test_process::output(
        Command::new(env!("CARGO")).current_dir(ROOT).args([
            "tree",
            "--offline",
            "-p",
            "was-compiler",
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ]),
        None,
        Duration::from_secs(30),
    );
    assert!(output.status.success(), "{output:?}");
    let graph = String::from_utf8(output.stdout).unwrap();
    let packages: Vec<_> = graph
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    for required in ["was-compiler", "wasm-encoder", "thiserror"] {
        assert!(packages.contains(&required), "missing {required}: {graph}");
    }
    for adapter_dependency in [
        "was-compiler-cli",
        "was-compiler-browser",
        "clap",
        "anyhow",
        "wasmprinter",
        "wasm-bindgen",
        "js-sys",
        "divan",
    ] {
        assert!(
            !packages.contains(&adapter_dependency),
            "production dependency {adapter_dependency}: {graph}"
        );
    }
}

#[test]
fn standalone_downstream_consumer_builds_and_uses_public_api() {
    // A separate target directory avoids waiting on the parent test's Cargo lock.
    let target = Path::new(ROOT).join("target/downstream-contract");
    let output = test_process::output(
        Command::new(env!("CARGO"))
            .current_dir(ROOT)
            .args([
                "run",
                "--offline",
                "--locked",
                "--manifest-path",
                "tests/fixtures/downstream/Cargo.toml",
                "--target-dir",
            ])
            .arg(target),
        None,
        Duration::from_secs(120),
    );
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("downstream compiler API and owned host snapshot passed")
    );
}

#[test]
fn shared_compatibility_assets_resolve_outside_package_directory() {
    let root = Path::new(ROOT).canonicalize().unwrap();
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    assert_ne!(root, package);
    for asset in [
        "tests/fixtures/abi/memory.was",
        "tests/fixtures/abi/memory.wasm",
        "tests/support/process.rs",
        "tests/browser.cjs",
        "runtime/run.cjs",
    ] {
        assert!(root.join(asset).is_file(), "missing shared asset {asset}");
    }
}
