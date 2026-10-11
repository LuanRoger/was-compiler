#[path = "support/process.rs"]
mod test_process;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use was_compiler::{Compiler, ExportOptions, Type};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef, Validator};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
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

    fn node_script(&self, script: &str) -> Output {
        let mut command = Command::new("node");
        command.current_dir(&self.0).args(["-e", script]);
        process_output(&mut command, None)
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

fn process_output(command: &mut Command, input: Option<&[u8]>) -> Output {
    test_process::output(command, input, PROCESS_TIMEOUT)
}

fn process_output_with_stdin(
    command: &mut Command,
    input: Option<&[u8]>,
    keep_open: bool,
) -> Output {
    test_process::output_with_stdin(command, input, keep_open, PROCESS_TIMEOUT)
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
    assert!(
        module.custom.is_empty(),
        "unexpected custom sections: {:?}",
        module.custom
    );
    assert_eq!(module.code_count as usize, module.functions.len());
    assert_eq!(module.bodies.len(), module.functions.len());
    module
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
fn missing_source_is_a_usage_error_without_artifacts() {
    let scratch = Scratch::new("no_arguments");
    for args in [
        vec![],
        vec!["--emit", "wat"],
        vec!["--export", "main"],
        vec!["--no-exports"],
        vec!["-O3"],
        vec!["--host", "print=env.log_number(number)->void"],
    ] {
        let stderr = failure(
            scratch.cli(&args, Some(SIMPLE_SOURCE.as_bytes())),
            2,
            &["required arguments", "<SOURCE>", "Usage:", "--help"],
        );
        assert!(stderr.starts_with("error:"));
        scratch.assert_entries(&[]);
    }
}

#[test]
fn output_flags_without_source_leave_existing_and_missing_artifacts_unchanged() {
    let scratch = Scratch::new("missing_source_outputs");
    let sentinel = b"existing artifact";
    for name in ["existing.wasm", "existing.wat", "-"] {
        scratch.write(name, sentinel);
    }
    for name in [
        "existing.wasm",
        "existing.wat",
        "missing.wasm",
        "missing.wat",
        "-",
    ] {
        for output_flag in ["-o", "--output"] {
            for format in [None, Some("wasm"), Some("wat")] {
                let mut args = vec![output_flag, name];
                if let Some(format) = format {
                    args.extend(["--emit", format]);
                }
                failure(
                    scratch.cli(&args, Some(SIMPLE_SOURCE.as_bytes())),
                    2,
                    &["required arguments", "<SOURCE>"],
                );
                for existing in ["existing.wasm", "existing.wat", "-"] {
                    assert_eq!(fs::read(scratch.0.join(existing)).unwrap(), sentinel);
                }
                scratch.assert_entries(&["existing.wasm", "existing.wat", "-"]);
            }
        }
    }
}

#[test]
fn stdin_is_not_consumed_implicitly_or_for_a_literal_dash_source() {
    let scratch = Scratch::new("open_stdin");
    scratch.write("program.was", SIMPLE_SOURCE);
    for args in [
        vec![],
        vec!["--emit", "wat"],
        vec!["program.was"],
        vec!["-"],
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_was-compiler"));
        command.current_dir(&scratch.0).args(&args);
        let output = process_output_with_stdin(&mut command, None, true);
        match args.first().copied() {
            Some("program.was") => assert_eq!(text(output), raw_wat(SIMPLE_SOURCE)),
            Some("-") => {
                failure(output, 1, &["-: input I/O error:"]);
            }
            _ => {
                failure(output, 2, &["required arguments", "<SOURCE>"]);
            }
        }
    }
    scratch.assert_entries(&["program.was"]);
}

#[test]
fn help_flags_print_usage_without_compiling_or_writing() {
    let scratch = Scratch::new("help");
    for flag in ["-h", "--help"] {
        let help = text(scratch.cli(&[flag], None));
        let help_with_source =
            text(scratch.cli(&["missing.was", "-o", "program.wasm", flag], None));
        let help_with_output = text(scratch.cli(&["-o", "program.wasm", flag], None));
        assert_eq!(help, help_with_source);
        assert_eq!(help, help_with_output);
        assert!(help.contains("Usage: was-compiler [OPTIONS] <SOURCE>"));
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
        assert!(!help.to_lowercase().contains("stdin"));
        assert!(!help.to_lowercase().contains("sample"));
        assert!(help.to_lowercase().contains("source file"));
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
        (&[""], &["<SOURCE>", "path cannot be empty"]),
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
        (
            &["program.was", "--emit", "wasm"],
            &["binary Wasm requires -o"],
        ),
        (
            &["program.was", "--emit", "wasm", "-o", "-"],
            &["binary Wasm requires -o"],
        ),
    ];
    for (args, details) in cases {
        let stderr = failure(scratch.cli(args, None), 2, details);

        assert!(stderr.starts_with("error:"));
        assert!(stderr.contains("--help"));
        scratch.assert_entries(&[]);
    }
}

#[test]
fn file_sources_emit_plain_wat_to_stdout() {
    let scratch = Scratch::new("file_stdout");
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
    let bytes = fs::read(scratch.0.join("program.wasm")).unwrap();
    decode(&bytes);
    assert_eq!(bytes, raw_wasm(SIMPLE_SOURCE));
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
            assert_eq!(bytes, raw, "CLI output differs from raw compiler bytes");
            decode(&bytes);
        } else {
            assert_eq!(assert_plain_wat(&bytes), raw_wat(SIMPLE_SOURCE));
        }
    }
    let mut names = vec!["program.was"];
    names.extend(cases.iter().map(|(name, _, _)| *name));
    scratch.assert_entries(&names);
}

#[test]
fn dash_source_is_a_literal_file_and_ignores_stdin() {
    let scratch = Scratch::new("dash_file");
    failure(
        scratch.cli(&["-", "-o", "program.wasm"], Some(SIMPLE_SOURCE.as_bytes())),
        1,
        &["-: input I/O error:"],
    );
    scratch.assert_entries(&[]);
    scratch.write("-", SIMPLE_SOURCE);
    for args in [
        vec!["-"],
        vec!["--", "-"],
        vec!["-", "-o", "-", "--emit", "wat"],
    ] {
        assert_eq!(
            text(scratch.cli(&args, Some(b"invalid stdin"))),
            raw_wat(SIMPLE_SOURCE)
        );
    }
    assert!(
        success(scratch.cli(&["-", "--output", "program.wasm"], Some(b"invalid stdin"))).is_empty()
    );
    let bytes = fs::read(scratch.0.join("program.wasm")).unwrap();
    decode(&bytes);
    assert_eq!(bytes, raw_wasm(SIMPLE_SOURCE));
    assert_eq!(
        fs::read(scratch.0.join("-")).unwrap(),
        SIMPLE_SOURCE.as_bytes()
    );
    scratch.assert_entries(&["-", "program.wasm"]);
}

#[test]
fn binary_cli_output_is_exactly_raw_library_codegen() {
    let scratch = Scratch::new("raw_library");
    let raw = raw_wasm(MEMORY_SOURCE);
    assert!(decode(&raw).custom.is_empty());
    let bytes = scratch.compile(MEMORY_SOURCE, &[]);
    assert_eq!(bytes, raw);
    let wat = success(scratch.cli(&["program.was"], None));
    assert_eq!(
        assert_plain_wat(&wat),
        wasmprinter::print_bytes(&raw).unwrap()
    );
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn compound_cli_output_matches_library_bytes_and_wat_at_every_level() {
    let scratch = Scratch::new("compound_binary");
    let source = "
        fn main() -> number {
            let values = [10];
            values[0] += 2;
            values[0] -= 1;
            values[0] *= 3;
            values[0] /= 2;
            values[0] %= 5;
            let n = 10;
            n += values[0];
            n -= 1;
            n *= 3;
            n /= 2;
            n %= 5;
            return n;
        }
        fn hidden() -> void { let n = 1; return; n %= 2; }
    ";
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let expected = compiler.compile_source(source).unwrap();

    for flag in ["-O0", "-O1", "-O2", "-O3"] {
        assert_eq!(
            scratch.compile(source, &[flag, "--export", "main"]),
            expected
        );
        let wat = success(scratch.cli(
            &["program.was", flag, "--export", "main", "--emit", "wat"],
            None,
        ));
        assert_eq!(
            assert_plain_wat(&wat),
            wasmprinter::print_bytes(&expected).unwrap()
        );
    }
}

#[test]
fn loop_transfers_match_library_binary_and_wat_at_every_level() {
    let scratch = Scratch::new("loop_control");
    let source = "
        fn main() -> number {
            let i = 0;
            let total = 0;
            while i < 10 {
                i += 1;
                if i == 3 { continue; }
                if i == 7 { break; }
                total += i;
            }
            return total;
        }
        fn hidden() -> void { while false { return; break; continue; } }
    ";
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let expected = compiler.compile_source(source).unwrap();

    for flag in ["-O0", "-O1", "-O2", "-O3"] {
        assert_eq!(
            scratch.compile(source, &[flag, "--export", "main"]),
            expected
        );
        let wat = success(scratch.cli(
            &["program.was", flag, "--export", "main", "--emit", "wat"],
            None,
        ));
        assert_eq!(
            assert_plain_wat(&wat),
            wasmprinter::print_bytes(&expected).unwrap()
        );
    }
}

#[test]
fn statement_and_value_loops_match_library_binary_and_wat_at_every_level() {
    let scratch = Scratch::new("value_loops_parity");
    let source = r#"
        fn main() -> number {
            let i = 0;
            loop {
                i += 1;
                if i == 2 { continue; }
                if i == 4 { break; }
            }
            let result = loop {
                let j = 0;
                loop { j += 1; if j == 2 { break; } }
                break i + j;
            };
            return result;
        }
        fn boolean(flag: bool) -> bool {
            return loop { if flag { break true; } else { break false; } };
        }
        fn text() -> string { return loop { break "é🦀"; }; }
        fn items() -> number[] { return loop { break [1, 2]; }; }
        fn early() -> number { loop { return 7; } }
        fn no_exit_statement() -> number { loop { continue; } }
        fn no_exit_value() -> bool { let n: number = loop {}; }
    "#;
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let baseline = compiler.compile_source(source).unwrap();
    let expected_wat = wasmprinter::print_bytes(&baseline).unwrap();
    assert_eq!(user_exports(&decode(&baseline)), ["main"]);

    let bodies = Parser::new(0)
        .parse_all(&baseline)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (body, (ty, break_depth)) in bodies.iter().zip([
        (wasmparser::ValType::F64, 1),
        (wasmparser::ValType::I32, 2),
        (wasmparser::ValType::I32, 1),
        (wasmparser::ValType::I32, 1),
    ]) {
        let operators = body
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert!(operators.iter().any(|op| matches!(
            op,
            wasmparser::Operator::Block { blockty }
                if *blockty == wasmparser::BlockType::Type(ty)
        )));
        assert!(operators.iter().any(|op| matches!(
            op,
            wasmparser::Operator::Loop {
                blockty: wasmparser::BlockType::Empty
            }
        )));
        assert!(operators.iter().any(|op| matches!(
            op,
            wasmparser::Operator::Br { relative_depth } if *relative_depth == break_depth
        )));
    }

    for (flag, level) in [
        ("-O0", was_compiler::OptimizationLevel::None),
        ("-O1", was_compiler::OptimizationLevel::Basic),
        ("-O2", was_compiler::OptimizationLevel::Standard),
        ("-O3", was_compiler::OptimizationLevel::Aggressive),
    ] {
        compiler.set_optimization_level(level);
        let expected = compiler.compile_source(source).unwrap();
        assert_eq!(expected, baseline);
        assert_eq!(
            scratch.compile(source, &[flag, "--export", "main"]),
            expected
        );
        let wat = success(scratch.cli(
            &["program.was", flag, "--export", "main", "--emit", "wat"],
            None,
        ));
        assert_eq!(assert_plain_wat(&wat), expected_wat);
        assert!(
            success(scratch.cli(
                &[
                    "program.was",
                    flag,
                    "--export",
                    "main",
                    "--emit",
                    "wat",
                    "-o",
                    "program.wat",
                ],
                None,
            ))
            .is_empty()
        );
        assert_eq!(fs::read(scratch.0.join("program.wat")).unwrap(), wat);
    }
    scratch.assert_entries(&["program.was", "program.wasm", "program.wat"]);
}

#[test]
fn value_loops_cli_errors_preserve_spans_and_destinations_at_every_level() {
    use was_compiler::{CompileError, semantic::SemanticError, tokens::Span};

    let scratch = Scratch::new("value_loops_errors");
    let sentinel = b"existing artifact";
    let cases = [
        (
            "fn main() -> void {\n    while true {\n        break 1;\n    }\n}",
            "break 1;",
            3,
            9,
            "A break value is not allowed in a statement loop",
            SemanticError::BreakValueNotAllowed {
                span: Span::new(0, 0),
            },
        ),
        (
            "fn main() -> void {\n    while true {\n        break outer;\n    }\n}",
            "break outer;",
            3,
            9,
            "A break value is not allowed in a statement loop",
            SemanticError::BreakValueNotAllowed {
                span: Span::new(0, 0),
            },
        ),
        (
            "fn main() -> void {\n    loop {\n        break 1;\n    }\n}",
            "break 1;",
            3,
            9,
            "A break value is not allowed in a statement loop",
            SemanticError::BreakValueNotAllowed {
                span: Span::new(0, 0),
            },
        ),
        (
            "fn main() -> number {\n    return loop {\n        break;\n    };\n}",
            "break;",
            3,
            9,
            "A value loop requires a break value",
            SemanticError::MissingBreakValue {
                span: Span::new(0, 0),
            },
        ),
        (
            "fn main() -> number {\n    return loop {\n        break {};\n    };\n}",
            "break {};",
            3,
            9,
            "A break value cannot have type void",
            SemanticError::InvalidBreakValue {
                span: Span::new(0, 0),
            },
        ),
        (
            "fn main(flag: bool) -> void {\n    let value = loop { if flag { break 1; } else { break false; } };\n}",
            "let value = loop { if flag { break 1; } else { break false; } };",
            2,
            5,
            "Type mismatch: expected Number, found Bool",
            SemanticError::TypeMismatch {
                expected: Type::Number,
                found: Type::Bool,
                span: Span::new(0, 0),
            },
        ),
        (
            "fn main() -> void {\n    let value = loop {};\n}",
            "let value = loop {};",
            2,
            5,
            "Cannot infer a concrete type for divergent initializer 'value'",
            SemanticError::CannotInferNeverType {
                name: "value".into(),
                span: Span::new(0, 0),
            },
        ),
    ];
    for existing in [false, true] {
        for (format, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
            if existing {
                scratch.write(destination, sentinel);
            }
            for (source, excerpt, line, column, message, variant) in &cases {
                let input = scratch.write("program.was", source);
                let start = source.find(excerpt).unwrap();
                let expected_span = Span::new(start, start + excerpt.len());
                let error = Compiler::new().compile_source(source).unwrap_err();
                let CompileError::Semantic(semantic) = &error else {
                    panic!("expected semantic error, got {error:?}");
                };
                assert_eq!(
                    std::mem::discriminant(semantic),
                    std::mem::discriminant(variant)
                );
                assert_eq!(semantic.span(), Some(expected_span));
                assert_eq!(error.span(), Some(expected_span));
                let expected = format!(
                    "{}\n",
                    was_compiler::diagnostic::render("program.was", source, &error)
                );
                let location = format!("program.was:{line}:{column}: semantic error:");
                let caret = format!(" |{}^", " ".repeat(*column));
                for flag in ["-O0", "-O1", "-O2", "-O3"] {
                    let before = scratch.entries();
                    let stderr = failure(
                        scratch.cli(
                            &["program.was", flag, "--emit", format, "-o", destination],
                            None,
                        ),
                        1,
                        &[&location, message, excerpt, &caret],
                    );
                    assert_eq!(stderr, expected);
                    assert_eq!(fs::read(&input).unwrap(), source.as_bytes());
                    if existing {
                        assert_eq!(fs::read(scratch.0.join(destination)).unwrap(), sentinel);
                    } else {
                        assert!(!scratch.0.join(destination).exists());
                    }
                    assert_eq!(scratch.entries(), before, "failed compilation leaked files");
                }
            }
        }
    }
    scratch.assert_entries(&["program.was", "program.wasm", "program.wat"]);
}

#[test]
fn invalid_loop_transfers_report_locations_without_replacing_output() {
    let scratch = Scratch::new("invalid_loop_control");
    scratch.write("program.wasm", b"existing output");
    for (keyword, message) in [
        ("break", "Break is only allowed inside a loop"),
        ("continue", "Continue is only allowed inside a loop"),
    ] {
        scratch.write(
            "program.was",
            format!("fn main() -> void {{\n    {keyword};\n}}"),
        );
        failure(
            scratch.cli(&["program.was", "-o", "program.wasm"], None),
            1,
            &["program.was:2:5", "semantic error", message],
        );
        assert_eq!(
            fs::read(scratch.0.join("program.wasm")).unwrap(),
            b"existing output"
        );
    }
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn comments_preserve_raw_cli_and_library_wasm() {
    let scratch = Scratch::new("comments");
    let plain =
        "fn main() -> number { return 84 / 2; } fn text() -> string { return \"// /* */ é🦀\"; }";
    let commented = concat!(
        "// leading comment\n",
        "/* multiline\n/* nested opener is text */fn/**/main()/* signature */->number{\n",
        "// stops at CR\rreturn/* left */84 / /* divisor */2;// stops at LF\n",
        "}\nfn text()->string{return \"// /* */ é🦀\";}// EOF",
    );
    let expected = raw_wasm(plain);
    assert_eq!(raw_wasm(commented), expected);
    assert_eq!(scratch.compile(plain, &[]), expected);
    assert_eq!(scratch.compile(commented, &[]), expected);
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn inferred_cli_binary_matches_annotated_library_at_every_level() {
    let scratch = Scratch::new("inference_binary");
    let annotated = r#"
        fn main() -> number {
            let n: number = 10;
            let flag: bool = true;
            let text: string = "é🦀";
            let values: number[] = [n, 2];
            let empty: number[] = [];
            let alias: number[] = values;
            if flag { alias[1] = later(n); }
            return values[1] + len(empty);
        }
        fn later(value: number) -> number { return value * 2; }
    "#;
    let inferred = annotated
        .replace(": number[] =", " =")
        .replace(": number =", " =")
        .replace(": bool =", " =")
        .replace(": string =", " =");
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    for (flag, level) in [
        ("-O0", was_compiler::OptimizationLevel::None),
        ("-O1", was_compiler::OptimizationLevel::Basic),
        ("-O2", was_compiler::OptimizationLevel::Standard),
        ("-O3", was_compiler::OptimizationLevel::Aggressive),
    ] {
        compiler.set_optimization_level(level);
        let expected = compiler.compile_source(annotated).unwrap();
        assert_eq!(
            scratch.compile(&inferred, &[flag, "--export", "main"]),
            expected
        );
    }
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn inferred_cli_reassignment_and_void_errors_preserve_diagnostics_and_outputs() {
    let scratch = Scratch::new("inference_errors");
    let sentinel = b"existing artifact";
    let cases = [
        (
            "fn main() -> void {\n    let n = 10;\n    n = true;\n}",
            "program.was:3:5: semantic error:",
            "Type mismatch: expected Number, found Bool",
            "3 |     n = true;",
        ),
        (
            "fn main() -> void {\n    let value = noop();\n}\nfn noop() -> void {}",
            "program.was:2:5: semantic error:",
            "'value' cannot have type void",
            "2 |     let value = noop();",
        ),
    ];
    for existing in [false, true] {
        if existing {
            scratch.write("program.wasm", sentinel);
        }
        for (source, location, message, excerpt) in cases {
            let input = scratch.write("program.was", source);
            let before = scratch.entries();
            let stderr = failure(
                scratch.cli(&["program.was", "-o", "program.wasm"], None),
                1,
                &[location, message, excerpt, " |     ^"],
            );
            assert!(stderr.starts_with(location));
            assert_eq!(fs::read(input).unwrap(), source.as_bytes());
            if existing {
                assert_eq!(fs::read(scratch.0.join("program.wasm")).unwrap(), sentinel);
            } else {
                assert!(!scratch.0.join("program.wasm").exists());
            }
            assert_eq!(scratch.entries(), before, "failed compilation leaked files");
        }
    }
}

#[test]
fn value_blocks_cli_binary_and_wat_match_library_at_every_level() {
    let scratch = Scratch::new("value_blocks_parity");
    let source = r#"
        fn main(flag: bool) -> number {
            let n = { let inner = 4; inner + 1 };
            return if flag { n % 3 } else if false { 99 } else { 7 };
        }
        fn boolean(flag: bool) -> bool {
            return { let result = if flag { true } else { false }; result };
        }
        fn text(flag: bool) -> string {
            return { if flag { "é🦀" } else { "other" } };
        }
        fn items(flag: bool) -> number[] {
            return { if flag { [1, 2] } else { [] } };
        }
        fn noop() -> void { {}; (if true {} else {}); }
        fn early(flag: bool) -> number {
            let n: number = if flag { return 11; } else { 5 };
            return n;
        }
        fn divergent() -> number { let n: bool = { return 13; }; }
        fn transfers() -> number {
            let i = 0;
            let total = 0;
            while i < 4 {
                i += 1;
                let n: number = if i == 2 { continue; } else {
                    if i == 4 { break; }
                    { let j = i; j }
                };
                total += n;
            }
            return total;
        }
    "#;
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let baseline = compiler.compile_source(source).unwrap();
    let expected_wat = wasmprinter::print_bytes(&baseline).unwrap();
    let module = decode(&baseline);
    assert_eq!(user_exports(&module), ["main"]);

    let bodies = Parser::new(0)
        .parse_all(&baseline)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (body, ty) in bodies.iter().zip([
        wasmparser::ValType::F64,
        wasmparser::ValType::I32,
        wasmparser::ValType::I32,
        wasmparser::ValType::I32,
    ]) {
        let operators = body
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert!(operators.iter().any(|op| matches!(
            op,
            wasmparser::Operator::Block { blockty }
                if *blockty == wasmparser::BlockType::Type(ty)
        )));
        assert!(operators.iter().any(|op| matches!(
            op,
            wasmparser::Operator::If { blockty }
                if *blockty == wasmparser::BlockType::Type(ty)
        )));
    }

    for (flag, level) in [
        ("-O0", was_compiler::OptimizationLevel::None),
        ("-O1", was_compiler::OptimizationLevel::Basic),
        ("-O2", was_compiler::OptimizationLevel::Standard),
        ("-O3", was_compiler::OptimizationLevel::Aggressive),
    ] {
        compiler.set_optimization_level(level);
        let expected = compiler.compile_source(source).unwrap();
        assert_eq!(expected, baseline);
        assert_eq!(
            scratch.compile(source, &[flag, "--export", "main"]),
            expected
        );
        let wat = success(scratch.cli(
            &["program.was", flag, "--export", "main", "--emit", "wat"],
            None,
        ));
        assert_eq!(assert_plain_wat(&wat), expected_wat);
        assert!(
            success(scratch.cli(
                &[
                    "program.was",
                    flag,
                    "--export",
                    "main",
                    "--emit",
                    "wat",
                    "-o",
                    "program.wat",
                ],
                None,
            ))
            .is_empty()
        );
        assert_eq!(fs::read(scratch.0.join("program.wat")).unwrap(), wat);
    }
    scratch.assert_entries(&["program.was", "program.wasm", "program.wat"]);
}

#[test]
fn value_blocks_cli_diagnostics_preserve_locations_and_destinations_at_every_level() {
    let scratch = Scratch::new("value_blocks_errors");
    let sentinel = b"existing artifact";
    let cases = [
        (
            "fn main() -> number {\n    let value = if true { 1 };\n    return value;\n}",
            "An if expression requires an else branch",
            "let value = if true { 1 };",
            5,
        ),
        (
            "fn main() -> number {\n    let value = { return 1; };\n}",
            "Cannot infer a concrete type for divergent initializer 'value'",
            "let value = { return 1; };",
            5,
        ),
        (
            "fn main() -> void {\n    let value = { 1; };\n}",
            "'value' cannot have type void",
            "let value = { 1; };",
            5,
        ),
        (
            "fn main() -> void {\n    let value = if true { 1 } else { false };\n}",
            "Type mismatch: expected Number, found Bool",
            "let value = if true { 1 } else { false };",
            5,
        ),
        (
            "fn main() -> void {\n    let value = if true { 1 } else { missing };\n}",
            "Undefined variable 'missing'",
            "let value = if true { 1 } else { missing };",
            36,
        ),
    ];
    for existing in [false, true] {
        for (format, destination) in [("wasm", "program.wasm"), ("wat", "program.wat")] {
            if existing {
                scratch.write(destination, sentinel);
            }
            for (source, message, excerpt, column) in cases {
                let input = scratch.write("program.was", source);
                let location = format!("program.was:2:{column}: semantic error:");
                let caret = format!(" |{}^", " ".repeat(column));
                let error = Compiler::new().compile_source(source).unwrap_err();
                let expected = format!(
                    "{}\n",
                    was_compiler::diagnostic::render("program.was", source, &error)
                );
                for flag in ["-O0", "-O1", "-O2", "-O3"] {
                    let before = scratch.entries();
                    let stderr = failure(
                        scratch.cli(
                            &["program.was", flag, "--emit", format, "-o", destination],
                            None,
                        ),
                        1,
                        &[&location, message, excerpt, &caret],
                    );
                    assert_eq!(stderr, expected);
                    assert_eq!(fs::read(&input).unwrap(), source.as_bytes());
                    if existing {
                        assert_eq!(fs::read(scratch.0.join(destination)).unwrap(), sentinel);
                    } else {
                        assert!(!scratch.0.join(destination).exists());
                    }
                    assert_eq!(scratch.entries(), before, "failed compilation leaked files");
                }
            }
        }
    }
    scratch.assert_entries(&["program.was", "program.wasm", "program.wat"]);
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
        let raw = compiler.compile_source(MEMORY_SOURCE).unwrap();
        assert_eq!(scratch.compile(MEMORY_SOURCE, &args[3..]), raw);
        let expected = wasmprinter::print_bytes(&raw).unwrap();
        let bytes = success(scratch.cli(&args, None));
        assert_eq!(assert_plain_wat(&bytes), expected);
    }
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn private_range_signatures_require_explicit_cli_exports_and_preserve_library_bytes() {
    let scratch = Scratch::new("range_exports");
    let source = r#"
fn identity(value: range) -> range { return value; }
fn main() -> number {
    let value = identity(1..=5 by by());
    return 65;
}
"#;
    scratch.write("program.was", source);
    let host = "by=env.step()->number";
    failure(
        scratch.cli(&["program.was", "-o", "program.wasm", "--host", host], None),
        1,
        &[
            "code generation error",
            "function `identity` has a private range signature",
            "explicit export selection",
            "ExportOptions::Only",
            "ExportOptions::None",
        ],
    );
    scratch.assert_entries(&["program.was"]);

    let mut compiler = Compiler::new();
    compiler
        .register_host_function("by", "env", "step", vec![], Type::Number)
        .unwrap();
    let mut selected: Option<Decoded> = None;
    for (flags, exports, expected_names) in [
        (
            vec!["--host", host, "--export", "main"],
            ExportOptions::Only(vec!["main".into()]),
            vec!["main"],
        ),
        (
            vec!["--host", host, "--no-exports"],
            ExportOptions::None,
            vec![],
        ),
    ] {
        compiler.set_exports(exports);
        let bytes = scratch.compile(source, &flags);
        assert_eq!(bytes, compiler.compile_source(source).unwrap());
        let decoded = decode(&bytes);
        assert_eq!(user_exports(&decoded), expected_names);
        assert_eq!(decoded.imports, [("env".into(), "step".into())]);
        let abi = abi_exports(&decoded);
        assert_eq!(abi.len(), 2);
        assert!(abi.iter().any(|(name, kind, index)| name == "memory"
            && *kind == ExternalKind::Memory
            && *index == 0));
        assert!(
            abi.iter()
                .any(|(name, kind, _)| name == "__alloc" && *kind == ExternalKind::Func)
        );
        if let Some(previous) = &selected {
            assert_eq!(decoded.bodies, previous.bodies);
            assert_eq!(decoded.functions, previous.functions);
            assert_eq!(abi, abi_exports(previous));
        }
        selected = Some(decoded);
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
        format!(
            "fn main() -> number {{ return 7; }}\nfn hidden() -> number {{ return {}; }}",
            "9".repeat(400)
        ),
    );
    for flags in [vec!["--export", "main"], vec!["--no-exports"]] {
        let mut args = vec!["program.was", "-o", "program.wasm"];
        args.extend(flags);
        let stderr = failure(
            scratch.cli(&args, None),
            1,
            &[
                "code generation error:",
                "outside the finite f64 range",
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
    assert_eq!(
        module.imports,
        [
            ("env".into(), "log_number".into()),
            ("app".into(), "ready".into())
        ]
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
    let snippet = format!("    return {};", "9".repeat(400));
    let overflow_source = format!("fn main() -> number {{\n{snippet}\n}}");

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
            overflow_source.as_str(),
            "code generation",
            snippet.as_str(),
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
fn unterminated_block_comment_reports_original_location_and_preserves_output() {
    let scratch = Scratch::new("unterminated_comment");
    let source = "// heading\r\nfn main() -> void {\r\n    /* é🦀 */ /* unterminated\nstill open";
    let input = scratch.write("program.was", source);
    let sentinel = b"existing artifact";
    let output = scratch.write("program.wasm", sentinel);
    let stderr = failure(
        scratch.cli(&["program.was", "-o", "program.wasm"], None),
        1,
        &[
            "program.was:3:14: parse error:",
            "3 |     /* é🦀 */ /* unterminated",
            " |              ^",
        ],
    );
    assert!(stderr.starts_with("program.was:3:14: parse error:"));
    assert_eq!(fs::read(input).unwrap(), source.as_bytes());
    assert_eq!(fs::read(output).unwrap(), sentinel);
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
fn file_diagnostics_and_unicode_crlf_tabs_have_correct_locations() {
    let scratch = Scratch::new("unicode_diagnostics");
    let source = "fn main() -> void {\r\n\tlet text: string = \"é🦀\"; missing();\r\n}\r\n";
    let line = source.lines().nth(1).unwrap();
    let prefix = &line[..line.find("missing").unwrap()];
    let column = prefix.chars().count() + 1;
    scratch.write("unicode.was", source);
    let location = format!("unicode.was:2:{column}: semantic error:");
    let stderr = failure(
        scratch.cli(&["unicode.was"], None),
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
    scratch.write("eof.was", "fn main() -> void {\n");
    let stderr = failure(
        scratch.cli(&["eof.was"], None),
        1,
        &["eof.was:2:1: parse error:", "2 |", "^"],
    );
    assert!(stderr.starts_with("eof.was:2:1:"));
    scratch.assert_entries(&["unicode.was", "eof.was"]);
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
    let overflow_source = format!("fn main() -> number {{ return {}; }}", "9".repeat(400));
    let cases: &[(&str, &[&str], &str)] = &[
        (SIMPLE_SOURCE, &["--host", "bad"], "host registration error"),
        (
            "fn main() -> void { let n: number = ; }",
            &[],
            "parse error:",
        ),
        ("fn main() -> void { missing(); }", &[], "semantic error:"),
        (overflow_source.as_str(), &[], "code generation error:"),
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
fn scalar_memory_and_allocator_names_are_valid_raw_exports() {
    let scratch = Scratch::new("scalar_abi_names");
    for name in ["memory", "__alloc"] {
        let source = format!("fn {name}() -> number {{ return 7; }}");
        let bytes = scratch.compile(&source, &[]);
        assert_eq!(bytes, raw_wasm(&source));
        assert_eq!(
            decode(&bytes).exports,
            [(name.to_owned(), ExternalKind::Func, 0)]
        );
        let wat = success(scratch.cli(&["program.was", "--emit", "wat"], None));
        assert_eq!(assert_plain_wat(&wat), raw_wat(&source));
        assert_eq!(scratch.compile(&source, &["--export", name]), bytes);
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::None);
        let none = scratch.compile(&source, &["--no-exports"]);
        assert_eq!(none, compiler.compile_source(&source).unwrap());
        assert!(decode(&none).exports.is_empty());
        scratch.assert_entries(&["program.was", "program.wasm"]);
    }
}

#[test]
fn memory_abi_name_conflicts_are_codegen_errors_only_when_selected() {
    let scratch = Scratch::new("abi_conflicts");
    let sentinel = b"existing artifact";
    for name in ["memory", "__alloc"] {
        let source = format!(
            "fn {name}() -> number {{ return 7; }} fn main() -> string {{ return \"text\"; }}"
        );
        scratch.write("program.was", &source);
        scratch.write("program.wasm", sentinel);
        for flags in [vec![], vec!["--export", name]] {
            for format in ["wasm", "wat"] {
                let mut args = vec!["program.was", "-o", "program.wasm", "--emit", format];
                args.extend_from_slice(&flags);
                let stderr = failure(
                    scratch.cli(&args, None),
                    1,
                    &[
                        "code generation error:",
                        &format!("duplicate user function export name `{name}`"),
                    ],
                );
                assert!(stderr.starts_with("program.was: code generation error:"));
                assert!(!stderr.contains('^'));
                assert_eq!(fs::read(scratch.0.join("program.wasm")).unwrap(), sentinel);
                assert_eq!(
                    fs::read(scratch.0.join("program.was")).unwrap(),
                    source.as_bytes()
                );
                scratch.assert_entries(&["program.was", "program.wasm"]);
            }
        }
        for (flags, selection) in [
            (
                vec!["--export", "main"],
                ExportOptions::Only(vec!["main".into()]),
            ),
            (vec!["--no-exports"], ExportOptions::None),
        ] {
            let mut compiler = Compiler::new();
            compiler.set_exports(selection);
            let bytes = scratch.compile(&source, &flags);
            assert_eq!(bytes, compiler.compile_source(&source).unwrap());
            assert_eq!(abi_exports(&decode(&bytes)).len(), 2);
        }
    }
}

#[test]
fn successful_compilation_overwrites_a_valid_artifact_without_leaking_files() {
    let scratch = Scratch::new("overwrite");
    let original = scratch.compile(SIMPLE_SOURCE, &[]);
    let source = "fn main() -> number { return 65; }";
    let replacement = scratch.compile(source, &[]);
    assert_ne!(original, replacement);
    assert_eq!(replacement, raw_wasm(source));
    decode(&replacement);
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
fn repeatable_host_declarations_preserve_raw_imports_in_registration_order() {
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
    let mut compiler = Compiler::new();
    for (name, module, field, params, result) in [
        ("print", "env", "log_number", vec![Type::Number], Type::Void),
        ("ready", "app", "ready", vec![], Type::Bool),
        ("bias", "env", "bias", vec![Type::Number], Type::Number),
        ("echo", "env", "echo", vec![Type::String], Type::String),
        (
            "edit",
            "env",
            "edit",
            vec![Type::NumberArray],
            Type::NumberArray,
        ),
        (
            "idle",
            "env",
            "unused",
            vec![Type::Bool, Type::String, Type::NumberArray],
            Type::Void,
        ),
    ] {
        compiler
            .register_host_function(name, module, field, params, result)
            .unwrap();
    }
    let bytes = scratch.compile(source, &hosts);
    assert_eq!(bytes, compiler.compile_source(source).unwrap());
    let all = decode(&bytes);
    assert_eq!(user_exports(&all), ["main", "text", "values"]);
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
    compiler.set_exports(ExportOptions::Only(vec!["values".into(), "main".into()]));
    let bytes = scratch.compile(source, &selected_flags);
    assert_eq!(bytes, compiler.compile_source(source).unwrap());
    let only = decode(&bytes);
    assert_eq!(user_exports(&only), ["main", "values"]);
    let mut none_flags = hosts.to_vec();
    none_flags.push("--no-exports");
    compiler.set_exports(ExportOptions::None);
    let bytes = scratch.compile(source, &none_flags);
    assert_eq!(bytes, compiler.compile_source(source).unwrap());
    let none = decode(&bytes);
    assert!(user_exports(&none).is_empty());
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
fn loop_is_reserved_only_as_a_host_language_name() {
    let scratch = Scratch::new("loop_host_names");
    let source = "fn main() -> number { return loop { break run(); }; }";
    let mut compiler = Compiler::new();
    assert!(matches!(
        compiler.register_host_function("loop", "env", "run", vec![], Type::Number),
        Err(was_compiler::HostRegistrationError::ReservedName { name }) if name == "loop"
    ));
    compiler
        .register_host_function("run", "loop", "loop", vec![], Type::Number)
        .unwrap();
    let bytes = scratch.compile(source, &["--host", "run=loop.loop()->number"]);
    assert_eq!(bytes, compiler.compile_source(source).unwrap());
    assert_eq!(decode(&bytes).imports, [("loop".into(), "loop".into())]);

    compiler
        .register_host_function(
            "arbitrary",
            "module with spaces/λ",
            "field:with,delimiters",
            vec![],
            Type::Void,
        )
        .unwrap();
    assert_eq!(
        decode(&compiler.compile_source(source).unwrap()).imports,
        [
            ("loop".into(), "loop".into()),
            (
                "module with spaces/λ".into(),
                "field:with,delimiters".into()
            ),
        ]
    );
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
        ("print=env.log_number(range)->void", "unknown type"),
        ("print=env.log_number()->range", "unknown type"),
        ("print=env.log_number(void)->void", "unsupported ABI type"),
        ("print=.log_number(number)->void", "module cannot be empty"),
        ("print=env.(number)->void", "field cannot be empty"),
        (
            "1print=env.log_number(number)->void",
            "Invalid host function name",
        ),
        ("len=env.len(number[])->number", "reserved"),
        ("loop=env.run()->void", "reserved"),
        ("range=env.run()->void", "reserved"),
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
    assert_eq!(
        scratch.compile(EXAMPLE_SOURCE, &[]),
        raw_wasm(EXAMPLE_SOURCE)
    );
    assert_eq!(text(scratch.node(&["program.wasm"])), "65\n");
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_entry_and_json_arguments_use_raw_numeric_scalars_and_handles() {
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
fn literal() -> string { return "fixture"; }
fn values(value: number[]) -> number[] { return value; }
fn make_values() -> number[] { return [1, 2]; }
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
            "literal",
            "--export",
            "values",
            "--export",
            "make_values",
        ],
    );
    for (entry, args, expected) in [
        ("calculate", "[21,1]", "42\n"),
        ("calculate", "[21,0]", "21\n"),
        ("invert", "[1]", "0\n"),
        ("invert", "[0]", "1\n"),
        ("nothing", "[]", ""),
    ] {
        assert_eq!(
            text(scratch.node(&["program.wasm", "--entry", entry, "--args", args])),
            expected
        );
    }
    let handle = text(scratch.node_script(
        r#"
const fs = require("node:fs");
const module = new WebAssembly.Module(fs.readFileSync("program.wasm"));
const {exports} = new WebAssembly.Instance(module);
console.log(exports.literal());
"#,
    ));
    let handle: u32 = handle.trim().parse().expect("static string handle");
    let args = format!("[{handle}]");
    assert_eq!(
        text(scratch.node(&["program.wasm", "--entry", "text", "--args", &args])),
        format!("{handle}\n")
    );
    for (entry, args) in [("text", "[1234]"), ("values", "[5678]")] {
        failure(
            scratch.node(&["program.wasm", "--entry", entry, "--args", args]),
            1,
            &["invocation:", "unreachable"],
        );
    }
    assert!(
        success(scratch.node_script(
            r#"
const assert = require("node:assert/strict");
const fs = require("node:fs");
const module = new WebAssembly.Module(fs.readFileSync("program.wasm"));
const {exports} = new WebAssembly.Instance(module);
assert.equal(exports.helper, undefined);
assert.equal(exports.main, undefined);
assert.equal(exports.calculate(21, 1), 42);
assert.equal(exports.invert(1), 0);
assert.equal(exports.nothing(), undefined);
const textHandle = exports.literal();
const arrayHandle = exports.make_values();
assert.equal(typeof textHandle, "number");
assert.equal(typeof arrayHandle, "number");
assert.equal(exports.text(textHandle), textHandle);
assert.equal(exports.values(arrayHandle), arrayHandle);
assert.throws(() => exports.text(1234), WebAssembly.RuntimeError);
assert.throws(() => exports.values(5678), WebAssembly.RuntimeError);
"#
        ))
        .is_empty()
    );
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_cli_raw_numeric_hosts_accept_unwrapped_callbacks() {
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
    scratch.write(
        "provider.cjs",
        r#"
const assert = require("node:assert/strict");
module.exports = {env: {
    log_number(value) { assert.equal(typeof value, "number"); assert.equal(value, 40); return 99; },
    ready() { return 1; },
    bias(value) { assert.equal(typeof value, "number"); return value + 2; }
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
    assert!(success(scratch.node_script(r#"
const assert = require("node:assert/strict");
const fs = require("node:fs");
const module = new WebAssembly.Module(fs.readFileSync("program.wasm"));
assert.throws(() => new WebAssembly.Instance(module), TypeError);
assert.throws(() => new WebAssembly.Instance(module, {env: {log_number() {}}}), WebAssembly.LinkError);
const {exports} = new WebAssembly.Instance(module, require("./provider.cjs"));
assert.equal(exports.main(40), 42);
"#)).is_empty());
    scratch.assert_entries(&["program.was", "program.wasm", "provider.cjs"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_embedding_string_and_array_hosts_receive_and_return_raw_handles() {
    let scratch = Scratch::new("node_memory_hosts");
    scratch.compile(
        r#"
fn text() -> string { return echo("héllo 🦀"); }
fn edited() -> number[] {
    let original: number[] = [1, 2];
    let result: number[] = edit(original);
    result[0] = result[0] + 1;
    return original;
}
fn first(values: number[]) -> number { return values[0]; }
fn count(values: number[]) -> number { return len(values); }
"#,
        &[
            "--host",
            "echo=env.echo(string)->string",
            "--host",
            "edit=env.edit(number[])->number[]",
        ],
    );
    assert!(
        success(scratch.node_script(
            r#"
const assert = require("node:assert/strict");
const fs = require("node:fs");
const module = new WebAssembly.Module(fs.readFileSync("program.wasm"));
let textHandle;
let arrayHandle;
const {exports} = new WebAssembly.Instance(module, {env: {
    echo(handle) { assert.equal(typeof handle, "number"); textHandle = handle; return handle; },
    edit(handle) { assert.equal(typeof handle, "number"); arrayHandle = handle; return handle; }
}});
assert.equal(exports.text(), textHandle);
const result = exports.edited();
assert.equal(result, arrayHandle);
assert.equal(exports.first(result), 2);
assert.equal(exports.count(result), 2);
assert(exports.memory instanceof WebAssembly.Memory);
assert.equal(typeof exports.__alloc, "function");
"#
        ))
        .is_empty()
    );
    scratch.assert_entries(&["program.was", "program.wasm"]);
}

#[test]
#[ignore = "requires Node.js"]
fn node_embedding_array_boundaries_trap_for_raw_handles() {
    let scratch = Scratch::new("node_bounds");
    scratch.compile(
        "fn values() -> number[] { return [4, 5]; } fn empty() -> number[] { return []; } fn load(values: number[], index: number) -> number { return values[index]; } fn store(values: number[], index: number) -> number { values[index] = 9; return values[0]; }",
        &[],
    );
    assert!(
        success(scratch.node_script(
            r#"
const assert = require("node:assert/strict");
const fs = require("node:fs");
const module = new WebAssembly.Module(fs.readFileSync("program.wasm"));
const {exports} = new WebAssembly.Instance(module);
const values = exports.values();
assert.equal(typeof values, "number");
assert.equal(exports.load(values, 1), 5);
assert.equal(exports.store(values, 1), 4);
assert.equal(exports.load(values, 1), 9);
for (const entry of ["load", "store"]) {
    for (const index of [-1, 2, 0.5, 4294967296, NaN, Infinity]) {
        assert.throws(() => exports[entry](values, index), WebAssembly.RuntimeError);
    }
    assert.throws(() => exports[entry](exports.empty(), 0), WebAssembly.RuntimeError);
}
"#
        ))
        .is_empty()
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
