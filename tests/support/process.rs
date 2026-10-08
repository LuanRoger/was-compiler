use std::{
    io::{self, Read, Write},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn output(command: &mut Command, input: Option<&[u8]>, timeout: Duration) -> Output {
    output_with_stdin(command, input, false, timeout)
}

pub fn output_with_stdin(
    command: &mut Command,
    input: Option<&[u8]>,
    keep_open: bool,
    timeout: Duration,
) -> Output {
    assert!(
        !(input.is_some() && keep_open),
        "stdin cannot be written and held open"
    );

    let description = format!("{command:?}");
    let mut child = ChildGuard(
        command
            .stdin(if input.is_some() || keep_open {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("start {description}: {error}")),
    );

    let open_stdin = if keep_open {
        child.0.stdin.take()
    } else {
        None
    };
    let mut stdout = child.0.stdout.take().expect("subprocess stdout");
    let mut stderr = child.0.stderr.take().expect("subprocess stderr");
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .expect("read subprocess stdout");
        bytes
    });
    let stderr = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .read_to_end(&mut bytes)
            .expect("read subprocess stderr");
        bytes
    });
    let stdin = input.map(|bytes| {
        let bytes = bytes.to_vec();
        let mut stdin = child.0.stdin.take().expect("subprocess stdin");
        thread::spawn(move || stdin.write_all(&bytes))
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("poll subprocess") {
            break status;
        }

        if Instant::now() >= deadline {
            child.0.kill().expect("kill timed out subprocess");
            timed_out = true;
            break child.0.wait().expect("reap timed out subprocess");
        }

        thread::sleep(Duration::from_millis(10));
    };

    drop(open_stdin);

    let output = Output {
        status,
        stdout: stdout.join().expect("join stdout reader"),
        stderr: stderr.join().expect("join stderr reader"),
    };

    if let Some(stdin) = stdin
        && let Err(error) = stdin.join().expect("join stdin writer")
    {
        assert_eq!(
            error.kind(),
            io::ErrorKind::BrokenPipe,
            "write subprocess stdin: {error}"
        );
    }

    assert!(
        !timed_out,
        "{description} exceeded {timeout:?}:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    output
}
