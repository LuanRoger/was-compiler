use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use was_compiler::{Compiler, ExportOptions};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef, Validator};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
const INTERFACE_SECTION: &str = "was-compiler.interface";
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

const SIMPLE_SOURCE: &str = "fn main() -> number { return 7; }";
const EXAMPLE_SOURCE: &str = r#"
fn main() -> number {
    let values: number[] = [10, 20, 30];
    values[1] = 25;
    return sum(values);
}
fn sum(values: number[]) -> number {
    let total: number = 0;
    let index: number = 0;
    while index < len(values) {
        total = total + values[index];
        index = index + 1;
    }
    return total;
}
"#;
const MEMORY_SOURCE: &str = r#"
fn hidden() -> string { return "hidden"; }
fn main(values: number[]) -> number { return helper(values); }
fn helper(values: number[]) -> number { return len(values); }
fn enabled(value: bool) -> bool { return !value; }
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "was-compiler-cli-{name}-{}-{timestamp}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create CLI test directory");
        Self(path)
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).expect("write CLI fixture");
        path
    }

    fn entries(&self) -> Vec<PathBuf> {
        let mut entries = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| PathBuf::from(entry.unwrap().file_name()))
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    fn assert_entries(&self, names: &[&str]) {
        let mut expected = names.iter().map(PathBuf::from).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(
            self.entries(),
            expected,
            "unexpected files or leaked temporaries"
        );
    }

    fn cli(&self, args: &[&str], stdin: Option<&[u8]>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_was-compiler"));
        command.current_dir(&self.0).args(args);
        process_output(&mut command, stdin)
    }

    fn compile(&self, source: &str, flags: &[&str]) -> Vec<u8> {
        self.write("program.was", source);
        let mut args = vec!["program.was", "-o", "program.wasm"];
        args.extend_from_slice(flags);
        assert!(success(self.cli(&args, None)).is_empty());
        let bytes = fs::read(self.0.join("program.wasm")).unwrap();
        decode(&bytes);
        bytes
    }

    fn node(&self, args: &[&str]) -> Output {
        let mut command = Command::new("node");
        command
            .current_dir(&self.0)
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/run.cjs"))
            .args(args);
        process_output(&mut command, None)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn process_output(command: &mut Command, input: Option<&[u8]>) -> Output {
    let description = format!("{command:?}");
    let mut child = ChildGuard(
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("start {description}: {error}")),
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
    let stdin = input.map(|bytes| {
        let bytes = bytes.to_vec();
        let mut stdin = child.0.stdin.take().unwrap();
        thread::spawn(move || stdin.write_all(&bytes))
    });
    let deadline = Instant::now() + PROCESS_TIMEOUT;
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
    let output = Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    };
    if let Some(stdin) = stdin
        && let Err(error) = stdin.join().unwrap()
    {
        assert_eq!(
            error.kind(),
            io::ErrorKind::BrokenPipe,
            "write subprocess stdin: {error}"
        );
    }
    assert!(
        !timed_out,
        "{description} exceeded {PROCESS_TIMEOUT:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn success(output: Output) -> Vec<u8> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn text(output: Output) -> String {
    String::from_utf8(success(output))
        .unwrap()
        .replace("\r\n", "\n")
}

fn failure(output: Output, code: i32, fragments: &[&str]) -> String {
    assert_eq!(
        output.status.code(),
        Some(code),
        "wrong exit status: {output:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty on failure: {output:?}"
    );
    let stderr = String::from_utf8(output.stderr)
        .unwrap()
        .replace("\r\n", "\n");
    for fragment in fragments {
        assert!(
            stderr.contains(fragment),
            "expected {fragment:?} in {stderr:?}"
        );
    }
    stderr
}

fn raw_wasm(source: &str) -> Vec<u8> {
    Compiler::new().compile_source(source).unwrap()
}

fn raw_wat(source: &str) -> String {
    wasmprinter::print_bytes(raw_wasm(source)).unwrap()
}

fn assert_plain_wat(bytes: &[u8]) -> &str {
    let wat = std::str::from_utf8(bytes).expect("UTF-8 WAT");
    assert!(
        wat.starts_with("(module"),
        "expected plain WAT, got {wat:?}"
    );
    assert!(wat.trim_end().ends_with(')'));
    assert!(!wat.contains(INTERFACE_SECTION));
    assert!(!wat.contains("memoryAbi"));
    wat
}

#[derive(Debug, Default)]
struct Decoded {
    sections: Vec<(u8, Vec<u8>)>,
    custom: Vec<(String, Vec<u8>)>,
    exports: Vec<(String, ExternalKind, u32)>,
    imports: Vec<(String, String)>,
    functions: Vec<u32>,
    code_count: u32,
    bodies: Vec<Vec<u8>>,
}

fn decode(bytes: &[u8]) -> Decoded {
    assert!(
        bytes.starts_with(b"\0asm\x01\0\0\0"),
        "missing Wasm magic/version"
    );
    Validator::new()
        .validate_all(bytes)
        .expect("valid Wasm artifact");
    let mut module = Decoded::default();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("Wasm payload");
        if let Some((id, range)) = payload.as_section()
            && id != 0
            && id != 7
        {
            let start = usize::try_from(range.start).expect("section start");
            let end = usize::try_from(range.end).expect("section end");
            module.sections.push((id, bytes[start..end].to_vec()));
        }
        match payload {
            Payload::CustomSection(section) => {
                module
                    .custom
                    .push((section.name().to_owned(), section.data().to_vec()));
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.unwrap();
                    module
                        .exports
                        .push((export.name.to_owned(), export.kind, export.index));
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.unwrap();
                    assert!(matches!(import.ty, TypeRef::Func(_)));
                    module
                        .imports
                        .push((import.module.to_owned(), import.name.to_owned()));
                }
            }
            Payload::FunctionSection(reader) => {
                module.functions = reader.into_iter().map(Result::unwrap).collect();
            }
            Payload::CodeSectionStart { count, .. } => module.code_count = count,
            Payload::CodeSectionEntry(body) => {
                let range = body.range();
                let start = usize::try_from(range.start).expect("body start");
                let end = usize::try_from(range.end).expect("body end");
                module.bodies.push(bytes[start..end].to_vec());
            }
            _ => {}
        }
    }
    assert_eq!(module.code_count as usize, module.functions.len());
    assert_eq!(module.bodies.len(), module.functions.len());
    module
}

fn assert_metadata(module: &Decoded, exports: &[&str], imports: &[&str]) {
    let expected = format!(
        r#"{{"version":1,"memoryAbi":"descriptor-v1","exports":[{}],"imports":[{}]}}"#,
        exports.join(","),
        imports.join(",")
    );
    assert_eq!(
        module.custom,
        vec![(INTERFACE_SECTION.to_owned(), expected.into_bytes())]
    );
}

fn user_exports(module: &Decoded) -> Vec<&str> {
    module
        .exports
        .iter()
        .filter_map(|(name, kind, _)| {
            if name == "memory" || name == "__alloc" {
                None
            } else {
                assert_eq!(*kind, ExternalKind::Func);
                Some(name.as_str())
            }
        })
        .collect()
}

fn abi_exports(module: &Decoded) -> Vec<(String, ExternalKind, u32)> {
    module
        .exports
        .iter()
        .filter(|(name, _, _)| name == "memory" || name == "__alloc")
        .cloned()
        .collect()
}

#[test]
fn no_arguments_prints_the_plain_array_example() {
    let scratch = Scratch::new("no_arguments");
    let bytes = success(scratch.cli(&[], None));
    let wat = assert_plain_wat(&bytes);
    assert_eq!(wat, raw_wat(EXAMPLE_SOURCE));
    assert!(wat.contains("(export \"main\""));
    assert!(wat.contains("(export \"sum\""));
    assert!(wat.contains("(export \"memory\""));
    scratch.assert_entries(&[]);
}

#[test]
fn help_flags_print_usage_without_compiling_or_writing() {
    let scratch = Scratch::new("help");
    for flag in ["-h", "--help"] {
        let help = text(scratch.cli(&["missing.was", "-o", "program.wasm", flag], None));
        assert!(help.contains("Usage: was-compiler [OPTIONS] [SOURCE]"));
        for option in [
            "--output",
            "--emit",
            "--export",
            "--no-exports",
            "--host",
            "-O0",
            "-O3",
            "--help",
        ] {
            assert!(help.contains(option), "help omits {option}: {help}");
        }
        assert!(help.contains("print=env.log_number(number)->void"));
        assert!(help.contains("node runtime/run.cjs"));
        scratch.assert_entries(&[]);
    }
}

#[test]
fn invalid_arguments_exit_two_without_artifacts() {
    let scratch = Scratch::new("usage_errors");
    let cases: &[(&[&str], &[&str])] = &[
        (&["--unknown"], &["unexpected argument '--unknown'"]),
        (&["-O4"], &["invalid value '4'", "-O <LEVEL>", "0..=3"]),
        (&["-O"], &["a value is required", "-O <LEVEL>"]),
        (&["-O", "fast"], &["invalid value 'fast'", "-O <LEVEL>"]),
        (&["-o"], &["a value is required", "--output <PATH>"]),
        (&["--output"], &["a value is required", "--output <PATH>"]),
        (&["--output="], &["--output <PATH>", "path cannot be empty"]),
        (&["--emit"], &["a value is required", "--emit <FORMAT>"]),
        (&["--export"], &["a value is required", "--export <NAME>"]),
        (&["--host"], &["a value is required", "--host <DECL>"]),
        (
            &["--host", "--export", "main"],
            &["a value is required", "--host <DECL>"],
        ),
        (
            &["--emit", "json"],
            &["invalid value 'json'", "--emit <FORMAT>", "wasm, wat"],
        ),
        (
            &["--emit", "wat", "--emit", "wat"],
            &["--emit <FORMAT>", "cannot be used multiple times"],
        ),
        (
            &["first.was", "second.was"],
            &["unexpected argument 'second.was'"],
        ),
        (&[""], &["[SOURCE]", "path cannot be empty"]),
        (
            &["--export", ""],
            &["a value is required", "--export <NAME>"],
        ),
        (&["--host="], &["a value is required", "--host <DECL>"]),
        (
            &["--no-exports", "--export", "main"],
            &["--no-exports", "cannot be used with", "--export <NAME>"],
        ),
        (
            &["--export", "main", "--no-exports"],
            &["--export <NAME>", "cannot be used with", "--no-exports"],
        ),
        (
            &["--no-exports", "--no-exports"],
            &["--no-exports", "cannot be used multiple times"],
        ),
        (
            &["-O0", "-O3"],
            &["-O <LEVEL>", "cannot be used multiple times"],
        ),
        (
            &["-o", "first.wasm", "--output", "second.wasm"],
            &["--output <PATH>", "cannot be used multiple times"],
        ),
        (&["--emit", "wasm"], &["binary Wasm requires -o"]),
        (&["--emit", "wasm", "-o", "-"], &["binary Wasm requires -o"]),
    ];
    for (args, details) in cases {
        let stderr = failure(scratch.cli(args, None), 2, details);

        assert!(stderr.starts_with("error:"));
        assert!(stderr.contains("--help"));
        scratch.assert_entries(&[]);
    }
}

#[test]
fn file_and_stdin_sources_have_identical_plain_wat_stdout() {
    let scratch = Scratch::new("file_stdin");
    scratch.write("source with spaces.was", SIMPLE_SOURCE);
    let expected = raw_wat(SIMPLE_SOURCE);
    for args in [
        vec!["source with spaces.was"],
        vec!["source with spaces.was", "--emit", "wat"],
        vec!["source with spaces.was", "-o", "-"],
        vec!["source with spaces.was", "--output", "-", "--emit", "wat"],
    ] {
        let bytes = success(scratch.cli(&args, None));
        assert_eq!(assert_plain_wat(&bytes), expected);
    }
    for args in [
        vec!["-"],
        vec!["-", "-o", "-"],
        vec!["--emit", "wat", "--output", "-", "-"],
    ] {
        let bytes = success(scratch.cli(&args, Some(SIMPLE_SOURCE.as_bytes())));
        assert_eq!(assert_plain_wat(&bytes), expected);
    }
    scratch.assert_entries(&["source with spaces.was"]);
}

#[test]
fn end_of_options_accepts_dash_prefixed_source_paths() {
    let scratch = Scratch::new("end_options");
    scratch.write("-source.was", SIMPLE_SOURCE);
    scratch.write("--help", SIMPLE_SOURCE);
    for args in [
        vec!["--", "-source.was"],
        vec!["./-source.was"],
        vec!["--", "--help"],
    ] {
        assert_eq!(text(scratch.cli(&args, None)), raw_wat(SIMPLE_SOURCE));
    }
    assert!(success(scratch.cli(&["-o", "program.wasm", "--", "-source.was"], None)).is_empty());
    assert_metadata(
        &decode(&fs::read(scratch.0.join("program.wasm")).unwrap()),
        &[r#"{"name":"main","params":[],"result":"number"}"#],
        &[],
    );
    failure(
        scratch.cli(&["--", "-source.was", "--help"], None),
        2,
        &["unexpected argument '--help'"],
    );
    scratch.assert_entries(&["-source.was", "--help", "program.wasm"]);
}

#[test]
fn output_formats_follow_extension_unless_explicitly_overridden() {
    let scratch = Scratch::new("formats");
    scratch.write("program.was", SIMPLE_SOURCE);
    let raw = raw_wasm(SIMPLE_SOURCE);
    let cases: &[(&str, &[&str], bool)] = &[
        ("default.wasm", &[], true),
        ("explicit.data", &["--emit", "wasm"], true),
        ("overridden.wat", &["--emit", "wasm"], true),
        ("default.wat", &[], false),
        ("default.data", &[], false),
        ("uppercase.WASM", &[], false),
        ("overridden.wasm", &["--emit", "wat"], false),
    ];
    for (index, (name, flags, binary)) in cases.iter().enumerate() {
        let output_flag = if index % 2 == 0 { "-o" } else { "--output" };
        let mut args = vec!["program.was", output_flag, name];
        args.extend_from_slice(flags);
        assert!(success(scratch.cli(&args, None)).is_empty());
        let bytes = fs::read(scratch.0.join(name)).unwrap();
        if *binary {
            assert!(
                bytes.starts_with(&raw),
                "packaging rewrote raw codegen bytes"
            );
            assert_metadata(
                &decode(&bytes),
                &[r#"{"name":"main","params":[],"result":"number"}"#],
                &[],
            );
        } else {
            assert_eq!(assert_plain_wat(&bytes), raw_wat(SIMPLE_SOURCE));
        }
    }
    let mut names = vec!["program.was"];
    names.extend(cases.iter().map(|(name, _, _)| *name));
    scratch.assert_entries(&names);
}

#[test]
fn stdin_can_write_a_valid_packaged_binary_without_stdout() {
    let scratch = Scratch::new("stdin_binary");
    assert!(
        success(scratch.cli(
            &["-", "--output", "stdin.wasm"],
            Some(SIMPLE_SOURCE.as_bytes())
        ))
        .is_empty()
    );
    let bytes = fs::read(scratch.0.join("stdin.wasm")).unwrap();
    assert_metadata(
        &decode(&bytes),
        &[r#"{"name":"main","params":[],"result":"number"}"#],
        &[],
    );
    scratch.assert_entries(&["stdin.wasm"]);
}

#[test]
fn library_codegen_is_raw_and_only_binary_cli_output_adds_metadata() {
    let scratch = Scratch::new("raw_library");
    let raw = raw_wasm(MEMORY_SOURCE);
    assert!(decode(&raw).custom.is_empty());
    let packaged = scratch.compile(MEMORY_SOURCE, &[]);
    assert!(packaged.starts_with(&raw));
    assert_eq!(decode(&packaged).custom.len(), 1);
    let wat = success(scratch.cli(&["program.was"], None));
    assert_eq!(
        assert_plain_wat(&wat),
        wasmprinter::print_bytes(&raw).unwrap()
    );
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn export_selection_preserves_source_order_bodies_and_memory_abi() {
    let scratch = Scratch::new("memory_exports");
    let all = decode(&scratch.compile(MEMORY_SOURCE, &[]));
    let only =
        decode(&scratch.compile(MEMORY_SOURCE, &["--export", "enabled", "--export", "main"]));
    let none = decode(&scratch.compile(MEMORY_SOURCE, &["--no-exports"]));
    assert_eq!(user_exports(&all), ["hidden", "main", "helper", "enabled"]);
    assert_eq!(user_exports(&only), ["main", "enabled"]);
    assert!(user_exports(&none).is_empty());
    let abi = abi_exports(&all);
    assert_eq!(abi.len(), 2);
    assert!(abi.iter().any(|(name, kind, index)| name == "memory"
        && *kind == ExternalKind::Memory
        && *index == 0));
    assert!(
        abi.iter()
            .any(|(name, kind, _)| name == "__alloc" && *kind == ExternalKind::Func)
    );
    assert!(all.code_count >= 4);
    for selected in [&only, &none] {
        assert_eq!(selected.sections, all.sections);
        assert_eq!(selected.functions, all.functions);
        assert_eq!(selected.bodies, all.bodies);
        assert_eq!(selected.code_count, all.code_count);
        assert_eq!(abi_exports(selected), abi);
        for export in &selected.exports {
            assert!(all.exports.contains(export));
        }
    }
    assert_metadata(
        &all,
        &[
            r#"{"name":"hidden","params":[],"result":"string"}"#,
            r#"{"name":"main","params":["number[]"],"result":"number"}"#,
            r#"{"name":"helper","params":["number[]"],"result":"number"}"#,
            r#"{"name":"enabled","params":["bool"],"result":"bool"}"#,
        ],
        &[],
    );
    assert_metadata(
        &only,
        &[
            r#"{"name":"main","params":["number[]"],"result":"number"}"#,
            r#"{"name":"enabled","params":["bool"],"result":"bool"}"#,
        ],
        &[],
    );
    assert_metadata(&none, &[], &[]);
    for (flags, selection) in [
        (
            vec!["--export", "enabled", "--export", "main"],
            ExportOptions::Only(vec!["enabled".into(), "main".into()]),
        ),
        (vec!["--no-exports"], ExportOptions::None),
    ] {
        let mut args = vec!["program.was", "--emit", "wat"];
        args.extend(flags);
        let mut compiler = Compiler::new();
        compiler.set_exports(selection);
        let expected =
            wasmprinter::print_bytes(compiler.compile_source(MEMORY_SOURCE).unwrap()).unwrap();
        let bytes = success(scratch.cli(&args, None));
        assert_eq!(assert_plain_wat(&bytes), expected);
    }
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn scalar_no_exports_retains_every_function_without_adding_memory() {
    let scratch = Scratch::new("scalar_exports");
    let source = "fn main() -> number { return helper(); } fn helper() -> number { return 65; } fn unused() -> void {}";
    let all = decode(&scratch.compile(source, &[]));
    let only = decode(&scratch.compile(source, &["--export", "main"]));
    let none = decode(&scratch.compile(source, &["--no-exports"]));
    assert_eq!(all.code_count, 3);
    assert_eq!(user_exports(&only), ["main"]);
    assert!(none.exports.is_empty());
    assert!(abi_exports(&all).is_empty());
    for selected in [&only, &none] {
        assert_eq!(selected.sections, all.sections);
        assert_eq!(selected.bodies, all.bodies);
        assert_eq!(selected.code_count, 3);
    }
    assert_metadata(
        &only,
        &[r#"{"name":"main","params":[],"result":"number"}"#],
        &[],
    );
    assert_metadata(&none, &[], &[]);
}

#[test]
fn unknown_and_duplicate_exports_are_unlocated_codegen_errors() {
    let scratch = Scratch::new("invalid_exports");
    scratch.write("program.was", MEMORY_SOURCE);
    for name in ["missing", "Main", " main", "memory", "__alloc"] {
        let expected = format!("unknown user function export `{name}`");
        let stderr = failure(
            scratch.cli(
                &["program.was", "-o", "program.wasm", "--export", name],
                None,
            ),
            1,
            &[&expected],
        );
        assert!(stderr.starts_with("program.was: code generation error:"));
        assert!(!stderr.contains('^'));
        scratch.assert_entries(&["program.was"]);
    }
    let stderr = failure(
        scratch.cli(
            &[
                "program.was",
                "-o",
                "program.wasm",
                "--export",
                "main",
                "--export",
                "main",
            ],
            None,
        ),
        1,
        &["duplicate requested export `main`"],
    );
    assert!(stderr.starts_with("program.was: code generation error:"));
    assert!(!stderr.contains('^'));
    failure(
        scratch.cli(
            &[
                "program.was",
                "--host",
                "clock=env.now()->number",
                "--export",
                "clock",
            ],
            None,
        ),
        1,
        &["unknown user function export `clock`"],
    );
    scratch.assert_entries(&["program.was"]);
}

#[test]
fn export_selection_still_compiles_invalid_unexported_bodies() {
    let scratch = Scratch::new("invalid_hidden_body");
    scratch.write(
        "program.was",
        "fn main() -> number { return 7; }\nfn hidden() -> number { return 1 % 2; }",
    );
    for flags in [vec!["--export", "main"], vec!["--no-exports"]] {
        let mut args = vec!["program.was", "-o", "program.wasm"];
        args.extend(flags);
        let stderr = failure(
            scratch.cli(&args, None),
            1,
            &[
                "code generation error:",
                "unsupported operator `%`",
                "fn hidden()",
                "^",
            ],
        );
        assert!(stderr.starts_with("program.was:2:"));
        scratch.assert_entries(&["program.was"]);
    }
}

#[test]
fn attached_option_values_and_separated_levels_preserve_artifacts() {
    let scratch = Scratch::new("attached_options");
    scratch.write(
        "program.was",
        "fn main() -> number { print(7); return helper(); } fn helper() -> number { return 7; }",
    );

    for args in [
        vec![
            "program.was",
            "--output=attached.wasm",
            "--emit=wasm",
            "--export=main",
            "--export=helper",
            "--host=print=env.log_number(number)->void",
            "--host=ready=app.ready()->bool",
            "-O",
            "3",
        ],
        vec![
            "program.was",
            "--output",
            "separated.wasm",
            "--emit",
            "wasm",
            "--export",
            "main",
            "--export",
            "helper",
            "--host",
            "print=env.log_number(number)->void",
            "--host",
            "ready=app.ready()->bool",
            "-O3",
        ],
    ] {
        assert!(success(scratch.cli(&args, None)).is_empty());
    }

    let bytes = fs::read(scratch.0.join("attached.wasm")).unwrap();

    assert_eq!(bytes, fs::read(scratch.0.join("separated.wasm")).unwrap());

    let module = decode(&bytes);

    assert_eq!(user_exports(&module), ["main", "helper"]);
    assert_metadata(
        &module,
        &[
            r#"{"name":"main","params":[],"result":"number"}"#,
            r#"{"name":"helper","params":[],"result":"number"}"#,
        ],
        &[
            r#"{"name":"print","module":"env","field":"log_number","params":["number"],"result":"void"}"#,
            r#"{"name":"ready","module":"app","field":"ready","params":[],"result":"bool"}"#,
        ],
    );
    scratch.assert_entries(&["program.was", "attached.wasm", "separated.wasm"]);
}

#[test]
fn every_optimization_preset_is_currently_a_noop() {
    let scratch = Scratch::new("optimization");
    let baseline = scratch.compile(MEMORY_SOURCE, &["--export", "main"]);
    for level in ["-O0", "-O1", "-O2", "-O3"] {
        assert_eq!(
            scratch.compile(MEMORY_SOURCE, &[level, "--export", "main"]),
            baseline
        );
        let bytes = success(scratch.cli(&["program.was", level, "--emit", "wat"], None));
        assert_eq!(assert_plain_wat(&bytes), raw_wat(MEMORY_SOURCE));
    }
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn located_diagnostics_show_absolute_file_paths_phases_and_source_carets() {
    let scratch = Scratch::new("diagnostics");
    for (source, stage, snippet, column) in [
        (
            "fn main() -> void {\n    let n: number = ;\n}",
            "parse",
            "    let n: number = ;",
            21,
        ),
        (
            "fn main() -> void {\n    missing();\n}",
            "semantic",
            "    missing();",
            5,
        ),
        (
            "fn main() -> number {\n    return 1 % 2;\n}",
            "code generation",
            "    return 1 % 2;",
            5,
        ),
    ] {
        let path = scratch.write("broken source.was", source);
        let location = format!("{}:2:{column}: {stage} error:", path.display());
        let stderr = failure(
            scratch.cli(&[path.to_str().unwrap(), "-o", "program.wasm"], None),
            1,
            &[
                &location,
                &format!("2 | {snippet}"),
                &format!(" | {}^", " ".repeat(column - 1)),
            ],
        );
        assert!(stderr.starts_with(&location));
        scratch.assert_entries(&["broken source.was"]);
    }
}

#[test]
fn stdin_diagnostics_and_unicode_crlf_tabs_have_correct_locations() {
    let scratch = Scratch::new("stdin_diagnostics");
    let source = "fn main() -> void {\r\n\tlet text: string = \"é🦀\"; missing();\r\n}\r\n";
    let line = source.lines().nth(1).unwrap();
    let prefix = &line[..line.find("missing").unwrap()];
    let column = prefix.chars().count() + 1;
    let location = format!("<stdin>:2:{column}: semantic error:");
    let stderr = failure(
        scratch.cli(&["-"], Some(source.as_bytes())),
        1,
        &[
            &location,
            &format!("2 | {}", line.replace('\t', "    ")),
            &format!(
                " | {}^",
                " ".repeat(prefix.replace('\t', "    ").chars().count())
            ),
        ],
    );
    assert!(stderr.starts_with(&location));
    assert!(!stderr.contains('\r'));
    let stderr = failure(
        scratch.cli(&["-"], Some(b"fn main() -> void {\n")),
        1,
        &["<stdin>:2:1: parse error:", "2 |", "^"],
    );
    assert!(stderr.starts_with("<stdin>:2:1:"));
    scratch.assert_entries(&[]);
}

#[test]
fn source_io_errors_are_exit_one_and_label_the_input() {
    let scratch = Scratch::new("input_io");
    let missing = scratch.0.join("missing source.was");
    let location = format!("{}: input I/O error:", missing.display());
    failure(
        scratch.cli(&[missing.to_str().unwrap()], None),
        1,
        &[&location],
    );
    scratch.write("invalid.was", [0xff, 0xfe]);
    failure(
        scratch.cli(&["invalid.was"], None),
        1,
        &["invalid.was: input I/O error:"],
    );
    failure(
        scratch.cli(&["-"], Some(&[0xff, 0xfe])),
        1,
        &["<stdin>: input I/O error:"],
    );
    fs::create_dir(scratch.0.join("directory.was")).unwrap();
    failure(
        scratch.cli(&["directory.was"], None),
        1,
        &["directory.was: input I/O error:"],
    );
    scratch.assert_entries(&["invalid.was", "directory.was"]);
}

#[test]
fn failed_compilation_never_replaces_or_creates_outputs() {
    let scratch = Scratch::new("preserve_outputs");
    let output = scratch.0.join("program.wasm");
    let sentinel = b"existing artifact must remain byte-for-byte intact";
    let cases: &[(&str, &[&str], &str)] = &[
        (SIMPLE_SOURCE, &["--host", "bad"], "host registration error"),
        (
            "fn main() -> void { let n: number = ; }",
            &[],
            "parse error:",
        ),
        ("fn main() -> void { missing(); }", &[], "semantic error:"),
        (
            "fn main() -> number { return 1 % 2; }",
            &[],
            "code generation error:",
        ),
        (
            SIMPLE_SOURCE,
            &["--export", "missing"],
            "unknown user function export",
        ),
        (
            SIMPLE_SOURCE,
            &["--export", "main", "--export", "main"],
            "duplicate requested export",
        ),
        (
            "fn memory() -> number { return 1; }",
            &[],
            "packaging error:",
        ),
    ];
    for existing in [false, true] {
        if existing {
            fs::write(&output, sentinel).unwrap();
        }
        for (source, flags, detail) in cases {
            let input = scratch.write("program.was", source);
            let before = scratch.entries();
            let mut args = vec!["program.was", "-o", "program.wasm"];
            args.extend_from_slice(flags);
            failure(scratch.cli(&args, None), 1, &[detail]);
            assert_eq!(fs::read(&input).unwrap(), source.as_bytes());
            if existing {
                assert_eq!(fs::read(&output).unwrap(), sentinel);
            } else {
                assert!(!output.exists());
            }
            assert_eq!(
                scratch.entries(),
                before,
                "failed compilation leaked temporary files"
            );
        }
    }
}

#[test]
fn packaging_errors_do_not_affect_plain_wat_or_unselected_reserved_names() {
    let scratch = Scratch::new("packaging");
    for name in ["memory", "__alloc"] {
        let source = format!("fn {name}() -> number {{ return 7; }}");
        scratch.write("program.was", &source);
        let stderr = failure(
            scratch.cli(&["program.was", "-o", "program.wasm"], None),
            1,
            &[
                "packaging error:",
                &format!("export `{name}` is reserved"),
                "emit raw WAT",
            ],
        );
        assert!(stderr.starts_with("program.was: packaging error:"));
        assert!(!stderr.contains('^'));
        assert!(!scratch.0.join("program.wasm").exists());
        let wat = success(scratch.cli(&["program.was", "--emit", "wat"], None));
        assert_eq!(assert_plain_wat(&wat), raw_wat(&source));
        let bytes = scratch.compile(&source, &["--no-exports"]);
        assert_metadata(&decode(&bytes), &[], &[]);
        fs::remove_file(scratch.0.join("program.wasm")).unwrap();
        scratch.assert_entries(&["program.was"]);
    }
}

#[test]
fn successful_compilation_overwrites_a_valid_artifact_without_leaking_files() {
    let scratch = Scratch::new("overwrite");
    let original = scratch.compile(SIMPLE_SOURCE, &[]);
    let source = "fn main() -> number { return 65; }";
    let replacement = scratch.compile(source, &[]);
    assert_ne!(original, replacement);
    assert!(replacement.starts_with(&raw_wasm(source)));
    assert_metadata(
        &decode(&replacement),
        &[r#"{"name":"main","params":[],"result":"number"}"#],
        &[],
    );
    assert_eq!(
        fs::read(scratch.0.join("program.was")).unwrap(),
        source.as_bytes()
    );
    assert!(
        success(scratch.cli(
            &["program.was", "--output", "program.wasm", "--emit", "wat"],
            None
        ))
        .is_empty()
    );
    assert_eq!(
        assert_plain_wat(&fs::read(scratch.0.join("program.wasm")).unwrap()),
        raw_wat(source)
    );
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn output_canonicalizing_to_input_is_refused_without_source_damage() {
    let scratch = Scratch::new("same_output");
    let input = scratch.write("program.was", SIMPLE_SOURCE);
    fs::create_dir(scratch.0.join("nested")).unwrap();
    for output in ["program.was", "./program.was", "nested/../program.was"] {
        for format in ["wasm", "wat"] {
            failure(
                scratch.cli(
                    &[input.to_str().unwrap(), "-o", output, "--emit", format],
                    None,
                ),
                1,
                &[
                    "output path refers to the source file",
                    "refusing to overwrite source",
                ],
            );
            assert_eq!(fs::read(&input).unwrap(), SIMPLE_SOURCE.as_bytes());
            scratch.assert_entries(&["program.was", "nested"]);
        }
    }
}

#[test]
fn output_io_failure_does_not_create_parents_or_leave_sibling_temporaries() {
    let scratch = Scratch::new("output_io");
    scratch.write("program.was", SIMPLE_SOURCE);
    failure(
        scratch.cli(&["program.was", "-o", "missing/program.wasm"], None),
        1,
        &["output I/O error:", "program.wasm"],
    );
    assert!(!scratch.0.join("missing").exists());
    scratch.assert_entries(&["program.was"]);
    let output = scratch.0.join("directory.wasm");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep.txt"), b"keep").unwrap();
    failure(
        scratch.cli(&["program.was", "-o", "directory.wasm"], None),
        1,
        &["directory.wasm: output I/O error:"],
    );
    assert_eq!(fs::read(output.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
    scratch.assert_entries(&["program.was", "directory.wasm"]);
}

#[test]
fn repeatable_host_declarations_package_all_signatures_in_registration_order() {
    let scratch = Scratch::new("host_snapshot");
    let source = r#"
fn main(value: number) -> number {
    print(value);
    if ready() { return bias(value); } else { return value; }
}
fn text(value: string) -> string { return echo(value); }
fn values(value: number[]) -> number[] { return edit(value); }
"#;
    let hosts = [
        "--host",
        "print=env.log_number(number)->void",
        "--host",
        "ready=app.ready()->bool",
        "--host",
        "bias=env.bias(number)->number",
        "--host",
        "echo=env.echo(string)->string",
        "--host",
        "edit=env.edit(number[])->number[]",
        "--host",
        "idle=env.unused(bool, string, number[])->void",
    ];
    let exports = [
        r#"{"name":"main","params":["number"],"result":"number"}"#,
        r#"{"name":"text","params":["string"],"result":"string"}"#,
        r#"{"name":"values","params":["number[]"],"result":"number[]"}"#,
    ];
    let imports = [
        r#"{"name":"print","module":"env","field":"log_number","params":["number"],"result":"void"}"#,
        r#"{"name":"ready","module":"app","field":"ready","params":[],"result":"bool"}"#,
        r#"{"name":"bias","module":"env","field":"bias","params":["number"],"result":"number"}"#,
        r#"{"name":"echo","module":"env","field":"echo","params":["string"],"result":"string"}"#,
        r#"{"name":"edit","module":"env","field":"edit","params":["number[]"],"result":"number[]"}"#,
        r#"{"name":"idle","module":"env","field":"unused","params":["bool","string","number[]"],"result":"void"}"#,
    ];
    let all = decode(&scratch.compile(source, &hosts));
    assert_metadata(&all, &exports, &imports);
    assert_eq!(
        all.imports,
        [
            ("env".into(), "log_number".into()),
            ("app".into(), "ready".into()),
            ("env".into(), "bias".into()),
            ("env".into(), "echo".into()),
            ("env".into(), "edit".into()),
            ("env".into(), "unused".into()),
        ]
    );
    let mut selected_flags = hosts.to_vec();
    selected_flags.extend(["--export", "values", "--export", "main"]);
    let only = decode(&scratch.compile(source, &selected_flags));
    assert_metadata(&only, &[exports[0], exports[2]], &imports);
    assert_eq!(user_exports(&only), ["main", "values"]);
    let mut none_flags = hosts.to_vec();
    none_flags.push("--no-exports");
    let none = decode(&scratch.compile(source, &none_flags));
    assert_metadata(&none, &[], &imports);
    for selected in [&only, &none] {
        assert_eq!(selected.imports, all.imports);
        assert_eq!(selected.sections, all.sections);
        assert_eq!(selected.bodies, all.bodies);
        assert_eq!(selected.code_count, all.code_count);
        assert_eq!(abi_exports(selected), abi_exports(&all));
    }
    let mut args = vec!["program.was", "--emit", "wat"];
    args.extend(hosts);
    let bytes = success(scratch.cli(&args, None));
    let wat = assert_plain_wat(&bytes);
    assert!(wat.contains("(import \"env\" \"log_number\""));
    assert!(wat.contains("(import \"env\" \"unused\""));
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn malformed_or_invalid_host_declarations_exit_one_without_outputs() {
    let scratch = Scratch::new("invalid_hosts");
    scratch.write("program.was", SIMPLE_SOURCE);
    for (declaration, expected) in [
        ("bad", "expected NAME=MODULE.FIELD(TYPES)->RESULT"),
        ("print=env.log_number(number,)->void", "unknown type"),
        ("print=env.log_number(unknown)->void", "unknown type"),
        ("print=env.log_number(number)->unknown", "unknown type"),
        ("print=env.log_number(void)->void", "unsupported ABI type"),
        ("print=.log_number(number)->void", "module cannot be empty"),
        ("print=env.(number)->void", "field cannot be empty"),
        (
            "1print=env.log_number(number)->void",
            "Invalid host function name",
        ),
        ("len=env.len(number[])->number", "reserved"),
    ] {
        let stderr = failure(
            scratch.cli(
                &["program.was", "-o", "program.wasm", "--host", declaration],
                None,
            ),
            1,
            &["host registration error", expected],
        );
        assert!(stderr.starts_with("host registration error"));
        scratch.assert_entries(&["program.was"]);
    }
    failure(
        scratch.cli(
            &[
                "program.was",
                "-o",
                "program.wasm",
                "--host",
                "print=env.log_number(number)->void",
                "--host",
                "print=env.again(number)->void",
            ],
            None,
        ),
        1,
        &["host registration error", "already registered"],
    );
    scratch.assert_entries(&["program.was"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_compiles_and_runs_the_array_example_to_65() {
    let scratch = Scratch::new("node_example");
    assert!(success(scratch.cli(&["-o", "program.wasm"], None)).is_empty());
    decode(&fs::read(scratch.0.join("program.wasm")).unwrap());
    assert_eq!(text(scratch.node(&["program.wasm"])), "65\n");
    scratch.assert_entries(&["program.wasm"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_entry_and_json_arguments_use_language_types_and_selected_exports() {
    let scratch = Scratch::new("node_arguments");
    scratch.compile(
        r#"
fn main() -> number { return 7; }
fn calculate(value: number, enabled: bool) -> number {
    if enabled { return helper(value); } else { return value; }
}
fn helper(value: number) -> number { return value * 2; }
fn invert(value: bool) -> bool { return !value; }
fn nothing() -> void {}
fn text(value: string) -> string { return value; }
fn values(value: number[]) -> number[] { value[0] = 9; return value; }
"#,
        &[
            "-O3",
            "--export",
            "calculate",
            "--export",
            "invert",
            "--export",
            "nothing",
            "--export",
            "text",
            "--export",
            "values",
        ],
    );
    for (entry, args, expected) in [
        ("calculate", "[21,true]", "42\n"),
        ("calculate", "[21,false]", "21\n"),
        ("invert", "[true]", "false\n"),
        ("nothing", "[]", ""),
        ("text", "[\"héllo 🦀\"]", "héllo 🦀\n"),
        ("text", "[\"\"]", "\n"),
        ("values", "[[1,2]]", "[ 9, 2 ]\n"),
    ] {
        assert_eq!(
            text(scratch.node(&["program.wasm", "--entry", entry, "--args", args])),
            expected
        );
    }
    for (entry, args, expected) in [
        (
            "calculate",
            "[]",
            "(number,bool)->number expects 2 arguments, got 0",
        ),
        ("calculate", "[21,true,0]", "expects 2 arguments, got 3"),
        (
            "calculate",
            "[\"21\",true]",
            "argument 1 of calculate must be number",
        ),
        ("invert", "[1]", "must be bool"),
        ("text", "[1]", "must be string"),
        ("values", "[[1,\"2\"]]", "[1] must be number"),
        ("calculate", "{}", "JSON array"),
        ("helper", "[21]", "not a language export"),
        ("main", "[]", "not a language export"),
    ] {
        let stderr = failure(
            scratch.node(&["program.wasm", "--entry", entry, "--args", args]),
            1,
            &[expected],
        );
        assert!(stderr.starts_with("invocation:"));
    }
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_native_numeric_hosts_and_missing_provider_signatures() {
    let scratch = Scratch::new("node_numeric_hosts");
    scratch.compile(
        r#"
fn main(value: number) -> number {
    print(value);
    if ready() { return bias(value); } else { return value; }
}
"#,
        &[
            "--host",
            "print=env.log_number(number)->void",
            "--host",
            "ready=env.ready()->bool",
            "--host",
            "bias=env.bias(number)->number",
        ],
    );
    let stderr = failure(
        scratch.node(&["program.wasm", "--args", "[40]"]),
        1,
        &["env.log_number (number)->void"],
    );
    assert!(stderr.starts_with("instantiation:"));
    scratch.write("partial.cjs", "module.exports = {env: {log_number() {}}};");
    failure(
        scratch.node(&["program.wasm", "--args", "[40]", "--imports", "partial.cjs"]),
        1,
        &["instantiation:", "env.ready ()->bool"],
    );
    scratch.write(
        "partial.cjs",
        "module.exports = {env: {log_number() {}, ready() { return true; }}};",
    );
    failure(
        scratch.node(&["program.wasm", "--args", "[40]", "--imports", "partial.cjs"]),
        1,
        &["instantiation:", "env.bias (number)->number"],
    );
    scratch.write(
        "provider.cjs",
        r#"
const assert = require('node:assert/strict');
module.exports = {env: {
    log_number(value) { assert.equal(typeof value, 'number'); assert.equal(value, 40); },
    ready() { return true; },
    bias(value) { assert.equal(typeof value, 'number'); return value + 2; }
}};
"#,
    );
    assert_eq!(
        text(scratch.node(&[
            "program.wasm",
            "--args",
            "[40]",
            "--imports",
            "provider.cjs"
        ])),
        "42\n"
    );
    scratch.write("bad.cjs", "module.exports = {env: {log_number() { return 99; }, ready() { return true; }, bias(value) { return value; }}};");
    failure(
        scratch.node(&["program.wasm", "--args", "[40]", "--imports", "bad.cjs"]),
        1,
        &[
            "invocation:",
            "env.log_number (number)->void",
            "must be void (undefined)",
        ],
    );
    scratch.assert_entries(&[
        "program.was",
        "program.wasm",
        "partial.cjs",
        "provider.cjs",
        "bad.cjs",
    ]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_native_string_and_array_hosts_copy_and_allocate_payloads() {
    let scratch = Scratch::new("node_memory_hosts");
    scratch.compile(
        r#"
fn text(value: string) -> string { return echo(value); }
fn edited() -> number[] {
    let original: number[] = [1, 2];
    let result: number[] = edit(original);
    result[0] = result[0] + 1;
    return [original[0], result[0]];
}
fn grown() -> number {
    let result: number[] = large();
    result[0] = 9;
    return len(result) + result[19999];
}
"#,
        &[
            "--host",
            "echo=env.echo(string)->string",
            "--host",
            "edit=env.edit(number[])->number[]",
            "--host",
            "large=env.large()->number[]",
        ],
    );
    scratch.write(
        "provider.cjs",
        r#"
const assert = require('node:assert/strict');
let retained;
const large = new Array(20000).fill(2);
module.exports = {env: {
    echo(value) { assert.equal(typeof value, 'string'); return value + '!'; },
    edit(values) {
        assert(Array.isArray(values));
        assert.deepEqual(values, [1, 2]);
        retained = values;
        values[0] = 40;
        return values;
    },
    large() { return large; }
}};
process.on('exit', () => {
    if (retained) assert.deepEqual(retained, [40, 2]);
    assert.equal(large[0], 2);
});
"#,
    );
    for (entry, args, expected) in [
        ("text", "[\"héllo 🦀\"]", "héllo 🦀!\n"),
        ("text", "[\"\"]", "!\n"),
        ("edited", "[]", "[ 1, 41 ]\n"),
        ("grown", "[]", "20002\n"),
    ] {
        assert_eq!(
            text(scratch.node(&[
                "program.wasm",
                "--entry",
                entry,
                "--args",
                args,
                "--imports",
                "provider.cjs"
            ])),
            expected
        );
    }
    scratch.write("bad.cjs", "module.exports = {env: {echo(value) { return value; }, edit() { return new Float64Array([1]); }, large() { return []; }}};");
    failure(
        scratch.node(&["program.wasm", "--entry", "edited", "--imports", "bad.cjs"]),
        1,
        &[
            "invocation:",
            "return value of env.edit (number[])->number[]",
            "must be number[]",
        ],
    );
    scratch.assert_entries(&["program.was", "program.wasm", "provider.cjs", "bad.cjs"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_array_boundaries_trap_instead_of_printing_a_result() {
    let scratch = Scratch::new("node_bounds");
    scratch.compile("fn main(values: number[], index: number) -> number { return values[index]; } fn store(values: number[], index: number) -> number { values[index] = 9; return values[0]; }", &[]);
    assert_eq!(
        text(scratch.node(&["program.wasm", "--args", "[[4,5],1]"])),
        "5\n"
    );
    assert_eq!(
        text(scratch.node(&["program.wasm", "--entry", "store", "--args", "[[4,5],1]"])),
        "4\n"
    );
    for entry in ["main", "store"] {
        for args in [
            "[[],0]",
            "[[1],-1]",
            "[[1],1]",
            "[[1],0.5]",
            "[[1],4294967296]",
        ] {
            let stderr = failure(
                scratch.node(&["program.wasm", "--entry", entry, "--args", args]),
                1,
                &["invocation:", "cannot call", "unreachable"],
            );
            assert!(stderr.contains(&format!("\"{entry}\"")));
        }
    }
    failure(
        scratch.node(&["program.wasm", "--args", "[[1],\"0\"]"]),
        1,
        &["invocation:", "argument 2 of main must be number"],
    );
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_runner_reports_compilation_errors_for_plain_wat() {
    let scratch = Scratch::new("node_wat");
    scratch.write("program.was", SIMPLE_SOURCE);
    assert!(
        success(scratch.cli(
            &["program.was", "-o", "program.wasm", "--emit", "wat"],
            None
        ))
        .is_empty()
    );
    assert_plain_wat(&fs::read(scratch.0.join("program.wasm")).unwrap());
    let stderr = failure(
        scratch.node(&["program.wasm"]),
        1,
        &["cannot compile WebAssembly bytes"],
    );
    assert!(stderr.starts_with("compilation:"));
    scratch.assert_entries(&["program.was", "program.wasm"]);
}
