use std::{
    any::Any,
    io::{Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[path = "../../../tests/support/process.rs"]
mod test_process;

const TIMEOUT: Duration = Duration::from_secs(10);

fn fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "subprocess_fixture", "--nocapture"])
        .env("WAS_COMPILER_PROCESS_FIXTURE", mode);
    command
}

fn panic_message(error: Box<dyn Any + Send>) -> String {
    if let Some(message) = error.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = error.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        panic!("unexpected panic payload");
    }
}

#[test]
fn subprocess_fixture() {
    let Ok(mode) = std::env::var("WAS_COMPILER_PROCESS_FIXTURE") else {
        return;
    };

    match mode.as_str() {
        "pipes" => {
            std::io::stderr()
                .write_all(&vec![b'e'; 256 * 1024])
                .unwrap();
            std::io::stdout()
                .write_all(&vec![b'o'; 256 * 1024])
                .unwrap();
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            assert_eq!(input, vec![b'i'; 256 * 1024]);
            println!("input received");
        }

        "eof" => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            println!("stdin bytes: {}", input.len());
        }

        "hang" => {
            println!("stdout before timeout");
            eprintln!("stderr before timeout");
            std::io::stdout().flush().unwrap();
            std::io::stderr().flush().unwrap();
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }

        "exit" => {
            println!("stdout before exit");
            eprintln!("stderr before exit");
            std::process::exit(7);
        }

        _ => panic!("unknown fixture {mode}"),
    }
}

#[test]
fn drains_both_output_pipes_while_writing_stdin() {
    let output = test_process::output(
        &mut fixture("pipes"),
        Some(&vec![b'i'; 256 * 1024]),
        TIMEOUT,
    );

    assert!(output.status.success());
    assert!(
        output
            .stdout
            .windows(256 * 1024)
            .any(|bytes| bytes.iter().all(|byte| *byte == b'o'))
    );
    assert_eq!(output.stderr, vec![b'e'; 256 * 1024]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("input received"));
}

#[test]
fn absent_input_is_immediate_eof() {
    let output = test_process::output(&mut fixture("eof"), None, TIMEOUT);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("stdin bytes: 0"));
    assert!(output.stderr.is_empty());
}

#[test]
fn preserves_nonzero_status_and_both_output_streams() {
    let output = test_process::output(&mut fixture("exit"), None, TIMEOUT);

    assert_eq!(output.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&output.stdout).contains("stdout before exit"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("stderr before exit"));
}

#[test]
fn early_exit_does_not_block_stdin_writer_or_hide_status() {
    let output = test_process::output(
        &mut fixture("exit"),
        Some(&vec![b'i'; 4 * 1024 * 1024]),
        TIMEOUT,
    );

    assert_eq!(output.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&output.stderr).contains("stderr before exit"));
}

#[test]
fn timeout_reports_both_streams_and_finishes_cleanup() {
    let start = Instant::now();
    let error = catch_unwind(AssertUnwindSafe(|| {
        test_process::output(&mut fixture("hang"), None, Duration::from_secs(1));
    }))
    .expect_err("hanging child must time out");
    let message = panic_message(error);

    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(message.contains("exceeded 1s"));
    assert!(message.contains("stdout before timeout"));
    assert!(message.contains("stderr before timeout"));
}

#[test]
fn timeout_unblocks_a_writer_when_child_does_not_read_stdin() {
    let start = Instant::now();
    let error = catch_unwind(AssertUnwindSafe(|| {
        test_process::output(
            &mut fixture("hang"),
            Some(&vec![b'i'; 4 * 1024 * 1024]),
            Duration::from_secs(1),
        );
    }))
    .expect_err("blocked writer must time out with its child");

    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(panic_message(error).contains("exceeded 1s"));
}

#[test]
fn held_open_stdin_is_not_closed_before_child_exits() {
    let error = catch_unwind(AssertUnwindSafe(|| {
        test_process::output_with_stdin(&mut fixture("eof"), None, true, Duration::from_secs(1));
    }))
    .expect_err("child waiting for EOF must time out while stdin is held open");

    assert!(panic_message(error).contains("exceeded 1s"));
}

#[test]
fn spawn_errors_identify_the_command() {
    let error = catch_unwind(AssertUnwindSafe(|| {
        test_process::output(
            &mut Command::new("was-compiler-nonexistent-subprocess-fixture"),
            None,
            TIMEOUT,
        );
    }))
    .expect_err("missing command must report a spawn error");
    let message = panic_message(error);

    assert!(message.contains("start "));
    assert!(message.contains("was-compiler-nonexistent-subprocess-fixture"));
}
