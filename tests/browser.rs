use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const NODE_TIMEOUT: Duration = Duration::from_secs(30);

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn node_output(command: &mut Command) -> Output {
    let mut child = ChildGuard(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start Node.js; ignored browser test requires node on PATH"),
    );
    let mut stdout = child.0.stdout.take().unwrap();
    let mut stderr = child.0.stderr.take().unwrap();
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let stderr = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let deadline = Instant::now() + NODE_TIMEOUT;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("poll Node.js") {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().expect("kill timed out Node.js");
            timed_out = true;
            break child.0.wait().expect("wait for timed out Node.js");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    };
    assert!(
        !timed_out,
        "Node.js exceeded {NODE_TIMEOUT:?}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
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
        String::from_utf8_lossy(&output.stdout).contains("browser compiler: 9 tests passed"),
        "missing browser test completion summary: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
