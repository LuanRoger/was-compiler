#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use was_compiler::{
    ArrayElementType, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic,
};
use wasmparser::Validator;

const SOURCE: &str = r#"
fn flags(value: bool[]) -> bool[] { return host_flags(value); }
fn texts(value: string[]) -> string[] { return host_texts(value); }
fn private_range() -> range { return 0..2; }
fn choose(words: string[], flags: bool[]) -> number { return len(words) + len(flags); }
fn empty() -> string[] { return if true { [] } else { loop { break []; } }; }
fn main() -> number {
    let flags: bool[] = [true, false];
    let words: string[] = ["é", "z"];
    let alias = words;
    let update = () => { alias[1] = "λ"; return flags[0]; };
    let score = 0;
    for flag in flags {
        let read = () => { return flag; };
        if read() { score += 1; }
    }
    for word in words {
        let read = () => { return word; };
        if read() == "é" { score += 2; }
    }
    if update() && words[1] == "λ" { score += 4; }
    score += choose(flags: [], words: []);
    score += words |> choose(flags: [true]);
    let nothing: bool[] = if true { [] } else { [] };
    return score + len(nothing) + len(empty());
}
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    for (name, element) in [
        ("host_flags", ArrayElementType::Bool),
        ("host_texts", ArrayElementType::String),
    ] {
        let ty = Type::Array(element);
        compiler
            .register_host_function(name, "env", name, vec![ty], ty)
            .unwrap();
    }
    compiler.set_exports(ExportOptions::Only(vec![
        "main".into(),
        "flags".into(),
        "texts".into(),
    ]));
    compiler
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("was-typed-arrays-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn cli(&self, args: &[&str]) -> Output {
        test_process::output(
            Command::new(env!("CARGO_BIN_EXE_was-compiler"))
                .current_dir(&self.0)
                .args(args),
            None,
            Duration::from_secs(20),
        )
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
const HOSTS: [&str; 4] = [
    "--host",
    "host_flags=env.host_flags(bool[])->bool[]",
    "--host",
    "host_texts=env.host_texts(string[])->string[]",
];

#[test]
fn cli_typed_array_hosts_selected_exports_and_contextual_source_match_core() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("input.was"), SOURCE).unwrap();
    let mut native = compiler();
    for (level, optimization) in [
        ("-O0", OptimizationLevel::None),
        ("-O1", OptimizationLevel::Basic),
        ("-O2", OptimizationLevel::Standard),
        ("-O3", OptimizationLevel::Aggressive),
    ] {
        native.set_optimization_level(optimization);
        for names in [vec!["main", "flags", "texts"], vec![]] {
            native.set_exports(ExportOptions::Only(
                names.iter().map(|s| (*s).into()).collect(),
            ));
            let expected = native.compile_source(SOURCE).unwrap();
            Validator::new().validate_all(&expected).unwrap();
            let mut args = vec!["input.was", "-o", "output.wasm", level];
            args.extend(HOSTS);
            if names.is_empty() {
                args.push("--no-exports");
            }
            for name in names {
                args.extend(["--export", name]);
            }
            let output = scratch.cli(&args);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(fs::read(scratch.0.join("output.wasm")).unwrap(), expected);
            args.extend(["--emit", "wat"]);
            args[2] = "output.wat";
            let output = scratch.cli(&args);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(scratch.0.join("output.wat")).unwrap(),
                wasmprinter::print_bytes(&expected).unwrap()
            );
        }
    }
}

#[test]
fn cli_typed_array_diagnostics_match_core_without_touching_output() {
    let scratch = Scratch::new();
    for source in [
        "fn main() { let a = [true, 1]; }",
        "fn take(a: bool[]) {} fn main() { [] |> take; }",
        "fn make() { return []; } fn main() { let a: string[] = make(); }",
        "fn main() { for value in [true] { value = false; } }",
        "fn main() { let a = [\"é\"]; a[0] += \"x\"; }",
        "fn main() { let a: bool[][] = []; }",
    ] {
        fs::write(scratch.0.join("input.was"), source).unwrap();
        fs::write(scratch.0.join("output.wasm"), b"unchanged").unwrap();
        let error = Compiler::new().compile_source(source).unwrap_err();
        let expected = diagnostic::render("input.was", source, &error);
        let output = scratch.cli(&["input.was", "-o", "output.wasm"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("{expected}\n")
        );
        assert_eq!(
            fs::read(scratch.0.join("output.wasm")).unwrap(),
            b"unchanged"
        );
    }
}

#[test]
fn cli_supported_type_guidance_keeps_range_restrictions_independent() {
    let scratch = Scratch::new();
    let help = scratch.cli(&["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("bool[]"));
    assert!(help.contains("string[]"));
    fs::write(scratch.0.join("input.was"), "fn main() {}").unwrap();
    for declaration in ["bad=env.bad(range)->void", "bad=env.bad()->range"] {
        let output = scratch.cli(&["input.was", "--host", declaration]);
        assert_eq!(output.status.code(), Some(1));
        let message = String::from_utf8(output.stderr).unwrap();
        assert!(message.contains("unsupported ABI type Range"));
        assert!(message.contains("range values are private"));
        assert!(message.contains("bool[], string[]"));
    }
    for ty in ["bool[][]", "string[][]", "range[]", "void[]", "unknown"] {
        let declaration = format!("bad=env.bad({ty})->void");
        let output = scratch.cli(&["input.was", "--host", &declaration]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains(&format!("unknown type {ty:?}"))
        );
    }
    fs::write(scratch.0.join("input.was"), SOURCE).unwrap();
    let mut args = vec!["input.was"];
    args.extend(HOSTS);
    let output = scratch.cli(&args);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--export NAME")
    );
}

#[test]
#[ignore = "requires Node.js"]
fn cli_typed_arrays_execute_with_raw_host_layouts() {
    let scratch = Scratch::new();
    fs::write(scratch.0.join("input.was"), SOURCE).unwrap();
    let mut args = vec![
        "input.was",
        "-o",
        "output.wasm",
        "--export",
        "main",
        "--export",
        "flags",
        "--export",
        "texts",
    ];
    args.extend(HOSTS);
    let output = scratch.cli(&args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let script = r#"
        const assert = require('node:assert/strict');
        const module = new WebAssembly.Module(require('node:fs').readFileSync('output.wasm'));
        assert.deepEqual(WebAssembly.Module.imports(module), [
            {module:'env', name:'host_flags', kind:'function'},
            {module:'env', name:'host_texts', kind:'function'}]);
        let calls = [];
        const e = new WebAssembly.Instance(module, {env:{
            host_flags(h) {calls.push('bool'); return h;},
            host_texts(h) {calls.push('string'); return h;}
        }}).exports;
        assert.equal(e.main(), 10);
        const bool = e.__alloc(16), text = e.__alloc(16);
        let v = new DataView(e.memory.buffer);
        v.setUint32(bool, bool+8, true); v.setUint32(bool+4, 2, true);
        v.setUint8(bool+8, 1); v.setUint8(bool+9, 0);
        v.setUint32(text, text+8, true); v.setUint32(text+4, 0, true);
        assert.equal(e.flags(bool), bool); assert.equal(e.texts(text), text);
        assert.deepEqual(calls, ['bool','string']);
        assert.ok(!('private_range' in e));
    "#;
    let output = test_process::output(
        Command::new("node")
            .args(["-e", script])
            .current_dir(&scratch.0),
        None,
        Duration::from_secs(20),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
