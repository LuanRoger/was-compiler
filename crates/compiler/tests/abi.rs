use std::{process::Command, time::Duration};

use was_compiler::{Compiler, CompilerOptions, OptimizationLevel, Type};
use wasmparser::{DataKind, Operator, Parser, Payload, Validator};

#[path = "../../../tests/support/process.rs"]
mod test_process;

const SOURCE: &str = include_str!("../../../tests/fixtures/abi/memory.was");
const GOLDEN: &[u8] = include_bytes!("../../../tests/fixtures/abi/memory.wasm");
const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn compiler(level: OptimizationLevel) -> Compiler {
    let mut compiler = Compiler::with_options(CompilerOptions {
        optimization_level: level,
    });

    for (name, ty) in [
        (
            "echo_array",
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
        ("echo_string", Type::String),
        ("echo_bool", Type::Bool),
    ] {
        compiler
            .register_host_function(name, "env", name, vec![ty], ty)
            .unwrap();
    }

    compiler
}

#[test]
fn memory_abi_preserves_pre_refactor_module_bytes_at_every_level() {
    Validator::new().validate_all(GOLDEN).unwrap();

    for level in LEVELS {
        let compiler = compiler(level);
        let program = compiler.analyze(SOURCE).unwrap();
        let bytes = compiler.compile(&program).unwrap();

        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(bytes, GOLDEN, "ABI bytes changed at {level:?}");
        assert_eq!(compiler.compile_source(SOURCE).unwrap(), GOLDEN);
    }
}

#[test]
fn static_descriptors_match_independent_little_endian_bytes() {
    let bytes = compiler(OptimizationLevel::None)
        .compile_source(SOURCE)
        .unwrap();
    let mut expected = vec![0; 8];
    expected.extend_from_slice(&[16, 0, 0, 0, 11, 0, 0, 0]);
    expected.extend_from_slice(&[104, 195, 169, 108, 108, 111, 32, 240, 159, 166, 128]);
    expected.resize(32, 0);
    expected.extend_from_slice(&[40, 0, 0, 0, 0, 0, 0, 0]);
    let mut data_count = 0;
    let mut global_count = 0;

    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::DataSection(reader) => {
                for data in reader {
                    let data = data.unwrap();
                    let DataKind::Active {
                        memory_index,
                        offset_expr,
                    } = data.kind
                    else {
                        panic!("expected active ABI data");
                    };
                    let mut offset = offset_expr.get_operators_reader();

                    assert_eq!(memory_index, 0);
                    assert!(matches!(
                        offset.read().unwrap(),
                        Operator::I32Const { value: 0 }
                    ));
                    assert!(matches!(offset.read().unwrap(), Operator::End));
                    assert_eq!(data.data, expected);
                    data_count += 1;
                }
            }

            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global.unwrap();
                    let mut initial = global.init_expr.get_operators_reader();

                    assert!(global.ty.mutable);
                    assert!(matches!(
                        initial.read().unwrap(),
                        Operator::I32Const { value: 40 }
                    ));
                    assert!(matches!(initial.read().unwrap(), Operator::End));
                    global_count += 1;
                }
            }

            _ => {}
        }
    }

    assert_eq!(data_count, 1);
    assert_eq!(global_count, 1);
}

#[test]
#[ignore = "requires Node.js"]
fn node_guest_and_explicit_host_marshaling_agree_on_descriptor_bytes() {
    let bytes = compiler(OptimizationLevel::None)
        .compile_source(SOURCE)
        .unwrap();
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        const {createAbi} = require('./runtime/abi.cjs');
        const calls = [];
        const instance = new WebAssembly.Instance(
            new WebAssembly.Module(fs.readFileSync(0)),
            {env: {
                echo_array(handle) { calls.push(['array', handle]); return handle; },
                echo_string(handle) { calls.push(['string', handle]); return handle; },
                echo_bool(value) { calls.push(['bool', value]); return 42; },
            }}
        );
        const e = instance.exports;
        const abi = createAbi(() => instance);
        const text = e.text();
        const empty = e.empty();
        let view = new DataView(e.memory.buffer);
        assert.equal(text, 8);
        assert.equal(empty, 32);
        assert.deepEqual(Array.from(new Uint8Array(e.memory.buffer, text, 8)),
            [16, 0, 0, 0, 11, 0, 0, 0]);
        assert.deepEqual(Array.from(new Uint8Array(e.memory.buffer, empty, 8)),
            [40, 0, 0, 0, 0, 0, 0, 0]);
        assert.equal(abi.decode('string', text), 'héllo 🦀');
        assert.equal(abi.decode('string', empty), '');
        const hostText = abi.encode('string', 'héllo 🦀');
        assert.equal(e.same_text(hostText), 1);
        assert.equal(e.host_text(hostText), hostText);
        assert.equal(e.host_bool(7), 1);
        const values = e.values();
        assert.equal(values, 64);
        assert.equal(view.getUint32(values, true), 72);
        assert.equal(view.getUint32(values + 4, true), 3);
        assert.deepEqual(abi.decode('number[]', values), [1, 0, 2.5]);
        const hostValues = abi.encode('number[]', [1, 0, 2.5]);
        assert.equal(e.same_values(values, hostValues), 1);
        assert.equal(e.count(hostValues), 3);
        assert.equal(e.through_host(hostValues), hostValues);
        assert.deepEqual(calls, [['string', hostText], ['bool', 1], ['array', hostValues]]);
        const copied = abi.decode('number[]', values);
        copied[0] = 9;
        assert.equal(view.getFloat64(72, true), 1);
        view.setFloat64(72, 4, true);
        assert.deepEqual(abi.decode('number[]', values), [4, 0, 2.5]);
        assert.deepEqual(copied, [9, 0, 2.5]);
        const oldBuffer = e.memory.buffer;
        e.__alloc(65536);
        assert.notEqual(e.memory.buffer, oldBuffer);
        assert.equal(oldBuffer.byteLength, 0);
        assert.deepEqual(abi.decode('number[]', values), [4, 0, 2.5]);
        assert.equal(abi.decode('string', hostText), 'héllo 🦀');
    "#;
    let output = test_process::output(
        Command::new("node")
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .args(["-e", script]),
        Some(&bytes),
        Duration::from_secs(15),
    );

    assert!(
        output.status.success(),
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
