#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    Compiler, ExportOptions, OptimizationLevel, Type,
    typed_ast::{TypedExprKind, TypedStatementKind},
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const SOURCE: &str = r#"
    fn identity(value: range) -> range { return value; }
    fn main() -> number {
        let a = [1, 2, 3];
        let alias = a;
        let r = identity(if true { { 0..3 } } else { loop { break 3..0 by -1; } });
        for index in r {
            let offset: number = if index == 1 { continue; } else { index };
            a[offset] += {
                sample(offset, 0);
                alias[offset] = 10 + offset;
                if offset in r { loop { break offset + 4; } } else { 0 }
            };
        }
        false && { for index in 0..1 by 0 { emit(index); } true };
        let sum = 0;
        for value in { emit(100); a } {
            emit(value);
            sum += value;
            alias[1] = 20;
            a = [99];
        }
        sum %= 100;
        return sum;
    }
    fn aborted() -> number {
        return 7 + loop {
            for index in { emit(200); break 8; 0..1 } { emit(999); }
            break 99;
        };
    }
    fn invalid() -> void {
        for index in sample(300, 0)..sample(301, 3) by sample(302, 0) { emit(999); }
    }
    fn skipped() -> bool {
        return false && { for value in [1] { value in 0..1 by 0; emit(999); } true };
    }
    fn hidden() -> void {
        return;
        false && { for index in 0..1 { for value in [1] { value in 0..1; } } true };
    }
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "sample",
            "env",
            "sample",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .unwrap();
    compiler
        .register_host_function("emit", "env", "emit", vec![Type::Number], Type::Void)
        .unwrap();
    compiler.set_exports(snapshot_exports());
    compiler
}

fn non_export_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| {
            let (id, range) = payload.unwrap().as_section()?;
            assert_ne!(id, 0, "no implicit marshaling metadata");
            (id != 7).then(|| {
                let start = usize::try_from(range.start).unwrap();
                let end = usize::try_from(range.end).unwrap();
                (id, bytes[start..end].to_vec())
            })
        })
        .collect()
}

#[test]
fn combined_features_preserve_snapshot_layout_helpers_and_export_independence() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let snapshot = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    Validator::new().validate_all(&baseline).unwrap();

    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(&baseline) {
        match payload.unwrap() {
            Payload::ImportSection(section) => {
                imports.extend(section.into_imports().map(|import| {
                    let import = import.unwrap();
                    (import.module.to_owned(), import.name.to_owned())
                }));
            }
            Payload::ExportSection(section) => {
                exports.extend(section.into_iter().map(|export| {
                    let export = export.unwrap();
                    (export.name.to_owned(), export.kind, export.index)
                }));
            }
            Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    assert_eq!(
        imports,
        [
            ("env".into(), "sample".into()),
            ("env".into(), "emit".into())
        ]
    );
    assert_eq!(
        exports,
        [
            ("main".into(), ExternalKind::Func, 3),
            ("aborted".into(), ExternalKind::Func, 4),
            ("invalid".into(), ExternalKind::Func, 5),
            ("skipped".into(), ExternalKind::Func, 6),
            ("memory".into(), ExternalKind::Memory, 0),
            ("__alloc".into(), ExternalKind::Func, 8),
        ]
    );
    // Two imports, six user functions, seven memory helpers, remainder,
    // then Construct / Validate / Contains. These expectations are independent
    // of the production helper enumeration.
    assert_eq!(bodies.len(), 17);
    let calls = |body: &wasmparser::FunctionBody<'_>| {
        body.get_operators_reader()
            .unwrap()
            .into_iter()
            .filter_map(|op| match op.unwrap() {
                Operator::Call { function_index } => Some(function_index),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let main_calls = calls(&bodies[1]);
    for index in [0, 1, 2, 8, 10, 11, 15, 16, 17, 18] {
        assert!(
            main_calls.contains(&index),
            "missing call to {index}: {main_calls:?}"
        );
    }
    let contains = bodies[16]
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(contains.iter().any(|op| matches!(op, Operator::I64RemS)));

    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let selected_bytes = compiler.compile(&program).unwrap();
        Validator::new().validate_all(&selected_bytes).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(selected_bytes, baseline);
        }
        let baseline = selected_bytes;
        assert_eq!(compiler.compile(&program).unwrap(), baseline, "{level:?}");
        for selection in [
            ExportOptions::None,
            ExportOptions::Only(vec!["main".into()]),
        ] {
            compiler.set_exports(selection);
            let bytes = compiler.compile(&program).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(non_export_sections(&bytes), non_export_sections(&baseline));
        }
        compiler.set_exports(snapshot_exports());
        assert_eq!(program, snapshot);
    }
}

fn snapshot_exports() -> ExportOptions {
    ExportOptions::Only(
        ["main", "aborted", "invalid", "skipped"]
            .map(String::from)
            .to_vec(),
    )
}

#[test]
fn combined_unreachable_skipped_unexported_children_are_still_checked() {
    let mut compiler = compiler();
    for source in [
        "fn main() -> void {} fn hidden() -> void { return; false && { for i in 0..1 { for v in [1] { v in [1]; } } true }; }",
        "fn main() -> void {} fn hidden() -> void { false && { for i in 0..1 { for v in [1] { continue; v += i; } } true }; }",
        "fn main() -> void {} fn hidden() -> void { return; for i in 0..1 { for v in [1] { break i; } } }",
    ] {
        let expected = compiler.analyze(source).unwrap_err();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile_source(source).unwrap_err(), expected);
        }
    }

    let mut program = compiler.analyze(SOURCE).unwrap();
    let TypedStatementKind::Expression(expression) = &mut program.functions[5].body[1].kind else {
        panic!("hidden logical expression");
    };
    let TypedExprKind::Binary { right, .. } = &mut expression.kind else {
        panic!("logical RHS");
    };
    let TypedExprKind::Block(block) = &mut right.kind else {
        panic!("value block");
    };
    let TypedStatementKind::For { body, .. } = &mut block.statements[0].kind else {
        panic!("range for");
    };
    let TypedStatementKind::For { iterable, .. } = &mut body[0].kind else {
        panic!("array for");
    };
    iterable.ty = Type::Range.into();
    compiler.set_exports(ExportOptions::None);
    let expected = compiler.compile(&program).unwrap_err();
    assert!(expected.span().is_some());
    for selection in [
        ExportOptions::None,
        ExportOptions::Only(vec!["main".into()]),
    ] {
        compiler.set_exports(selection);
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap_err(), expected);
        }
    }
}

#[test]
#[ignore = "requires Node.js"]
fn combined_features_execute_effects_aliases_transfers_and_traps_at_all_levels() {
    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const events = [];
        const { exports: e } = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
            env: { sample(tag, value) { events.push(tag); return value; }, emit(value) { events.push(value); } }
        });
        assert.equal(e.main(), 34);
        assert.deepEqual(events.splice(0), [0, 2, 100, 5, 20, 9]);
        assert.equal(e.aborted(), 15);
        assert.deepEqual(events.splice(0), [200]);
        assert.equal(e.skipped(), 0);
        assert.deepEqual(events, []);
        assert.throws(() => e.invalid(), WebAssembly.RuntimeError);
        assert.deepEqual(events, [300, 301, 302]);
    "#;
    let mut compiler = compiler();
    let baseline = compiler.compile_source(SOURCE).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(SOURCE).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(bytes, baseline);
        }
        assert_eq!(compiler.compile_source(SOURCE).unwrap(), bytes);
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
