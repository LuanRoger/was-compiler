use std::{
    borrow::Cow,
    fmt::Write as _,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use was_compiler::{Compiler, ExportOptions, Type, TypedProgram};
use wasm_encoder::{CustomSection, Module, RawSection};
use wasmparser::{Parser, Payload, Validator};

const INTERFACE_SECTION: &str = "was-compiler.interface";
const NODE_TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            ch if ch <= '\u{1f}' => write!(result, "\\u{:04x}", ch as u32).unwrap(),
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn type_name(ty: Type) -> &'static str {
    match ty {
        Type::Number => "number",
        Type::Bool => "bool",
        Type::String => "string",
        Type::NumberArray => "number[]",
        Type::Void => "void",
    }
}

fn signature_json(params: impl Iterator<Item = Type>, result: Type) -> String {
    let params = params
        .map(|ty| json_string(type_name(ty)))
        .collect::<Vec<_>>();
    format!(
        "\"params\":[{}],\"result\":{}",
        params.join(","),
        json_string(type_name(result))
    )
}

fn interface_json(program: &TypedProgram, selection: &ExportOptions) -> String {
    let exports = program
        .functions
        .iter()
        .filter(|function| match selection {
            ExportOptions::All => true,
            ExportOptions::None => false,
            ExportOptions::Only(names) => names.contains(&function.name),
        })
        .map(|function| {
            format!(
                "{{\"name\":{},{}}}",
                json_string(&function.name),
                signature_json(
                    function.params.iter().map(|param| param.ty),
                    function.return_type
                )
            )
        })
        .collect::<Vec<_>>();
    let imports = program
        .host_functions
        .iter()
        .map(|host| {
            format!(
                "{{\"name\":{},\"module\":{},\"field\":{},{}}}",
                json_string(&host.name),
                json_string(&host.module),
                json_string(&host.field),
                signature_json(host.params.iter().copied(), host.return_type)
            )
        })
        .collect::<Vec<_>>();
    format!(
        "{{\"version\":1,\"memoryAbi\":\"descriptor-v1\",\"exports\":[{}],\"imports\":[{}]}}",
        exports.join(","),
        imports.join(",")
    )
}

fn append_metadata(bytes: &[u8], metadata: &[u8]) -> Vec<u8> {
    let mut module = Module::new();
    for payload in Parser::new(0).parse_all(bytes) {
        if let Some((id, range)) = payload.unwrap().as_section() {
            let start = usize::try_from(range.start).expect("section start fits usize");
            let end = usize::try_from(range.end).expect("section end fits usize");
            module.section(&RawSection {
                id,
                data: &bytes[start..end],
            });
        }
    }
    module.section(&CustomSection {
        name: Cow::Borrowed(INTERFACE_SECTION),
        data: Cow::Borrowed(metadata),
    });
    module.finish()
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let program = compiler.analyze(source).expect("analyze runtime source");
    let bytes = compiler.compile(&program).expect("compile runtime source");
    let metadata = interface_json(&program, compiler.exports());
    let bytes = append_metadata(&bytes, metadata.as_bytes());
    Validator::new()
        .validate_all(&bytes)
        .expect("valid runtime fixture");
    bytes
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "was-compiler-runtime-{}-{timestamp}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create runtime test directory");
        Self(path)
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).expect("write runtime fixture");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn node_output(command: &mut Command) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Node.js; ignored runtime tests require node on PATH");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
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
        if let Some(status) = child.try_wait().expect("poll Node.js") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed out Node.js");
            timed_out = true;
            break child.wait().expect("wait for timed out Node.js");
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
        "Node.js exceeded {NODE_TIMEOUT:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n")
}

fn failure(output: Output, stage: &str, expected: &str) {
    assert!(!output.status.success(), "runner unexpectedly succeeded");
    assert!(
        output.stdout.is_empty(),
        "runner printed a result on failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with(&format!("{stage}: ")),
        "wrong stage: {stderr}"
    );
    assert!(
        stderr.contains(expected),
        "expected {expected:?} in {stderr}"
    );
}

const NODE_PRELUDE: &str = r#"
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { instantiate } = require('./runtime/index.cjs');
const wasm = fs.readFileSync(process.argv[1]);
function fails(stage, pattern, action) {
    assert.throws(action, error => {
        assert.equal(error.stage, stage, error.message);
        assert.match(error.message, pattern);
        return true;
    });
}
function leb(value) {
    const bytes = [];
    do {
        const byte = value % 128;
        value = Math.floor(value / 128);
        bytes.push(byte | (value ? 128 : 0));
    } while (value);
    return Buffer.from(bytes);
}
function custom(bytes, data) {
    const name = Buffer.from('was-compiler.interface');
    if (typeof data === 'string') data = Buffer.from(data);
    const payload = Buffer.concat([leb(name.length), name, data]);
    return Buffer.concat([bytes, Buffer.from([0]), leb(payload.length), payload]);
}
"#;

fn script(bytes: &[u8], body: &str) {
    let scratch = Scratch::new();
    let wasm = scratch.write("program.wasm", bytes);
    let source = format!("{NODE_PRELUDE}\n{body}");
    let mut command = Command::new("node");
    command.current_dir(root()).args(["-e", &source]).arg(wasm);
    assert_eq!(success(node_output(&mut command)), "");
}

fn runner(directory: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("node");
    command
        .current_dir(directory)
        .arg(root().join("runtime/run.cjs"))
        .args(args);
    node_output(&mut command)
}

#[test]
fn metadata_fixtures_use_real_typed_program_fields_and_export_selection() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("print", "env", "log_number", vec![Type::Number], Type::Void)
        .unwrap();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let bytes = compile(
        &compiler,
        "fn main(x: number) -> void { print(x); } fn hidden() -> string { return \"hidden\"; }",
    );
    let sections = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CustomSection(section) if section.name() == INTERFACE_SECTION => {
                Some(section.data().to_vec())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sections.len(), 1);
    assert_eq!(
        String::from_utf8(sections[0].clone()).unwrap(),
        "{\"version\":1,\"memoryAbi\":\"descriptor-v1\",\"exports\":[{\"name\":\"main\",\"params\":[\"number\"],\"result\":\"void\"}],\"imports\":[{\"name\":\"print\",\"module\":\"env\",\"field\":\"log_number\",\"params\":[\"number\"],\"result\":\"void\"}]}"
    );
    assert_eq!(
        json_string("\"\\\n\r\t\0é"),
        "\"\\\"\\\\\\n\\r\\t\\u0000é\""
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_library_and_cli_invocation_arguments() {
    let bytes = compile(
        &Compiler::new(),
        r#"
        fn main() -> number { return 7; }
        fn calculate(value: number, enabled: bool) -> number {
            if enabled { return value * 2; } else { return value; }
        }
        fn invert(value: bool) -> bool { return !value; }
        fn nothing() -> void {}
    "#,
    );
    script(
        &bytes,
        r#"
        const runtime = instantiate(wasm);
        assert(runtime.instance instanceof WebAssembly.Instance);
        assert.equal(runtime.call('main'), 7);
        assert.equal(runtime.call('calculate', [21, true]), 42);
        assert.equal(runtime.call('calculate', [Infinity, true]), Infinity);
        assert.equal(runtime.call('calculate', [-Infinity, true]), -Infinity);
        assert(Number.isNaN(runtime.call('calculate', [NaN, true])));
        assert.equal(runtime.call('invert', [true]), false);
        assert.equal(runtime.call('nothing'), undefined);
        assert.equal(runtime.interface.version, 1);
        assert.equal(runtime.interface.memoryAbi, 'descriptor-v1');
        assert.deepEqual(runtime.interface.exports.map(e => e.name), ['main', 'calculate', 'invert', 'nothing']);
        assert(Object.isFrozen(runtime.interface));
        assert(Object.isFrozen(runtime.interface.exports));
        assert(Object.isFrozen(runtime.interface.exports[1]));
        assert(Object.isFrozen(runtime.interface.exports[1].params));
        assert.throws(() => { runtime.interface.exports[1].params[0] = 'bool'; }, TypeError);
        assert.throws(() => { runtime.interface.exports[1].name = 'missing'; }, TypeError);
        assert.equal(runtime.call('calculate', [4, true]), 8);
        require('./runtime/run.cjs');
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    assert_eq!(success(runner(&scratch.0, &["program.wasm"])), "7\n");
    assert_eq!(
        success(runner(
            &scratch.0,
            &[
                "program.wasm",
                "--entry",
                "calculate",
                "--args",
                "[21,true]"
            ]
        )),
        "42\n"
    );
    assert_eq!(
        success(runner(
            &scratch.0,
            &["program.wasm", "--entry", "invert", "--args", "[true]"]
        )),
        "false\n"
    );
    assert_eq!(
        success(runner(&scratch.0, &["program.wasm", "--entry", "nothing"])),
        ""
    );
    assert!(
        success(runner(&scratch.0, &["--help"]))
            .starts_with("Usage: node runtime/run.cjs PROGRAM.wasm")
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_native_imports_and_missing_providers() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("print", "env", "log_number", vec![Type::Number], Type::Void)
        .unwrap();
    compiler
        .register_host_function("ready", "env", "ready", vec![], Type::Bool)
        .unwrap();
    compiler
        .register_host_function("bias", "env", "bias", vec![Type::Number], Type::Number)
        .unwrap();
    let bytes = compile(
        &compiler,
        r#"
        fn main(value: number) -> number {
            print(value);
            if ready() { return bias(value); } else { return value; }
        }
    "#,
    );
    script(
        &bytes,
        r#"
        fails('instantiation', /env\.log_number \(number\)->void/, () => instantiate(wasm));
        fails('instantiation', /noncallable.*env\.log_number/, () => instantiate(wasm, {env: {log_number: 7}}));
        const inherited = Object.create({env: {log_number() {}}});
        fails('instantiation', /env\.log_number/, () => instantiate(wasm, inherited));
        fails('instantiation', /nativeImports must be an object/, () => instantiate(wasm, null));
        const seen = [];
        const provider = {env: {
            log_number(value) { assert.equal(typeof value, 'number'); seen.push(value); },
            ready() { return true; },
            bias(value) { return value + 2; }
        }};
        const runtime = instantiate(wasm, provider);
        assert.equal(runtime.call('main', [40]), 42);
        assert.deepEqual(seen, [40]);
        provider.env.ready = () => 1;
        fails('invocation', /return value.*env\.ready.*must be bool/, () => instantiate(wasm, provider).call('main', [1]));
        provider.env.ready = () => false;
        provider.env.log_number = () => 99;
        fails('invocation', /return value.*env\.log_number.*void/, () => instantiate(wasm, provider).call('main', [1]));
        provider.env.log_number = () => { throw new Error('provider exploded'); };
        fails('invocation', /env\.log_number.*provider exploded/, () => instantiate(wasm, provider).call('main', [1]));
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    scratch.write("provider.cjs", "module.exports = {env: {log_number(value) { if (typeof value !== 'number') throw Error('bad number'); }, ready() { return true; }, bias(value) { return value + 2; }}};");
    failure(
        runner(&scratch.0, &["program.wasm", "--args", "[40]"]),
        "instantiation",
        "env.log_number (number)->void",
    );
    assert_eq!(
        success(runner(
            &scratch.0,
            &[
                "program.wasm",
                "--args",
                "[40]",
                "--imports",
                "provider.cjs"
            ]
        )),
        "42\n"
    );
    scratch.write("invalid.cjs", "module.exports = {env: {log_number: 42}};");
    failure(
        runner(&scratch.0, &["program.wasm", "--imports", "invalid.cjs"]),
        "instantiation",
        "env.log_number",
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_malformed_binary_and_metadata_versions_and_schema() {
    let compiler = Compiler::new();
    let program = compiler
        .analyze("fn main() -> number { return 1; }")
        .unwrap();
    let raw = compiler.compile(&program).unwrap();
    let metadata = interface_json(&program, compiler.exports());
    script(
        &raw,
        &format!(
            r#"
        const metadata = {metadata};
        const pack = value => custom(wasm, JSON.stringify(value));
        fails('compilation', /compile WebAssembly/, () => instantiate(Buffer.from('not wasm')));
        fails('metadata', /exactly one.*found 0/, () => instantiate(wasm));
        const valid = pack(metadata);
        assert.equal(instantiate(valid).call('main'), 1);
        fails('metadata', /exactly one.*found 2/, () => instantiate(custom(valid, JSON.stringify(metadata))));
        fails('metadata', /version 2.*expected version 1/, () => instantiate(pack({{...metadata, version: 2}})));
        fails('metadata', /expected version 1/, () => instantiate(pack({{...metadata, version: '1'}})));
        fails('metadata', /memoryAbi.*descriptor-v1/, () => instantiate(pack({{...metadata, memoryAbi: 'other'}})));
        fails('metadata', /interface metadata.*fields/, () => instantiate(pack({{version: 1}})));
        fails('metadata', /exports must be an array/, () => instantiate(pack({{...metadata, exports: null}})));
        fails('metadata', /imports must be an array/, () => instantiate(pack({{...metadata, imports: {{}}}})));
        fails('metadata', /void is only a result/, () => instantiate(pack({{...metadata, exports: [{{name: 'main', params: ['void'], result: 'number'}}]}})));
        fails('metadata', /result must be/, () => instantiate(pack({{...metadata, exports: [{{name: 'main', params: [], result: 'object'}}]}})));
        fails('metadata', /duplicate exports name/, () => instantiate(pack({{...metadata, exports: [metadata.exports[0], metadata.exports[0]]}})));
        fails('metadata', /ABI helper/, () => instantiate(pack({{...metadata, exports: [{{name: '__alloc', params: ['number'], result: 'number'}}]}})));
        fails('metadata', /ABI helper/, () => instantiate(pack({{...metadata, exports: [{{name: 'memory', params: [], result: 'void'}}]}})));
        fails('metadata', /invalid interface metadata/, () => instantiate(custom(wasm, '{{')));
        fails('metadata', /invalid interface metadata/, () => instantiate(custom(wasm, Buffer.from([0xc3, 0x28]))));
        fails('metadata', /metadata exceeds/, () => instantiate(custom(wasm, Buffer.alloc(1024 * 1024 + 1, 32))));
        fails('metadata', /at most 1024/, () => instantiate(pack({{...metadata, exports: [{{name: 'main', params: new Array(1025).fill('number'), result: 'number'}}]}})));
    "#
        ),
    );
    let scratch = Scratch::new();
    scratch.write("bad.wasm", b"not wasm");
    scratch.write("missing.wasm", &raw);
    scratch.write(
        "version.wasm",
        append_metadata(
            &raw,
            metadata
                .replace("\"version\":1", "\"version\":2")
                .as_bytes(),
        ),
    );
    scratch.write(
        "duplicate.wasm",
        append_metadata(
            &append_metadata(&raw, metadata.as_bytes()),
            metadata.as_bytes(),
        ),
    );
    failure(
        runner(&scratch.0, &["bad.wasm"]),
        "compilation",
        "WebAssembly",
    );
    failure(runner(&scratch.0, &["missing.wasm"]), "metadata", "found 0");
    failure(
        runner(&scratch.0, &["version.wasm"]),
        "metadata",
        "version 2",
    );
    failure(
        runner(&scratch.0, &["duplicate.wasm"]),
        "metadata",
        "found 2",
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_memory_roundtrips_copy_payloads_and_refresh_views_after_growth() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "edit",
            "env",
            "edit",
            vec![Type::NumberArray],
            Type::NumberArray,
        )
        .unwrap();
    compiler
        .register_host_function(
            "observe",
            "env",
            "observe",
            vec![Type::String],
            Type::String,
        )
        .unwrap();
    compiler
        .register_host_function("large", "env", "large", vec![], Type::NumberArray)
        .unwrap();
    let bytes = compile(
        &compiler,
        r#"
        fn text(value: string) -> string { return value; }
        fn values(value: number[]) -> number[] { value[0] = 9; return value; }
        fn empty(value: number[]) -> number[] { return value; }
        fn edited() -> number[] {
            let original: number[] = [1, 2];
            let result: number[] = edit(original);
            result[0] = result[0] + 1;
            return [original[0], result[0]];
        }
        fn literal() -> string { return "héllo 🦀"; }
        fn observed() -> string { return observe("héllo 🦀"); }
        fn grown() -> number[] { return large(); }
        fn lengths(value: number[]) -> number { return len(value); }
    "#,
    );
    script(
        &bytes,
        r#"
        let retained;
        const hostArray = new Array(20000).fill(2);
        const runtime = instantiate(wasm, {env: {
            edit(values) {
                assert.deepEqual(values, [1, 2]);
                retained = values;
                values[0] = 40;
                return values;
            },
            observe(value) {
                assert.equal(typeof value, 'string');
                assert.equal(value, 'héllo 🦀');
                return value + '!';
            },
            large() { return hostArray; }
        }});
        assert(!runtime.interface.exports.some(e => e.name === 'memory' || e.name === '__alloc'));
        for (const value of ['', 'héllo 🦀', '\u0000', '\ufeffBOM', 'x'.repeat(100000)]) {
            assert.equal(runtime.call('text', [value]), value);
        }
        assert.equal(runtime.call('observed'), 'héllo 🦀!');
        assert.equal(runtime.call('literal'), 'héllo 🦀');
        assert.equal(runtime.call('observed'), 'héllo 🦀!');
        const input = Object.freeze([1, 2]);
        assert.deepEqual(runtime.call('values', [input]), [9, 2]);
        assert.deepEqual(input, [1, 2]);
        assert.deepEqual(runtime.call('empty', [[]]), []);
        const specials = [NaN, Infinity, -Infinity, -0];
        assert.deepEqual(runtime.call('empty', [specials]), specials);
        assert.deepEqual(runtime.call('edited'), [1, 41]);
        assert.deepEqual(retained, [40, 2]);
        retained[0] = 100;
        assert.deepEqual(runtime.call('edited'), [1, 41]);
        const before = runtime.instance.exports.memory.buffer;
        const result = runtime.call('grown');
        assert.notEqual(runtime.instance.exports.memory.buffer, before);
        assert.equal(result.length, 20000);
        assert.equal(result[0], 2);
        result[0] = 99;
        assert.equal(hostArray[0], 2);
        assert.equal(runtime.call('grown')[0], 2);
        assert.equal(runtime.call('lengths', [new Array(25000).fill(1)]), 25000);
        fails('invocation', /not a language export/, () => runtime.call('__alloc', [8]));
        fails('invocation', /not a language export/, () => runtime.call('memory'));
        const wrong = instantiate(wasm, {env: {edit() { return new Float64Array([1]); }, observe() { return 'ok'; }, large() { return []; }}});
        fails('invocation', /return value.*number\[\]/, () => wrong.call('edited'));
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    scratch.write("provider.cjs", "module.exports = {env: {edit(values) { return values; }, observe(value) { return value; }, large() { return []; }}};");
    assert_eq!(
        success(runner(
            &scratch.0,
            &[
                "program.wasm",
                "--entry",
                "text",
                "--args",
                "[\"héllo 🦀\"]",
                "--imports",
                "provider.cjs"
            ]
        )),
        "héllo 🦀\n"
    );
    assert_eq!(
        success(runner(
            &scratch.0,
            &[
                "program.wasm",
                "--entry",
                "values",
                "--args",
                "[[1,2]]",
                "--imports",
                "provider.cjs"
            ]
        )),
        "[ 9, 2 ]\n"
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_typed_counts_and_bounds_traps() {
    let bytes = compile(
        &Compiler::new(),
        r#"
        fn main(values: number[], index: number) -> number { return values[index]; }
        fn flag(value: bool) -> bool { return value; }
        fn text(value: string) -> string { return value; }
        fn echo_number(value: number) -> number { return value; }
    "#,
    );
    script(
        &bytes,
        r#"
        const runtime = instantiate(wasm);
        assert.equal(runtime.call('main', [[4, 5], 1]), 5);
        fails('invocation', /expects 2 arguments, got 0/, () => runtime.call('main'));
        fails('invocation', /expects 2 arguments, got 3/, () => runtime.call('main', [[1], 0, 3]));
        fails('invocation', /must be an array/, () => runtime.call('main', '[]'));
        fails('invocation', /argument 1.*must be number\[\]/, () => runtime.call('main', [new Float64Array([1]), 0]));
        fails('invocation', /argument 1.*\[1\] must be number/, () => runtime.call('main', [[1, '2'], 0]));
        fails('invocation', /\[0\] must be number/, () => runtime.call('main', [new Array(1), 0]));
        fails('invocation', /argument 2.*must be number/, () => runtime.call('main', [[1], '0']));
        for (const value of [0, 1, 'true', null, new Boolean(true)]) {
            fails('invocation', /must be bool/, () => runtime.call('flag', [value]));
        }
        fails('invocation', /must be string/, () => runtime.call('text', [new String('text')]));
        fails('invocation', /must be number/, () => runtime.call('echo_number', [new Number(1)]));
        fails('invocation', /entry name must be a string/, () => runtime.call(1));
        fails('invocation', /not a language export/, () => runtime.call('missing'));
        for (const index of [-1, 1, 0.5, NaN, Infinity]) {
            fails('invocation', /cannot call.*main/, () => runtime.call('main', [[1], index]));
        }
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    failure(
        runner(&scratch.0, &["program.wasm"]),
        "invocation",
        "expects 2 arguments",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--args", "[[1],1]"]),
        "invocation",
        "main",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--args", "[[1],\"0\"]"]),
        "invocation",
        "must be number",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--args", "{}"]),
        "invocation",
        "JSON array",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--args", "["]),
        "invocation",
        "runner arguments",
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_export_selection_and_interface_mismatch() {
    let source = "fn main(value: number) -> number { return helper(value); } fn helper(value: number) -> number { return value * 2; } fn hidden() -> string { return \"hidden\"; }";
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let bytes = compile(&compiler, source);
    script(
        &bytes,
        r#"
        const runtime = instantiate(wasm);
        assert.deepEqual(runtime.interface.exports.map(e => e.name), ['main']);
        assert.equal(runtime.call('main', [21]), 42);
        assert.equal(runtime.instance.exports.helper, undefined);
        assert(runtime.instance.exports.memory instanceof WebAssembly.Memory);
        fails('invocation', /not a language export/, () => runtime.call('helper', [1]));
        fails('invocation', /ABI helpers/, () => runtime.call('__alloc', [8]));
    "#,
    );
    compiler.set_exports(ExportOptions::None);
    script(
        &compile(&compiler, source),
        r#"
        const runtime = instantiate(wasm);
        assert.deepEqual(runtime.interface.exports, []);
        assert(runtime.instance.exports.memory instanceof WebAssembly.Memory);
        fails('invocation', /not a language export/, () => runtime.call('main', [1]));
    "#,
    );
    compiler.set_exports(ExportOptions::All);
    let program = compiler.analyze(source).unwrap();
    let raw = compiler.compile(&program).unwrap();
    let metadata = interface_json(&program, compiler.exports());
    script(
        &raw,
        &format!(
            r#"
        const metadata = {metadata};
        fails('metadata', /module export main.*not declared/, () => instantiate(custom(wasm, JSON.stringify({{...metadata, exports: metadata.exports.slice(1)}}))));
        const extra = {{name: 'missing', params: [], result: 'void'}};
        fails('metadata', /metadata export missing.*missing/, () => instantiate(custom(wasm, JSON.stringify({{...metadata, exports: metadata.exports.concat([extra])}}))));
    "#
        ),
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_alias_imports_share_wrappers_and_reject_conflicting_signatures() {
    let mut compiler = Compiler::new();
    for name in ["left", "right"] {
        compiler
            .register_host_function(name, "env", "shared", vec![Type::Number], Type::Number)
            .unwrap();
    }
    let source = "fn main(value: number) -> number { return left(value) + right(value); }";
    let bytes = compile(&compiler, source);
    script(
        &bytes,
        r#"
        const originalInstance = WebAssembly.Instance;
        WebAssembly.Instance = class extends originalInstance {
            constructor(module, imports) {
                assert.equal(Object.getPrototypeOf(imports), null);
                assert.equal(Object.getPrototypeOf(imports.env), null);
                assert.deepEqual(Object.keys(imports.env), ['shared']);
                super(module, imports);
            }
        };
        let calls = 0;
        const runtime = instantiate(wasm, {env: {shared(value) { calls++; return value + 1; }}});
        assert.equal(runtime.call('main', [20]), 42);
        assert.equal(calls, 2);
        assert.deepEqual(runtime.interface.imports.map(e => e.name), ['left', 'right']);
        WebAssembly.Instance = originalInstance;
    "#,
    );
    let program = compiler.analyze(source).unwrap();
    let raw = compiler.compile(&program).unwrap();
    let metadata = interface_json(&program, compiler.exports());
    script(
        &raw,
        &format!(
            r#"
        const metadata = {metadata};
        const provider = {{env: {{shared(value) {{ return value; }}}}}};
        function pack(value) {{ return custom(wasm, JSON.stringify(value)); }}
        fails('metadata', /module import env\.shared.*fewer times/, () => instantiate(pack({{...metadata, imports: metadata.imports.slice(1)}}), provider));
        fails('metadata', /env\.shared.*import count/, () => instantiate(pack({{...metadata, imports: metadata.imports.concat([{{...metadata.imports[0], name: 'third'}}])}}), provider));
        fails('metadata', /module import env\.shared.*not declared/, () => instantiate(pack({{...metadata, imports: metadata.imports.map(e => ({{...e, field: 'other'}}))}}), provider));
        fails('metadata', /duplicate imports name/, () => instantiate(pack({{...metadata, imports: [metadata.imports[0], metadata.imports[0]]}}), provider));
        const conflicting = metadata.imports.map(e => ({{...e}}));
        conflicting[1].params = ['bool'];
        fails('metadata', /conflicting signatures.*env\.shared.*\(number\)->number.*\(bool\)->number/, () => instantiate(pack({{...metadata, imports: conflicting}}), provider));
        conflicting[1].params = ['number'];
        conflicting[1].result = 'bool';
        fails('metadata', /conflicting signatures.*env\.shared/, () => instantiate(pack({{...metadata, imports: conflicting}}), provider));
    "#
        ),
    );
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("left", "env", "shared", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("right", "env", "shared", vec![Type::Bool], Type::Bool)
        .unwrap();
    script(
        &compile(&compiler, "fn main() -> void { left(1); right(true); }"),
        r#"
        fails('metadata', /conflicting signatures.*env\.shared/, () => instantiate(wasm, {env: {shared(value) { return value; }}}));
    "#,
    );
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "special",
            "__proto__",
            "__proto__",
            vec![Type::Number],
            Type::Number,
        )
        .unwrap();
    script(
        &compile(&compiler, "fn main() -> number { return special(21); }"),
        r#"
        const originalInstance = WebAssembly.Instance;
        WebAssembly.Instance = class extends originalInstance {
            constructor(module, imports) {
                assert.equal(Object.getPrototypeOf(imports), null);
                assert.equal(Object.getPrototypeOf(imports.__proto__), null);
                super(module, imports);
            }
        };
        const provider = Object.create(null);
        provider.__proto__ = Object.create(null);
        provider.__proto__.__proto__ = value => value * 2;
        assert.equal(instantiate(wasm, provider).call('main'), 42);
        WebAssembly.Instance = originalInstance;
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_abi_descriptor_alignment_ranges_utf8_and_copy_limits() {
    let bytes = compile(&Compiler::new(), "fn main() -> void {}");
    script(
        &bytes,
        r#"
        const {createAbi, readDescriptor, MAX_COPY_BYTES} = require('./runtime/abi.cjs');
        const memory = new WebAssembly.Memory({initial: 1, maximum: 4});
        let heap = 8;
        const abi = createAbi(() => ({exports: {memory, __alloc(size) {
            const handle = heap;
            heap += Math.ceil(size / 8) * 8;
            if (heap > memory.buffer.byteLength) memory.grow(Math.ceil((heap - memory.buffer.byteLength) / 65536));
            return handle;
        }}}));
        assert.equal(abi.decode('string', abi.encode('string', '\ufeffhéllo 🦀')), '\ufeffhéllo 🦀');
        const input = Object.freeze([1, NaN, Infinity, -Infinity, -0]);
        const arrayHandle = abi.encode('number[]', input);
        const decoded = abi.decode('number[]', arrayHandle);
        assert.deepEqual(decoded, input);
        decoded[0] = 9;
        assert.equal(abi.decode('number[]', arrayHandle)[0], 1);
        assert.equal(input[0], 1);
        assert.equal(abi.decode('string', abi.encode('string', '')), '');
        assert.deepEqual(abi.decode('number[]', abi.encode('number[]', [])), []);
        const before = memory.buffer;
        const handle = abi.encode('string', 'x'.repeat(100000));
        assert.notEqual(memory.buffer, before);
        assert.equal(abi.decode('string', handle), 'x'.repeat(100000));
        for (const handle of [0, 1, 7, 9, 15]) {
            assert.throws(() => readDescriptor(memory, handle, 1), /descriptor/);
        }
        assert.throws(() => readDescriptor(memory, 8, 2), /width must be 1 or 8/);
        for (const handle of [0x100000000, -0x80000001, 1.5, NaN]) {
            assert.throws(() => readDescriptor(memory, handle, 1), /32-bit integer/);
        }
        assert.throws(() => readDescriptor(memory, -0x80000000, 1), /out of bounds/);
        assert.throws(() => readDescriptor(memory, memory.buffer.byteLength, 1), /out of bounds/);
        const view = new DataView(memory.buffer);
        const header = 8;
        function descriptor(data, length) {
            view.setUint32(header, data, true);
            view.setUint32(header + 4, length, true);
        }
        descriptor(0, 0);
        assert.throws(() => readDescriptor(memory, header, 1), /payload.*at least 8/);
        descriptor(17, 1);
        assert.throws(() => readDescriptor(memory, header, 8), /payload.*8-byte aligned/);
        descriptor(16, 0xffffffff);
        assert.throws(() => readDescriptor(memory, header, 8), /payload.*out of bounds/);
        descriptor(0xffffffff, 2);
        assert.throws(() => readDescriptor(memory, header, 1), /payload.*out of bounds/);
        descriptor(memory.buffer.byteLength, 1);
        assert.throws(() => readDescriptor(memory, header, 1), /payload.*out of bounds/);
        descriptor(17, 2);
        new Uint8Array(memory.buffer, 17, 2).set([0xc3, 0x28]);
        assert.throws(() => abi.decode('string', header), /encoded data/);
        descriptor(17, 2);
        new Uint8Array(memory.buffer, 17, 2).set([0xc0, 0xaf]);
        assert.throws(() => abi.decode('string', header), /encoded data/);
        descriptor(memory.buffer.byteLength, 0);
        assert.equal(abi.decode('string', header), '');
        const tooLarge = [];
        tooLarge.length = MAX_COPY_BYTES / 8 + 1;
        assert.throws(() => abi.encode('number[]', tooLarge), /copy limit/);
        assert.throws(() => abi.decode('bool', 2), /ABI bool/);
        assert.throws(() => abi.encode('void', null), /void/);
        assert.throws(() => abi.encode('number[]', new Float64Array([1])), /number\[\]/);
        const badAllocator = createAbi(() => ({exports: {memory, __alloc() { return 9; }}}));
        assert.throws(() => badAllocator.encode('string', 'value'), /allocator result.*aligned/);
        const overflowingAllocator = createAbi(() => ({exports: {memory, __alloc() { return memory.buffer.byteLength - 8; }}}));
        assert.throws(() => overflowingAllocator.encode('string', 'value'), /allocator result.*out of bounds/);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_start_callbacks_and_nonfunction_module_interfaces() {
    use wasm_encoder::{
        EntityType, ExportKind, ExportSection, ImportSection, MemorySection, MemoryType,
        StartSection, TypeSection,
    };
    let mut types = TypeSection::new();
    types.ty().function([], []);
    let mut imports = ImportSection::new();
    imports.import("env", "boot", EntityType::Function(0));
    let mut module = Module::new();
    module
        .section(&types)
        .section(&imports)
        .section(&StartSection { function_index: 0 });
    let metadata = b"{\"version\":1,\"memoryAbi\":\"descriptor-v1\",\"exports\":[],\"imports\":[{\"name\":\"boot\",\"module\":\"env\",\"field\":\"boot\",\"params\":[],\"result\":\"void\"}]}";
    script(
        &append_metadata(&module.finish(), metadata),
        r#"
        let called = false;
        fails('instantiation', /env\.boot.*Start function.*before the instance.*remove the Start section/, () => instantiate(wasm, {env: {boot() { called = true; }}}));
        assert.equal(called, false);
    "#,
    );
    let memory_type = MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: false,
        page_size_log2: None,
    };
    let empty_metadata =
        b"{\"version\":1,\"memoryAbi\":\"descriptor-v1\",\"exports\":[],\"imports\":[]}";
    let mut imports = ImportSection::new();
    imports.import("env", "memory", EntityType::Memory(memory_type));
    let mut module = Module::new();
    module.section(&imports);
    script(
        &append_metadata(&module.finish(), empty_metadata),
        r#"
        fails('metadata', /env\.memory.*only language function imports/, () => instantiate(wasm));
    "#,
    );
    let mut memories = MemorySection::new();
    memories.memory(memory_type);
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    let mut module = Module::new();
    module.section(&memories).section(&exports);
    script(
        &append_metadata(&module.finish(), empty_metadata),
        r#"
        fails('metadata', /must export both memory and __alloc/, () => instantiate(wasm));
    "#,
    );
    let mut exports = ExportSection::new();
    exports.export("extra", ExportKind::Memory, 0);
    let mut module = Module::new();
    module.section(&memories).section(&exports);
    script(
        &append_metadata(&module.finish(), empty_metadata),
        r#"
        fails('metadata', /export extra.*only language function exports/, () => instantiate(wasm));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_runner_file_errors_and_option_validation() {
    let bytes = compile(&Compiler::new(), "fn main() -> void {}");
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    scratch.write("broken.cjs", "throw new Error('broken provider');");
    failure(
        runner(&scratch.0, &["missing.wasm"]),
        "file",
        "missing.wasm",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--imports", "missing.cjs"]),
        "file",
        "missing.cjs",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--imports", "broken.cjs"]),
        "file",
        "broken provider",
    );
    failure(
        runner(&scratch.0, &[]),
        "invocation",
        "PROGRAM.wasm is required",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--unknown"]),
        "invocation",
        "unknown option",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--entry"]),
        "invocation",
        "requires a value",
    );
    failure(
        runner(
            &scratch.0,
            &["program.wasm", "--entry", "main", "--entry", "main"],
        ),
        "invocation",
        "duplicate option",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "extra.wasm"]),
        "invocation",
        "unknown option",
    );
    failure(
        runner(&scratch.0, &["program.wasm", "--entry", "__alloc"]),
        "invocation",
        "not a language export",
    );
}
