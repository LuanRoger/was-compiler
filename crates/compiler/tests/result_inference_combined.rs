#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    semantic::SemanticError,
    typed_ast::{CallBindingOrigin, FunctionId, FunctionRef, TypedExprKind, TypedStatementKind},
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const DIAMOND: &str = r#"
    fn main() /*number*/ { return left(c: sample(9), a: sample(2)) + right(sample(3)); }
    fn left(a: number, b: number = a + 1, c: number = b * 2) /*number*/ { return leaf(c: c, a: a); }
    fn right(a: number, b: number = a + 1, c: number = b * 2) /*number*/ { return leaf(a, b, c); }
    fn leaf(a: number, b: number = a + 1, c: number = b * 2) /*number*/ { return a * 100 + b * 10 + c; }
"#;

fn explicit(source: &str) -> String {
    let mut source = source.to_owned();
    for ty in ["number", "bool", "string", "number[]", "range"] {
        source = source.replace(&format!("/*{ty}*/"), &format!("-> {ty}"));
    }
    source
}

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("sample", "env", "sample", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler
}

#[derive(Default)]
struct Decoded<'a> {
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    bodies: Vec<Vec<Operator<'a>>>,
}

fn decode(bytes: &[u8]) -> Decoded<'_> {
    Validator::new().validate_all(bytes).unwrap();
    let mut decoded = Decoded::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                for signature in section.into_iter_err_on_gc_types() {
                    let signature = signature.unwrap();
                    decoded
                        .signatures
                        .push((signature.params().to_vec(), signature.results().to_vec()));
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import.unwrap();
                    let TypeRef::Func(index) = import.ty else {
                        panic!("function import")
                    };
                    decoded
                        .imports
                        .push((import.module.into(), import.name.into(), index));
                }
            }
            Payload::FunctionSection(section) => decoded
                .functions
                .extend(section.into_iter().map(Result::unwrap)),
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    decoded
                        .exports
                        .push((export.name.into(), export.kind, export.index));
                }
            }
            Payload::CodeSectionEntry(body) => decoded.bodies.push(
                body.get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .map(Result::unwrap)
                    .collect(),
            ),
            _ => {}
        }
    }
    decoded
}

#[test]
fn mixed_call_diamond_has_canonical_concrete_calls_and_owned_source_order_layout() {
    let mut compiler = compiler();
    let program = compiler.analyze(&String::from(DIAMOND)).unwrap();
    let snapshot = program.clone();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f.id, f.return_type, f.params.len()))
            .collect::<Vec<_>>(),
        [
            ("main", FunctionId(0), Type::Number, 0),
            ("left", FunctionId(1), Type::Number, 3),
            ("right", FunctionId(2), Type::Number, 3),
            ("leaf", FunctionId(3), Type::Number, 3),
        ]
    );
    let TypedStatementKind::Return(Some(value)) = &program.functions[1].body[0].kind else {
        panic!("return")
    };
    let TypedExprKind::NormalizedCall(sequence) = &value.kind else {
        panic!("normalized named/default call")
    };
    assert_eq!(
        sequence
            .bindings
            .iter()
            .map(|b| b.origin)
            .collect::<Vec<_>>(),
        [
            CallBindingOrigin::Explicit { argument: 0 },
            CallBindingOrigin::Explicit { argument: 1 },
            CallBindingOrigin::Default { formal: 1 },
        ]
    );
    assert_eq!(
        sequence.formal_to_binding,
        [
            sequence.bindings[1].id,
            sequence.bindings[2].id,
            sequence.bindings[0].id
        ]
    );
    let TypedExprKind::Call { func, args, .. } = &sequence.call.kind else {
        panic!("canonical call")
    };
    assert_eq!(*func, FunctionRef::User(FunctionId(3)));
    assert_eq!(sequence.call.ty, Type::Number);
    for (arg, id) in args.iter().zip(&sequence.formal_to_binding) {
        assert!(matches!(arg.kind, TypedExprKind::Identifier { id: actual, .. } if actual == *id));
        assert_eq!(arg.ty, Type::Number);
    }
    assert_eq!(args.len(), 3);
    let TypedStatementKind::Return(Some(value)) = &program.functions[2].body[0].kind else {
        panic!("return")
    };
    assert!(
        matches!(&value.kind, TypedExprKind::Call { func: FunctionRef::User(FunctionId(3)), args, .. } if args.len() == 3)
    );

    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for exports in [
        ExportOptions::Only(vec!["main".into(), "leaf".into()]),
        ExportOptions::None,
    ] {
        compiler.set_exports(exports.clone());
        let mut baseline = None;
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            // A fresh compiler retains precisely the analysis-time import snapshot.
            let mut fresh = self::compiler();
            fresh.set_exports(exports.clone());
            fresh.set_optimization_level(level);
            assert_eq!(bytes, fresh.compile_source(DIAMOND).unwrap());
            assert_eq!(bytes, fresh.compile_source(&explicit(DIAMOND)).unwrap());
            assert_eq!(bytes, compiler.compile(&program).unwrap());
            if let Some(previous) = &baseline {
                assert_eq!(&bytes, previous);
            }
            let decoded = decode(&bytes);
            assert_eq!(
                decoded.imports,
                [
                    ("env".into(), "sample".into(), 0),
                    ("env".into(), "unused".into(), 1)
                ]
            );
            assert_eq!(
                decoded.signatures,
                [
                    (vec![ValType::F64], vec![ValType::F64]),
                    (vec![], vec![]),
                    (vec![], vec![ValType::F64]),
                    (vec![ValType::F64; 3], vec![ValType::F64]),
                ]
            );
            assert_eq!(decoded.functions, [2, 3, 3, 3]);
            assert_eq!(
                decoded
                    .bodies
                    .iter()
                    .map(|ops| ops
                        .iter()
                        .filter_map(|op| match op {
                            Operator::Call { function_index } => Some(*function_index),
                            _ => None,
                        })
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                [vec![0, 0, 3, 0, 4], vec![5], vec![5], vec![]]
            );
            assert!(
                decoded
                    .bodies
                    .iter()
                    .flatten()
                    .all(|op| !matches!(op, Operator::CallIndirect { .. }))
            );
            assert!(matches!(decoded.bodies[1].as_slice(), [
                Operator::LocalGet { local_index: 2 }, Operator::LocalSet { local_index: 3 },
                Operator::LocalGet { local_index: 0 }, Operator::LocalSet { local_index: 4 },
                Operator::LocalGet { local_index: 4 }, Operator::F64Const { value: one }, Operator::F64Add, Operator::LocalSet { local_index: 5 },
                Operator::LocalGet { local_index: 4 }, Operator::LocalGet { local_index: 5 }, Operator::LocalGet { local_index: 3 },
                Operator::Call { function_index: 5 }, Operator::Return, Operator::End,
            ] if one.bits() == 1f64.to_bits()));
            assert!(matches!(
                decoded.bodies[2].as_slice(),
                [
                    Operator::LocalGet { local_index: 0 },
                    Operator::LocalGet { local_index: 1 },
                    Operator::LocalGet { local_index: 2 },
                    Operator::Call { function_index: 5 },
                    Operator::Return,
                    Operator::End,
                ]
            ));
            assert_eq!(
                decoded.exports,
                if exports == ExportOptions::None {
                    vec![]
                } else {
                    vec![
                        ("main".into(), ExternalKind::Func, 2),
                        ("leaf".into(), ExternalKind::Func, 5),
                    ]
                }
            );
            baseline = Some(bytes);
        }
    }
    assert_eq!(program, snapshot);
}

const DESCRIPTORS: &str = r#"
    fn main() /*number*/ {
        let xs = [1, 2]; let alias = xs; let scalar = 3;
        let l = left(xs: xs, seed: sample(scalar), r: 0..3,
            trigger: { scalar = 9; alias[0] = 8; xs = [99]; sample(102) });
        let r = right(alias, sample(scalar), 0..3, sample(104));
        return l + r + alias[0] + xs[0] + scalar;
    }
    fn left(xs: number[], seed: number, r: range, trigger: number, copy: number[] = xs) /*number*/ {
        return leaf(r: r, xs: copy, seed: seed, trigger: trigger);
    }
    fn right(xs: number[], seed: number, r: range, trigger: number) /*number*/ {
        return leaf(xs, seed, r, trigger, xs, len(xs), true, "é", "é", r);
    }
    fn leaf(xs: number[], seed: number, r: range, trigger: number,
        copy: number[] = xs, n: number = len(copy), flag: bool = xs == copy && n == 2,
        text: string = "é", mirror: string = text, range_copy: range = r) /*number*/ {
        let shared = arrays(copy);
        shared[0] += seed;
        return xs[0] * 100 + n * 10 + if flags(flag) && texts(text) == mirror && 2 in ranges(range_copy) { 1 } else { 0 };
    }
    fn flags(x: bool, copy: bool = x) /*bool*/ { return flag_leaf(copy: copy, x: x); }
    fn texts(x: string, copy: string = x) /*string*/ { return text_leaf(x, copy); }
    fn arrays(x: number[], copy: number[] = x) /*number[]*/ { return array_leaf(x: copy); }
    fn ranges(x: range, copy: range = x) /*range*/ { return range_leaf(copy: copy, x: x); }
    fn flag_leaf(x: bool, copy: bool = x) /*bool*/ { return copy; }
    fn text_leaf(x: string, copy: string = x) /*string*/ { return copy; }
    fn array_leaf(x: number[], copy: number[] = x) /*number[]*/ { return copy; }
    fn range_leaf(x: range, copy: range = x) /*range*/ { return copy; }
"#;

#[test]
fn descriptor_default_diamond_keeps_full_arity_private_ranges_and_required_memory() {
    let mut compiler = compiler();
    let program = compiler.analyze(DESCRIPTORS).unwrap();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|f| f.return_type)
            .collect::<Vec<_>>(),
        [
            Type::Number,
            Type::Number,
            Type::Number,
            Type::Number,
            Type::Bool,
            Type::String,
            Type::NumberArray,
            Type::Range,
            Type::Bool,
            Type::String,
            Type::NumberArray,
            Type::Range,
        ]
    );
    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::RangeExport {
            function: "left".into()
        })
    );
    let signatures = [
        (vec![], ValType::F64),
        (
            vec![
                ValType::I32,
                ValType::F64,
                ValType::I32,
                ValType::F64,
                ValType::I32,
            ],
            ValType::F64,
        ),
        (
            vec![ValType::I32, ValType::F64, ValType::I32, ValType::F64],
            ValType::F64,
        ),
        (
            vec![
                ValType::I32,
                ValType::F64,
                ValType::I32,
                ValType::F64,
                ValType::I32,
                ValType::F64,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            ValType::F64,
        ),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
        (vec![ValType::I32; 2], ValType::I32),
    ];
    for exports in [
        ExportOptions::Only(vec!["main".into(), "arrays".into(), "texts".into()]),
        ExportOptions::None,
    ] {
        compiler.set_exports(exports.clone());
        let baseline = compiler.compile(&program).unwrap();
        let decoded = decode(&baseline);
        for (index, (params, result)) in signatures.iter().enumerate() {
            assert_eq!(
                &decoded.signatures[usize::try_from(decoded.functions[index]).unwrap()],
                &(params.clone(), vec![*result]),
                "function {index}"
            );
        }
        assert!(
            decoded
                .exports
                .contains(&("memory".into(), ExternalKind::Memory, 0))
        );
        let function_exports = decoded
            .exports
            .iter()
            .filter(|(_, kind, _)| *kind == ExternalKind::Func)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            function_exports,
            if exports == ExportOptions::None {
                vec![("__alloc".into(), ExternalKind::Func, 14)]
            } else {
                vec![
                    ("main".into(), ExternalKind::Func, 2),
                    ("texts".into(), ExternalKind::Func, 7),
                    ("arrays".into(), ExternalKind::Func, 8),
                    ("__alloc".into(), ExternalKind::Func, 14),
                ]
            }
        );
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap(), baseline);
            assert_eq!(compiler.compile_source(DESCRIPTORS).unwrap(), baseline);
            assert_eq!(
                compiler.compile_source(&explicit(DESCRIPTORS)).unwrap(),
                baseline
            );
        }
    }
}

fn run_node(bytes: &[u8], assertions: &str) {
    Validator::new().validate_all(bytes).unwrap();
    let script = format!(
        r#"
        const assert = require('node:assert/strict');
        const events = [];
        let e;
        e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {{env: {{
            sample(value) {{ events.push(value); return value; }},
            unused() {{ throw new Error('unused import executed'); }},
            corrupt(handle) {{
                events.push(99);
                new DataView(e.memory.buffer).setUint32(handle + 4, 0xffffffff, true);
                return 0;
            }}
        }} }}).exports;
        {assertions}
    "#
    );
    let output = test_process::output(
        Command::new("node").arg("-e").arg(script),
        Some(bytes),
        Duration::from_secs(30),
    );
    assert!(
        output.status.success(),
        "Node failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn mixed_diamonds_execute_once_in_written_order_with_capture_aliases_and_fresh_invocations() {
    for (source, exports, assertions) in [
        (
            DIAMOND,
            vec!["main".into(), "leaf".into()],
            r#"
            assert.equal(e.main(), 587);
            assert.deepEqual(events.splice(0), [9, 2, 3]);
            assert.equal(e.main(), 587);
            assert.deepEqual(events, [9, 2, 3]);
            assert.equal(e.leaf.length, 3);
            assert.equal(e.leaf(2, 4, 6), 246);
            assert.ok(Number.isNaN(e.leaf(2)));
        "#,
        ),
        (
            DESCRIPTORS,
            vec!["main".into(), "arrays".into(), "texts".into()],
            r#"
            assert.equal(e.main(), 3270);
            assert.deepEqual(events.splice(0), [3, 102, 9, 104]);
            assert.equal(e.main(), 3270);
            assert.deepEqual(events, [3, 102, 9, 104]);
            assert.equal(e.left, undefined);
            assert.equal(e.ranges, undefined);
            assert.equal(e.arrays.length, 2);
            const h = e.__alloc(24);
            const view = new DataView(e.memory.buffer);
            view.setUint32(h, h + 8, true);
            view.setUint32(h + 4, 2, true);
            view.setFloat64(h + 8, 41, true);
            view.setFloat64(h + 16, 42, true);
            assert.equal(e.arrays(h, h), h);
        "#,
        ),
    ] {
        let mut compiler = compiler();
        compiler.set_exports(ExportOptions::Only(exports));
        for level in LEVELS {
            compiler.set_optimization_level(level);
            run_node(&compiler.compile_source(source).unwrap(), assertions);
        }
    }
}

const TRANSFERS: &str = r#"
    fn main() /*number*/ { return returned() + broken() + continued() + skipped(); }
    fn returned() /*number*/ {
        return 5 + if left(flag: loop { break { sample(1); if true { return 77; } else { false } }; }, trigger: sample(99)) { 1 } else { 0 };
    }
    fn broken() /*number*/ {
        return 10 + loop {
            left(flag: { let inner = loop { break true; }; sample(2); if inner { break 8; } else { false } }, trigger: sample(99));
            break 99;
        };
    }
    fn continued() /*number*/ {
        let n = 0;
        for i in 1..=2 {
            for j in [1, 2] {
                right(flag: { if j == 1 { sample(20 + i); continue; } else { false } }, trigger: sample(30 + i));
                n += 1;
            }
        }
        return n;
    }
    fn skipped() /*number*/ {
        false && left(flag: { sample(90); true }, trigger: sample(91));
        return if right(flag: false, trigger: sample(4)) { 0 } else { 5 };
        left(flag: { sample(92); true }, trigger: sample(93));
    }
    fn left(flag: bool, trigger: number, copy: bool = flag) /*bool*/ { return leaf(trigger: trigger, flag: copy); }
    fn right(flag: bool, trigger: number, copy: bool = flag) /*bool*/ { return leaf(flag, trigger, copy); }
    fn leaf(flag: bool, trigger: number, copy: bool = flag) /*bool*/ { sample(98); return copy; }
"#;

#[test]
fn nested_transfer_diamond_resolves_caller_results_and_validates_at_every_level() {
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let program = compiler.analyze(TRANSFERS).unwrap();
    assert_eq!(
        program
            .functions
            .iter()
            .map(|f| f.return_type)
            .collect::<Vec<_>>(),
        [
            Type::Number,
            Type::Number,
            Type::Number,
            Type::Number,
            Type::Number,
            Type::Bool,
            Type::Bool,
            Type::Bool,
        ]
    );
    let baseline = compiler.compile(&program).unwrap();
    decode(&baseline);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), baseline);
        assert_eq!(compiler.compile_source(TRANSFERS).unwrap(), baseline);
        assert_eq!(
            compiler.compile_source(&explicit(TRANSFERS)).unwrap(),
            baseline
        );
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn nested_explicit_transfers_keep_caller_and_nearest_loop_contexts_in_the_diamond() {
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(
            &compiler.compile_source(TRANSFERS).unwrap(),
            r#"
            assert.equal(e.main(), 102);
            assert.deepEqual(events.splice(0), [1, 2, 21, 31, 98, 22, 32, 98, 4, 98]);
            assert.equal(e.main(), 102);
            assert.deepEqual(events, [1, 2, 21, 31, 98, 22, 32, 98, 4, 98]);
        "#,
        );
    }
}

#[test]
fn skipped_and_unreachable_arguments_are_checked_after_combined_dependency_resolution() {
    for body in [
        "false && left(flag: numeric(seed: 2), trigger: 0); return 1;",
        "return 1; right(flag: numeric(2), trigger: 0);",
        "return 1; left(flag: { return 1; left(flag: numeric(seed: 2), trigger: 0); true }, trigger: 0);",
    ] {
        let source = format!(
            r#"
            fn main() {{ {body} }}
            fn left(flag: bool, trigger: number, copy: bool = flag) {{ return leaf(flag: copy); }}
            fn right(flag: bool, trigger: number, copy: bool = flag) {{ return leaf(flag, copy); }}
            fn numeric(seed: number, copy: number = seed) {{ return number_leaf(copy: copy, seed: seed); }}
            fn leaf(flag: bool, copy: bool = flag) {{ return copy; }}
            fn number_leaf(seed: number, copy: number = seed) {{ return copy; }}
        "#
        );
        for level in LEVELS {
            let mut compiler = compiler();
            compiler.set_optimization_level(level);
            let Err(CompileError::Semantic(error)) = compiler.analyze(&source) else {
                panic!("expected semantic rejection: {source}")
            };
            assert!(
                matches!(
                    error,
                    SemanticError::TypeMismatch {
                        expected: Type::Bool,
                        found: Type::Number,
                        ..
                    }
                ),
                "{source}: {error:?}"
            );
        }
    }
    let source = DIAMOND.replace(
        "+ right(sample(3));",
        "+ right(sample(3)); leaf(c: 9, a: 2);",
    );
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::None);
    let mut program = compiler.analyze(&source).unwrap();
    let TypedStatementKind::Expression(value) = &mut program.functions[0].body[1].kind else {
        panic!("unreachable expression")
    };
    let TypedExprKind::NormalizedCall(sequence) = &mut value.kind else {
        panic!("normalized call")
    };
    let span = sequence.bindings[2].span;
    sequence.bindings[2].value.kind = TypedExprKind::Bool(false);
    sequence.bindings[2].value.ty = Type::Bool.into();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::TypeMismatch {
                function: "main".into(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            })
        );
    }
}

const TRAPS: &str = r#"
    fn main(mode: number) /*number*/ {
        let xs = [7];
        return if mode == 0 {
            left(trigger: { corrupt(xs); sample(1) }, xs: { sample(2); xs })
        } else if mode == 1 {
            right(trigger: { sample(3); xs[9] }, xs: { sample(44); xs })
        } else if mode == 3 {
            left(xs: xs, trigger: { corrupt(xs); if true { return 55; } else { 0 } })
        } else {
            left(xs: xs, trigger: sample(4))
        };
    }
    fn left(xs: number[], trigger: number, copy: number[] = xs, n: number = len(copy)) /*number*/ {
        return leaf(n: n, xs: copy);
    }
    fn right(xs: number[], trigger: number, copy: number[] = xs, n: number = len(copy)) /*number*/ {
        return leaf(xs, n, copy, n);
    }
    fn leaf(xs: number[], n: number, copy: number[] = xs, check: number = len(copy)) /*number*/ {
        sample(98); return n + check;
    }
"#;

fn trap_compiler() -> Compiler {
    let mut compiler = compiler();
    compiler
        .register_host_function(
            "corrupt",
            "env",
            "corrupt",
            vec![Type::NumberArray],
            Type::Number,
        )
        .unwrap();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    compiler
}

#[test]
fn trapping_default_diamond_is_concrete_and_matches_explicit_bytes_at_all_levels() {
    let mut compiler = trap_compiler();
    let program = compiler.analyze(TRAPS).unwrap();
    assert!(
        program
            .functions
            .iter()
            .all(|f| f.return_type == Type::Number)
    );
    let baseline = compiler.compile(&program).unwrap();
    decode(&baseline);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), baseline);
        assert_eq!(compiler.compile_source(TRAPS).unwrap(), baseline);
        assert_eq!(compiler.compile_source(&explicit(TRAPS)).unwrap(), baseline);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn combined_diamond_traps_after_all_explicit_effects_or_before_later_arguments_and_defaults() {
    let mut compiler = trap_compiler();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(
            &compiler.compile_source(TRAPS).unwrap(),
            r#"
            assert.throws(() => e.main(0), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [99, 1, 2]);
            assert.throws(() => e.main(1), WebAssembly.RuntimeError);
            assert.deepEqual(events.splice(0), [3]);
            assert.equal(e.main(2), 2);
            assert.deepEqual(events.splice(0), [4, 98]);
            assert.equal(e.main(3), 55);
            assert.deepEqual(events.splice(0), [99]);
            assert.throws(() => e.main(0), WebAssembly.RuntimeError);
            assert.deepEqual(events, [99, 1, 2]);
        "#,
        );
    }
}
