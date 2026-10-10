#[path = "support/process.rs"]
mod test_process;

use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

const NODE_TIMEOUT: Duration = Duration::from_secs(30);

fn node_output(command: &mut Command) -> Output {
    test_process::output(command, None, NODE_TIMEOUT)
}

#[test]
#[ignore = "requires Node.js with ESM support and a built browser package in pkg"]
fn node_browser_compiler() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let package = root.join("pkg");
    for name in ["was_compiler.js", "was_compiler_bg.wasm"] {
        assert!(
            package.join(name).is_file(),
            "missing browser package file {}; build the browser package before running this ignored test",
            package.join(name).display()
        );
    }
    let output = node_output(
        Command::new("node")
            .arg(root.join("tests/browser.cjs"))
            .arg(package)
            .current_dir(root),
    );
    assert!(
        output.status.success(),
        "browser compiler tests failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("browser compiler: 18 tests passed"),
        "missing browser test completion summary: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
