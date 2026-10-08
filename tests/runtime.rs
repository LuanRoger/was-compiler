use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use was_compiler::{Compiler, ExportOptions, Type};
use wasm_encoder::Module;
use wasmparser::{Parser, Payload, Validator};

const INTERFACE_SECTION: &str = "was-compiler.interface";
const NODE_TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let program = compiler.analyze(source).expect("analyze runtime source");
    let bytes = compiler.compile(&program).expect("compile runtime source");
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
fn runtime_fixtures_have_no_interface_metadata() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("print", "env", "log_number", vec![Type::Number], Type::Void)
        .unwrap();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let bytes = compile(
        &compiler,
        r#"fn main(x: number) -> void { print(x); } fn hidden() -> string { return "hidden"; }"#,
    );
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::CustomSection(section) = payload.unwrap() {
            assert_ne!(section.name(), INTERFACE_SECTION);
        }
    }
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
        assert.deepEqual(Object.keys(runtime), ['instance', 'call']);
        assert.equal(runtime.call('main'), 7);
        assert.equal(runtime.call('calculate', [21, 1]), 42);
        assert.equal(runtime.call('calculate', [21, 0]), 21);
        for (const value of [Infinity, -Infinity, NaN]) {
            assert(Object.is(runtime.call('calculate', [value, 1]), value));
        }
        assert.equal(runtime.call('invert', [1]), 0);
        assert.equal(runtime.call('nothing'), undefined);
        fails('invocation', /numeric scalar/, () => runtime.call('calculate', [21, true]));
        require('./runtime/run.cjs');
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    assert_eq!(success(runner(&scratch.0, &["program.wasm"])), "7\n");
    assert_eq!(
        success(runner(
            &scratch.0,
            &["program.wasm", "--entry", "calculate", "--args", "[21,1]"]
        )),
        "42\n"
    );
    assert_eq!(
        success(runner(
            &scratch.0,
            &["program.wasm", "--entry", "invert", "--args", "[1]"]
        )),
        "0\n"
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
fn node_raw_imports_and_missing_providers() {
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
        "fn main(value: number) -> number { print(value); if ready() { return bias(value); } else { return value; } }",
    );
    script(
        &bytes,
        r#"
        fails('instantiation', /env/, () => instantiate(wasm));
        fails('instantiation', /callable/, () => instantiate(wasm, {env: {log_number: 7}}));
        fails('instantiation', /must be an object/, () => instantiate(wasm, null));
        const seen = [];
        const provider = {env: {
            log_number(value) { seen.push(value); return 99; },
            ready() { return 1; },
            bias(value) { return value + 2; }
        }};
        const originalInstance = WebAssembly.Instance;
        WebAssembly.Instance = class extends originalInstance {
            constructor(module, imports) {
                assert.equal(imports, provider);
                super(module, imports);
            }
        };
        const runtime = instantiate(wasm, provider);
        WebAssembly.Instance = originalInstance;
        assert.equal(runtime.call('main', [40]), 42);
        assert.deepEqual(seen, [40]);
        provider.env.ready = () => 0;
        assert.equal(instantiate(wasm, provider).call('main', [40]), 40);
        provider.env.log_number = () => { throw new Error('provider exploded'); };
        fails('invocation', /provider exploded/, () => instantiate(wasm, provider).call('main', [1]));
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    scratch.write("provider.cjs", "module.exports = {env: {log_number() {}, ready() { return 1; }, bias(value) { return value + 2; }}};");
    failure(
        runner(&scratch.0, &["program.wasm", "--args", "[40]"]),
        "instantiation",
        "env",
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
        "callable",
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_malformed_binary_and_ignored_custom_sections() {
    let bytes = compile(&Compiler::new(), "fn main() -> number { return 1; }");
    script(
        &bytes,
        r#"
        fails('compilation', /compile WebAssembly/, () => instantiate(Buffer.from('not wasm')));
        assert.equal(WebAssembly.Module.customSections(new WebAssembly.Module(wasm), 'was-compiler.interface').length, 0);
        assert.equal(instantiate(wasm).call('main'), 1);
        const decorated = custom(custom(wasm, '{not JSON'), Buffer.from([0xff]));
        const originalCustomSections = WebAssembly.Module.customSections;
        WebAssembly.Module.customSections = () => { throw Error('runtime must not inspect metadata'); };
        try {
            const runtime = instantiate(decorated);
            assert.equal(runtime.call('main'), 1);
            assert.equal(Object.hasOwn(runtime, 'interface'), false);
        } finally {
            WebAssembly.Module.customSections = originalCustomSections;
        }
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("bad.wasm", b"not wasm");
    failure(
        runner(&scratch.0, &["bad.wasm"]),
        "compilation",
        "WebAssembly",
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_explicit_memory_roundtrips_and_raw_descriptor_imports() {
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
        const {createAbi} = require('./runtime/abi.cjs');
        let runtime;
        const abi = createAbi(() => runtime.instance);
        let retained;
        const hostArray = new Array(20000).fill(2);
        runtime = instantiate(wasm, {env: {
            edit(handle) {
                assert.equal(typeof handle, 'number');
                const values = abi.decode('number[]', handle);
                assert.deepEqual(values, [1, 2]);
                retained = values;
                values[0] = 40;
                return abi.encode('number[]', values);
            },
            observe(handle) {
                assert.equal(typeof handle, 'number');
                assert.equal(abi.decode('string', handle), 'héllo 🦀');
                return abi.encode('string', 'héllo 🦀!');
            },
            large() { return abi.encode('number[]', hostArray); }
        }});
        for (const value of ['', 'héllo 🦀', '\u0000', '\ufeffBOM', 'x'.repeat(100000)]) {
            const handle = abi.encode('string', value);
            assert.equal(runtime.call('text', [handle]), handle);
            assert.equal(abi.decode('string', handle), value);
        }
        assert.equal(abi.decode('string', runtime.call('observed')), 'héllo 🦀!');
        assert.equal(typeof runtime.call('literal'), 'number');
        assert.equal(abi.decode('string', runtime.call('literal')), 'héllo 🦀');
        const input = Object.freeze([1, 2]);
        assert.deepEqual(abi.decode('number[]', runtime.call('values', [abi.encode('number[]', input)])), [9, 2]);
        assert.deepEqual(input, [1, 2]);
        for (const value of [[], [NaN, Infinity, -Infinity, -0]]) {
            assert.deepEqual(abi.decode('number[]', runtime.call('empty', [abi.encode('number[]', value)])), value);
        }
        assert.deepEqual(abi.decode('number[]', runtime.call('edited')), [1, 41]);
        assert.deepEqual(retained, [40, 2]);
        retained[0] = 100;
        assert.deepEqual(abi.decode('number[]', runtime.call('edited')), [1, 41]);
        const before = runtime.instance.exports.memory.buffer;
        const result = abi.decode('number[]', runtime.call('grown'));
        assert.notEqual(runtime.instance.exports.memory.buffer, before);
        assert.equal(result.length, 20000);
        assert.equal(result[0], 2);
        result[0] = 99;
        assert.equal(hostArray[0], 2);
        assert.equal(abi.decode('number[]', runtime.call('grown'))[0], 2);
        assert.equal(runtime.call('lengths', [abi.encode('number[]', new Array(25000).fill(1))]), 25000);
        assert.equal(typeof runtime.call('__alloc', [8]), 'number');
        fails('invocation', /not a function export/, () => runtime.call('memory'));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_raw_arguments_counts_and_bounds_traps() {
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
        const {createAbi} = require('./runtime/abi.cjs');
        const abi = createAbi(() => runtime.instance);
        const handle = abi.encode('number[]', [4, 5]);
        assert.equal(runtime.call('main', [handle, 1]), 5);
        fails('invocation', /expects 2 arguments, got 0/, () => runtime.call('main'));
        fails('invocation', /expects 2 arguments, got 3/, () => runtime.call('main', [handle, 0, 3]));
        fails('invocation', /must be an array/, () => runtime.call('main', '[]'));
        fails('invocation', /limited to 1024/, () => runtime.call('main', new Array(1025).fill(0)));
        for (const value of [[1], new Float64Array([1]), '0', true, null, {}, new Number(1), undefined]) {
            fails('invocation', /numeric scalar/, () => runtime.call('echo_number', [value]));
        }
        fails('invocation', /numeric scalar/, () => runtime.call('echo_number', new Array(1)));
        fails('invocation', /entry name must be a string/, () => runtime.call(1));
        fails('invocation', /not a function export/, () => runtime.call('missing'));
        for (const index of [-1, 2, 0.5, NaN, Infinity]) {
            fails('invocation', /cannot call.*main/, () => runtime.call('main', [handle, index]));
        }
        fails('invocation', /cannot call.*main/, () => runtime.call('main', [0, 0]));
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
        runner(&scratch.0, &["program.wasm", "--args", "[0,0]"]),
        "invocation",
        "main",
    );
    for args in ["[[1],1]", "[1,\"0\"]", "[true]", "[null]", "[{}]"] {
        failure(
            runner(&scratch.0, &["program.wasm", "--args", args]),
            "invocation",
            "raw numeric scalars",
        );
    }
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
fn node_raw_export_selection() {
    let source = r#"fn main(value: number) -> number { return helper(value); } fn helper(value: number) -> number { return value * 2; } fn hidden() -> string { return "hidden"; }"#;
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    script(
        &compile(&compiler, source),
        r#"
        const runtime = instantiate(wasm);
        assert.equal(runtime.call('main', [21]), 42);
        assert.equal(runtime.instance.exports.helper, undefined);
        assert(runtime.instance.exports.memory instanceof WebAssembly.Memory);
        fails('invocation', /not a function export/, () => runtime.call('helper', [1]));
        assert.equal(typeof runtime.call('__alloc', [8]), 'number');
    "#,
    );
    compiler.set_exports(ExportOptions::None);
    script(
        &compile(&compiler, source),
        r#"
        const runtime = instantiate(wasm);
        assert(runtime.instance.exports.memory instanceof WebAssembly.Memory);
        fails('invocation', /not a function export/, () => runtime.call('main', [1]));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_alias_imports_use_raw_callbacks() {
    let mut compiler = Compiler::new();
    for name in ["left", "right"] {
        compiler
            .register_host_function(name, "env", "shared", vec![Type::Number], Type::Number)
            .unwrap();
    }
    script(
        &compile(
            &compiler,
            "fn main(value: number) -> number { return left(value) + right(value); }",
        ),
        r#"
        let calls = 0;
        const runtime = instantiate(wasm, {env: {shared(value) { calls++; return value + 1; }}});
        assert.equal(runtime.call('main', [20]), 42);
        assert.equal(calls, 2);
    "#,
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
        const seen = [];
        instantiate(wasm, {env: {shared(value) { seen.push(value); return value; }}}).call('main');
        assert.deepEqual(seen, [1, 1]);
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
        const provider = Object.create(null);
        provider.__proto__ = Object.create(null);
        provider.__proto__.__proto__ = value => value * 2;
        assert.equal(instantiate(wasm, provider).call('main'), 42);
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
fn node_start_callbacks_and_raw_memory_imports_and_exports() {
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
    script(
        &module.finish(),
        r#"
        let called = false;
        instantiate(wasm, {env: {boot() { called = true; }}});
        assert.equal(called, true);
        fails('instantiation', /boot exploded/, () => instantiate(wasm, {env: {boot() { throw Error('boot exploded'); }}}));
    "#,
    );
    let memory_type = MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: false,
        page_size_log2: None,
    };
    let mut imports = ImportSection::new();
    imports.import("env", "memory", EntityType::Memory(memory_type));
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    let mut module = Module::new();
    module.section(&imports).section(&exports);
    script(
        &module.finish(),
        r#"
        const memory = new WebAssembly.Memory({initial: 1, maximum: 1});
        const runtime = instantiate(wasm, {env: {memory}});
        assert.equal(runtime.instance.exports.memory, memory);
        fails('invocation', /not a function export/, () => runtime.call('memory'));
    "#,
    );
    let mut memories = MemorySection::new();
    memories.memory(memory_type);
    let mut exports = ExportSection::new();
    exports.export("extra", ExportKind::Memory, 0);
    let mut module = Module::new();
    module.section(&memories).section(&exports);
    script(
        &module.finish(),
        r#"
        const runtime = instantiate(wasm);
        assert(runtime.instance.exports.extra instanceof WebAssembly.Memory);
        fails('invocation', /not a function export/, () => runtime.call('extra'));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_raw_i64_arguments_and_results() {
    use wasm_encoder::{
        CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
        TypeSection, ValType,
    };
    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I64]);
    let mut functions = FunctionSection::new();
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 0);
    let mut function = Function::new([]);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::End);
    let mut code = CodeSection::new();
    code.function(&function);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&exports)
        .section(&code);
    script(
        &module.finish(),
        r#"
        const runtime = instantiate(wasm);
        assert.equal(runtime.call('main', [9007199254740993n]), 9007199254740993n);
        fails('invocation', /BigInt/, () => runtime.call('main', [1]));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_example_provider_uses_explicit_descriptor_abi() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("echo", "env", "echo", vec![Type::String], Type::String)
        .unwrap();
    compiler
        .register_host_function("print", "env", "print", vec![Type::String], Type::Void)
        .unwrap();
    compiler
        .register_host_function(
            "adjust",
            "env",
            "adjust",
            vec![Type::NumberArray],
            Type::NumberArray,
        )
        .unwrap();
    let bytes = compile(
        &compiler,
        r#"
        fn main() -> void { print(echo("hello")); }
        fn adjusted() -> number[] { return adjust([1, 2]); }
    "#,
    );
    script(
        &bytes,
        r#"
        const {createAbi} = require('./runtime/abi.cjs');
        const createProvider = require('./examples/host-provider.cjs');
        let runtime;
        const provider = createProvider(() => runtime && runtime.instance);
        runtime = instantiate(wasm, provider);
        const abi = createAbi(() => runtime.instance);
        assert.deepEqual(abi.decode('number[]', runtime.call('adjusted')), [2, 3]);
    "#,
    );
    let scratch = Scratch::new();
    scratch.write("program.wasm", &bytes);
    let provider = root().join("examples/host-provider.cjs");
    assert_eq!(
        success(runner(
            &scratch.0,
            &["program.wasm", "--imports", provider.to_str().unwrap()]
        )),
        "hello from host\n"
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
        runner(&scratch.0, &["--entry", "main"]),
        "invocation",
        "PROGRAM.wasm is required",
    );
    failure(runner(&scratch.0, &["-"]), "file", "cannot read program");
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
        runner(&scratch.0, &["program.wasm", "--entry", "memory"]),
        "invocation",
        "not a function export",
    );
}
