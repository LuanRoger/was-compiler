#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type, codegen};
use wasmparser::{BlockType, Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const VALUES: &str = r#"
    fn main() -> number {
        let sum = 0;
        for value in [2, 3] { sum += value; }
        return sum;
    }
    fn empty() -> number {
        for value in [] { emit(value); return 99; }
        return 7;
    }
    fn captured() -> number {
        let source = [1, 2, 3];
        let alias = source;
        let sum = 0;
        for value in source {
            emit(value);
            sum += value;
            alias[1] = 20;
            source = [99];
        }
        return sum;
    }
    fn effect() -> number {
        let sum = 0;
        for value in supply() { sum += value; emit(value); }
        return sum;
    }
    fn special(a: number[]) -> void { for value in a { emit(value); } }
"#;

const CONTROL: &str = r#"
    fn nested() -> number {
        let sum = 0;
        for r in 0..2 {
            for value in [1, 2, 3] {
                let x: number = if value == 2 { continue; } else { value };
                let n = 0;
                while n < 2 {
                    n += 1;
                    if n == 1 { continue; }
                    sum += (loop { break r + x; });
                }
            }
        }
        return sum;
    }
    fn reverse() -> number {
        let sum = 0;
        for value in [1, 2] {
            for r in 0..3 { if r == 1 { continue; } sum += value + r; }
        }
        return sum;
    }
    fn stop() -> number {
        let sum = 0;
        for value in [1, 2, 3] { if value == 2 { break; } sum += value; }
        return sum;
    }
    fn early() -> number {
        for value in [4, 5] { return value; }
        return 99;
    }
    fn shadow() -> number {
        let sum = 0;
        for value in [1, 2] { { let value = 8; value += 1; sum += value; }; sum += value; }
        return sum;
    }
    fn short_circuit() -> bool {
        return false && { for value in [1] { emit(value); } true };
    }
    fn from_value_loop() -> number {
        return loop { for value in [1, 2] { if value == 1 { continue; } break; } break 8; };
    }
    fn logical_continue() -> number {
        let sum = 0;
        for value in [1, 2, 3] {
            true && { if value == 2 { continue; } true };
            sum += value;
        }
        return sum;
    }
"#;

const TRANSFERS: &str = r#"
    fn skip() -> number {
        let sum = 0;
        for outer in [1, 2, 3] {
            for value in { if outer == 2 { continue; } [outer] } { sum += value; }
        }
        return sum;
    }
    fn stop() -> number {
        let sum = 0;
        for outer in [1, 2, 3] {
            for value in { if outer == 2 { break; } [outer] } { sum += value; }
        }
        return sum;
    }
    fn while_transfer() -> number {
        let n = 0;
        while n < 3 {
            n += 1;
            for value in { if n == 1 { continue; } break; [1] } { emit(value); }
        }
        return n;
    }
    fn returned() -> number { for value in { return 7; [1] } { emit(value); } }
    fn divergent() -> number { for value in loop {} { emit(value); } }
    fn hidden() -> number { for value in { loop {} [1] } { emit(value); } }
"#;

const DESCRIPTORS: &str = r#"
    fn main(a: number[]) -> number {
        let sum = 0;
        for value in a { emit(value); sum += value; mutate(a); }
        return sum;
    }
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    for (name, params, result) in [
        ("emit", vec![Type::Number], Type::Void),
        (
            "supply",
            vec![],
            Type::Array(was_compiler::ArrayElementType::Number),
        ),
        (
            "mutate",
            vec![Type::Array(was_compiler::ArrayElementType::Number)],
            Type::Void,
        ),
    ] {
        compiler
            .register_host_function(name, "env", name, params, result)
            .unwrap();
    }
    compiler
        .register_host_function("unused", "other", "unused", vec![], Type::Bool)
        .unwrap();
    compiler
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let bytes = compiler
        .compile_source(source)
        .expect("compile array for loops");
    Validator::new()
        .validate_all(&bytes)
        .expect("valid array-for Wasm");
    bytes
}

fn run_node_at_all_levels(source: &str, script: &str) {
    let mut compiler = compiler();
    let baseline = compile(&compiler, source);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compile(&compiler, source);
        assert_eq!(bytes, baseline, "{level:?}");
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: Node failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn array_for_compiles_through_public_pipeline() {
    let bytes = Compiler::new()
        .compile_source("fn main() -> number { let sum = 0; for value in [2, 3] { sum += value; } return sum; }")
        .expect("numeric arrays are valid for iterables");
    Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn fixtures_validate_deterministically_with_owned_imports_and_optimization_parity() {
    for source in [VALUES, CONTROL, TRANSFERS, DESCRIPTORS] {
        let mut compiler = compiler();
        let program = compiler.analyze(source).unwrap();
        let snapshot = program.clone();
        let baseline = compile(&compiler, source);
        assert_eq!(codegen::compile(&program).unwrap(), baseline);
        compiler
            .register_host_function("later", "later", "later", vec![], Type::Void)
            .unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            for _ in 0..2 {
                assert_eq!(compiler.compile(&program).unwrap(), baseline, "{level:?}");
                assert_eq!(program, snapshot);
            }
        }
        let imports = Parser::new(0)
            .parse_all(&baseline)
            .find_map(|payload| match payload.unwrap() {
                Payload::ImportSection(section) => Some(
                    section
                        .into_imports()
                        .map(|entry| {
                            let entry = entry.unwrap();
                            (entry.module.to_owned(), entry.name.to_owned())
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            imports,
            [
                ("env".into(), "emit".into()),
                ("env".into(), "supply".into()),
                ("env".into(), "mutate".into()),
                ("other".into(), "unused".into())
            ]
        );
    }
}

fn non_export_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| {
            let (id, range) = payload.unwrap().as_section()?;
            (id != 7).then(|| {
                let start = usize::try_from(range.start).unwrap();
                let end = usize::try_from(range.end).unwrap();
                (id, bytes[start..end].to_vec())
            })
        })
        .collect()
}

#[test]
fn export_selection_does_not_prune_array_bodies_imports_or_shift_helpers() {
    let mut compiler = compiler();
    let program = compiler.analyze(VALUES).unwrap();
    let baseline = compiler.compile(&program).unwrap();
    for selection in [
        ExportOptions::None,
        ExportOptions::Only(vec!["main".into()]),
    ] {
        compiler.set_exports(selection);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(non_export_sections(&bytes), non_export_sections(&baseline));
            let exports = Parser::new(0)
                .parse_all(&bytes)
                .find_map(|payload| match payload.unwrap() {
                    Payload::ExportSection(section) => Some(
                        section
                            .into_iter()
                            .map(|entry| entry.unwrap().name.to_owned())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .unwrap();
            assert!(exports.contains(&"memory".into()));
            assert!(exports.contains(&"__alloc".into()));
            assert!(!exports.contains(&"captured".into()));
        }
    }
}

#[test]
fn decoded_array_for_has_exact_scratch_unsigned_count_checked_load_and_latch() {
    let source = "fn main(a: number[], flag: bool) -> void { for value in a { if flag { continue; } break; } }";
    let mut compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let baseline = codegen::compile(&program).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(bytes, baseline);
        Validator::new().validate_all(&bytes).unwrap();
        let bodies = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|payload| match payload.unwrap() {
                Payload::CodeSectionEntry(body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bodies.len(),
            8,
            "one user and seven memory helpers, no range or iteration helpers"
        );
        let locals = bodies[0]
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .flat_map(|entry| {
                let (n, ty) = entry.unwrap();
                std::iter::repeat_n(ty, usize::try_from(n).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            locals,
            [ValType::F64, ValType::I32, ValType::I32, ValType::I32],
            "binding, captured handle, index, count (parameters excluded)"
        );
        let ops = bodies[0]
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let calls = ops
            .iter()
            .filter_map(|op| match op {
                Operator::Call { function_index } => Some(*function_index),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls,
            [3, 3, 4],
            "ValidateArray at parameter and iterable entry, ArrayAddress at each yield; no ArrayLen revalidation"
        );
        let validation = ops
            .iter()
            .rposition(|op| matches!(op, Operator::Call { function_index: 3 }))
            .unwrap();
        let first_region = ops
            .iter()
            .position(|op| matches!(op, Operator::Block { .. }))
            .unwrap();
        let length = ops
            .iter()
            .position(|op| matches!(op, Operator::I32Load { memarg } if memarg.offset == 4))
            .unwrap();
        assert!(
            validation < length && length < first_region,
            "validate before caching length and entering loop regions"
        );
        assert!(ops.windows(3).any(|ops| matches!(
            ops,
            [
                Operator::LocalGet { local_index: 4 },
                Operator::LocalGet { local_index: 5 },
                Operator::I32GeU
            ]
        )));
        assert!(ops.windows(5).any(|ops| matches!(ops, [Operator::LocalGet { local_index: 3 }, Operator::LocalGet { local_index: 4 }, Operator::F64ConvertI32U, Operator::Call { function_index: 4 }, Operator::F64Load { memarg }] if memarg.offset == 0 && memarg.align == 3)));
        assert!(ops.windows(4).any(|ops| matches!(
            ops,
            [
                Operator::LocalGet { local_index: 4 },
                Operator::I32Const { value: 1 },
                Operator::I32Add,
                Operator::LocalSet { local_index: 4 }
            ]
        )));
        let regions = ops
            .iter()
            .filter_map(|op| match op {
                Operator::Block { blockty } | Operator::Loop { blockty } => Some(*blockty),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(regions, [BlockType::Empty; 3]);
        let branches = ops
            .iter()
            .filter_map(|op| match op {
                Operator::Br { relative_depth } => Some(*relative_depth),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            branches,
            [1, 2, 0],
            "continue exits latch, break exits for, backedge retests"
        );
        assert!(!ops.iter().any(|op| matches!(
            op,
            Operator::I64Add | Operator::F64Add | Operator::F64ConvertI32S
        )));
    }
}

#[test]
fn array_literal_temporary_follows_reserved_for_scratch() {
    let mut compiler = Compiler::new();
    let program = compiler
        .analyze("fn main() -> void { for value in [1] { value; } }")
        .unwrap();
    let snapshot = program.clone();
    let baseline = codegen::compile(&program).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(bytes, baseline);
        Validator::new().validate_all(&bytes).unwrap();
        let body = Parser::new(0)
            .parse_all(&bytes)
            .find_map(|payload| match payload.unwrap() {
                Payload::CodeSectionEntry(body) => Some(body),
                _ => None,
            })
            .unwrap();
        let locals = body
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .flat_map(|entry| {
                let (n, ty) = entry.unwrap();
                std::iter::repeat_n(ty, usize::try_from(n).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            locals,
            [
                ValType::F64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32
            ],
            "binding, for handle/index/count, then array-literal handle"
        );
        let ops = body
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let array_handle = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    Operator::LocalSet { local_index: 4 } | Operator::LocalTee { local_index: 4 }
                )
            })
            .unwrap();
        let captured_handle = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    Operator::LocalSet { local_index: 1 } | Operator::LocalTee { local_index: 1 }
                )
            })
            .unwrap();
        assert!(array_handle < captured_handle);
        assert_eq!(program, snapshot);
    }
}

#[test]
fn scratch_is_reserved_before_iterable_children_without_nested_or_sibling_reuse() {
    let source = "fn main(a: number[]) -> void { for i in { for j in a {} a } {} false && { for k in a {} true }; return; for n in a {} }";
    let compiler = Compiler::new();
    let bytes = compile(&compiler, source);
    let body = Parser::new(0)
        .parse_all(&bytes)
        .find_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .unwrap();
    let locals = body
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .flat_map(|entry| {
            let (n, ty) = entry.unwrap();
            std::iter::repeat_n(ty, usize::try_from(n).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        locals,
        [
            ValType::F64,
            ValType::F64,
            ValType::F64,
            ValType::F64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32
        ]
    );
    let ops = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let updates = ops
        .windows(2)
        .filter_map(|ops| match ops {
            [Operator::I32Add, Operator::LocalSet { local_index }] => Some(*local_index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        updates,
        [9, 6, 12, 15],
        "outer scratch precedes iterable child scratch; emit child first; skipped and dead loops retain distinct slots"
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_empty_capture_alias_mutation_once_only_effect_and_special_elements() {
    run_node_at_all_levels(
        VALUES,
        r#"
        const assert = require('node:assert/strict');
        let e, emitted = [], supplied = 0;
        function array(values) {
            const h = e.__alloc(8), p = e.__alloc(Math.max(8, values.length * 8));
            const v = new DataView(e.memory.buffer);
            v.setUint32(h,p,true); v.setUint32(h+4,values.length,true);
            values.forEach((x,i) => v.setFloat64(p+i*8,x,true)); return h;
        }
        e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            env: { emit(x) { assert.ok(emitted.length < 32, 'guest iteration cap'); emitted.push(x); },
                supply() { ++supplied; return array([5,6]); }, mutate() {} },
            other: { unused() { return 0; } }
        }).exports;
        assert.equal(e.main(),5);
        assert.equal(e.empty(),7); assert.deepEqual(emitted,[]);
        assert.equal(e.captured(),24); assert.deepEqual(emitted,[1,20,3]);
        emitted = []; assert.equal(e.effect(),11); assert.equal(supplied,1); assert.deepEqual(emitted,[5,6]);
        emitted = []; e.special(array([NaN,Infinity,-Infinity,-0,Number.MIN_VALUE,Number.MAX_VALUE]));
        assert.equal(emitted.length,6); assert.ok(Number.isNaN(emitted[0]));
        assert.equal(emitted[1],Infinity); assert.equal(emitted[2],-Infinity);
        assert.ok(Object.is(emitted[3],-0)); assert.equal(emitted[4],Number.MIN_VALUE); assert.equal(emitted[5],Number.MAX_VALUE);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_nested_loops_expression_continue_break_return_and_shadowing() {
    run_node_at_all_levels(
        CONTROL,
        r#"
        const assert = require('node:assert/strict'); let emitted = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            env: { emit(x) { assert.ok(emitted.length < 32); emitted.push(x); }, supply() { throw Error('unexpected'); }, mutate() {} },
            other: { unused() { return 0; } }
        }).exports;
        assert.equal(e.nested(),10); assert.equal(e.reverse(),10); assert.equal(e.stop(),1);
        assert.equal(e.early(),4); assert.equal(e.shadow(),21); assert.equal(e.from_value_loop(),8);
        assert.equal(e.logical_continue(),4);
                assert.equal(e.short_circuit(),0); assert.deepEqual(emitted,[]);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_iterable_transfers_target_outer_loop_and_return_without_yielding() {
    run_node_at_all_levels(
        TRANSFERS,
        r#"
        const assert = require('node:assert/strict');
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            env: { emit() { throw Error('body must not execute'); }, supply() { throw Error('unexpected'); }, mutate() {} },
            other: { unused() { return 0; } }
        }).exports;
        assert.equal(e.skip(),4); assert.equal(e.stop(),1); assert.equal(e.while_transfer(),2); assert.equal(e.returned(),7);
        // Divergent exports are statically validated, deliberately never executed.
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_descriptor_entry_traps_even_empty_and_each_element_rechecks_descriptor() {
    run_node_at_all_levels(
        DESCRIPTORS,
        r#"
        const assert = require('node:assert/strict');
        const wasm = new WebAssembly.Module(require('node:fs').readFileSync(0));
        let e, emitted, mode;
        function fresh() {
            emitted = []; mode = 'none';
            e = new WebAssembly.Instance(wasm, {
                env: { emit(x) { assert.ok(emitted.length < 8,'guest iteration cap'); emitted.push(x); },
                    supply() { throw Error('unexpected'); }, mutate(h) {
                        const v = new DataView(e.memory.buffer);
                        if (mode === 'length') v.setUint32(h+4,0,true);
                        if (mode === 'pointer') v.setUint32(h,e.memory.buffer.byteLength+8,true);
                                                if (mode === 'grow_length') v.setUint32(h+4,3,true);
                        if (mode === 'relocate') { v.setUint32(h,alternate,true); mode = 'none'; }
                    } }, other: { unused() { return 0; } }
            }).exports;
        }
        function array(values) {
            const h = e.__alloc(8), p = e.__alloc(Math.max(8,values.length*8));
            const v = new DataView(e.memory.buffer);
            v.setUint32(h,p,true); v.setUint32(h+4,values.length,true);
            values.forEach((x,i) => v.setFloat64(p+i*8,x,true)); return h;
        }
        for (const handle of [0,1,7,9,4294967288]) {
            fresh(); assert.throws(() => e.main(handle),WebAssembly.RuntimeError); assert.deepEqual(emitted,[]);
        }
        for (const corruption of ['pointer','alignment','length']) {
            fresh(); const h = array([]), v = new DataView(e.memory.buffer);
            if (corruption === 'pointer') v.setUint32(h,e.memory.buffer.byteLength+8,true);
            if (corruption === 'alignment') v.setUint32(h,9,true);
            if (corruption === 'length') v.setUint32(h+4,4294967295,true);
            assert.throws(() => e.main(h),WebAssembly.RuntimeError); assert.deepEqual(emitted,[]);
        }
        fresh(); assert.equal(e.main(array([])),0); assert.deepEqual(emitted,[]);
        for (const corruption of ['pointer','length']) {
            fresh(); const h = array([1,2]); mode = corruption;
            assert.throws(() => e.main(h),WebAssembly.RuntimeError); assert.deepEqual(emitted,[1]);
        }
        fresh(); const growing = array([1,2,3]);
        new DataView(e.memory.buffer).setUint32(growing+4,2,true);
        mode = 'grow_length'; assert.equal(e.main(growing),3); assert.deepEqual(emitted,[1,2], 'iteration count is cached');
        fresh(); const h = array([1,2,3]); const alternate = e.__alloc(24);
        const v = new DataView(e.memory.buffer); [10,20,30].forEach((x,i) => v.setFloat64(alternate+i*8,x,true));
        mode = 'relocate'; assert.equal(e.main(h),51); assert.deepEqual(emitted,[1,20,30]);
    "#,
    );
}
