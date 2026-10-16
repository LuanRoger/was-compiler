#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{path::PathBuf, process::Command, time::Duration};
use was_compiler::{
    ArrayElementType, Compiler, ExportOptions, OptimizationLevel, Type, diagnostic,
};
use wasmparser::Validator;

const SOURCE: &str = r#"
fn flags(value: bool[]) -> bool[] { return host_flags(value); }
fn texts(value: string[]) -> string[] { return host_texts(value); }
fn hidden() -> range { return 0..2; }
fn choose(words: string[], flags: bool[]) -> number { return len(words) + len(flags); }
fn empty() -> string[] { return match true { true => [], false => loop { break []; } }; }
fn main() -> number {
    let flags = [true, false];
    let words = ["é", "z"];
    let alias = words;
    let update = () => { alias[1] = "λ"; return flags[0]; };
    let score = 0;
    for flag in flags { let read = () => flag; if read() { score += 1; } }
    for word in words { let read = () => word; if read() == "é" { score += 2; } }
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
    compiler
}

fn field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
    input.extend_from_slice(bytes);
}

#[test]
#[ignore = "requires Node.js and pkg freshly rebuilt from current source"]
fn rebuilt_browser_typed_array_hosts_bytes_diagnostics_and_execution_match_core() {
    let package = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../pkg"));
    assert!(package.join("was_compiler_bg.wasm").is_file());
    let mut input = Vec::new();
    field(&mut input, SOURCE.as_bytes());
    let mut native = compiler();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        native.set_optimization_level(level);
        for names in [vec!["main", "flags", "texts"], vec![]] {
            native.set_exports(ExportOptions::Only(
                names.into_iter().map(String::from).collect(),
            ));
            let bytes = native.compile_source(SOURCE).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            field(&mut input, &bytes);
            field(
                &mut input,
                wasmprinter::print_bytes(&bytes).unwrap().as_bytes(),
            );
        }
    }
    let cases = [
        "fn main() { let a = [true, 1]; }",
        "fn take(a: bool[]) {} fn main() { [] |> take; }",
        "fn make() { return []; } fn main() { let a: string[] = make(); }",
        "fn main() { for value in [true] { value = false; } }",
        "fn main() { let a = [\"é\"]; a[0] += \"x\"; }",
        "fn main() { let a: bool[][] = []; }",
    ];
    for source in cases {
        let error = Compiler::new().compile_source(source).unwrap_err();
        field(&mut input, source.as_bytes());
        field(
            &mut input,
            diagnostic::render("<browser>", source, &error).as_bytes(),
        );
    }
    let script = r#"
        const assert = require('node:assert/strict'), fs = require('node:fs'), path = require('node:path');
        const {pathToFileURL} = require('node:url');
        async function main() {
            const input = fs.readFileSync(0); let offset = 0;
            function field() { const n = input.readUInt32LE(offset); offset += 4;
                const b = input.subarray(offset, offset+n); offset += n; return b; }
            const source = field().toString();
            const native = Array.from({length:4}, () => Array.from({length:2}, () => ({bytes:field(), wat:field().toString()})));
            const diagnostics = Array.from({length:6}, () => ({source:field().toString(), message:field().toString()}));
            assert.equal(offset, input.length);
            const dir = process.argv[1];
            const {default:init, Compiler} = await import(pathToFileURL(path.join(dir,'was_compiler.js')).href);
            await init({module_or_path:fs.readFileSync(path.join(dir,'was_compiler_bg.wasm'))});
            const c = new Compiler();
            try {
                c.registerHostFunction('host_flags','env','host_flags',['bool[]'],'bool[]');
                c.registerHostFunction('host_texts','env','host_texts',['string[]'],'string[]');
                for (let level=0; level<4; level++) {
                    c.setOptimizationLevel(level);
                    for (const [index,names] of [['main','flags','texts'],[]].entries()) {
                        c.setExports(names);
                        const bytes = c.compile(source);
                        assert.deepEqual(Buffer.from(bytes), native[level][index].bytes);
                        assert.equal(c.compileWat(source), native[level][index].wat);
                        assert.ok(WebAssembly.validate(bytes));
                        const m = new WebAssembly.Module(bytes);
                        assert.deepEqual(WebAssembly.Module.imports(m), [
                            {module:'env',name:'host_flags',kind:'function'},
                            {module:'env',name:'host_texts',kind:'function'}]);
                        const calls = [];
                        const e = new WebAssembly.Instance(m, {env:{
                            host_flags(h) {calls.push('bool'); return h;},
                            host_texts(h) {calls.push('string'); return h;}
                        }}).exports;
                        assert.ok(!('hidden' in e));
                        if (index === 0) {
                            assert.equal(e.main(),10);
                            const b=e.__alloc(16), s=e.__alloc(16);
                            const v=new DataView(e.memory.buffer);
                            v.setUint32(b,b+8,true); v.setUint32(b+4,2,true);
                            v.setUint8(b+8,1); v.setUint8(b+9,0);
                            v.setUint32(s,s+8,true); v.setUint32(s+4,0,true);
                            assert.equal(e.flags(b),b); assert.equal(e.texts(s),s);
                            assert.deepEqual(calls,['bool','string']);
                        } else { assert.ok(!('main' in e)); assert.ok(e.memory); }
                    }
                    for (const d of diagnostics) for (const method of ['compile','compileWat']) {
                        assert.throws(() => c[method](d.source), e => e instanceof Error && e.message === d.message);
                    }
                }
                for (const params of [['range'],[]]) {
                    assert.throws(() => c.registerHostFunction('bad','env','bad',params,params.length ? 'void':'range'), /range values are private/);
                }
                // Failed registrations must not reserve the source name.
                c.registerHostFunction('bad','env','bad',['bool[]'],'string[]');
            } finally { c.free(); }
        }
        main().catch(e => { console.error(e); process.exitCode=1; });
    "#;
    let output = test_process::output(
        Command::new("node").args(["-e", script]).arg(package),
        Some(&input),
        Duration::from_secs(60),
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
