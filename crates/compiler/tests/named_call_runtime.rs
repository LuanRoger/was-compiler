#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    Compiler, ExportOptions, OptimizationLevel, Type,
    tokens::Span,
    typed_ast::{
        CallBindingOrigin, ExprResult, FunctionRef, TypedBlock, TypedCallBinding, TypedClosureBody,
        TypedExpr, TypedExprKind, TypedNormalizedCall, TypedProgram, TypedStatementKind,
        TypedValueBlock, VariableId,
    },
};
use wasmparser::{Parser, Payload, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

// Markers are ordinary comments for the legacy frontend and become argument names
// for the primary source tests. The typed fallback normalizes these same written
// operands, not a reordered source program with different observable effects.
const CALLEES: &str = r#"
    fn digits(a: number, b: number, c: number) -> number {
        emit(901); return a * 100 + b * 10 + c;
    }
    fn mixed(a: number, b: number, c: number) -> number {
        emit(902); return a * 100 + b * 10 + c;
    }
    fn pair(a: number, b: number) -> number {
        emit(900); return a * 10 + b;
    }
    fn inspect(arr: number[], extra: number) -> number {
        emit(903); arr[1] = 8;
        return arr[0] * 100 + arr[1] * 10 + extra;
    }
"#;

const EFFECTS: &str = r#"
    fn reordered() -> number {
        return digits(/*c*/ sample(1, 3), /*a*/ sample(2, 1), /*b*/ sample(3, 2));
    }
    fn prefix() -> number {
        return mixed(sample(4, 4), /*c*/ sample(5, 6), /*b*/ sample(6, 5));
    }
    fn scalar() -> number {
        let x = 7;
        let result = pair(/*b*/ x, /*a*/ { x = 9; sample(10, x) });
        emit(x);
        return result;
    }
    fn arrays() -> number {
        let values = [1, 2];
        let alias = values;
        let result = inspect(/*arr*/ values, /*extra*/ {
            alias[0] = 40;
            values = [99];
            sample(11, 2)
        });
        emit(alias[0]); emit(alias[1]); emit(values[0]);
        return result + alias[1] + values[0];
    }
    fn nested() -> number {
        return digits(/*c*/ pair(/*b*/ sample(12, 2), /*a*/ sample(13, 1)),
                      /*a*/ sample(14, 3), /*b*/ sample(15, 4));
    }
"#;

const TRANSFERS: &str = r#"
    fn returned() -> number {
        return 5 + pair(/*b*/ sample(20, 1), /*a*/ {
            sample(21, 0); return 77; 0
        });
    }
    fn value_break() -> number {
        return 10 + loop {
            pair(/*b*/ sample(30, 1), /*a*/ if true {
                { sample(31, 0); break 8; 0 }
            } else { 0 });
            break 99;
        };
    }
    fn statement_break() -> number {
        let i = 0;
        while i < 3 {
            i += 1;
            pair(/*b*/ sample(40 + i, i), /*a*/ {
                if i == 2 { break; }
                0
            });
            emit(49);
        }
        return i;
    }
    fn continued() -> number {
        let i = 0;
        let sum = 0;
        while i < 3 {
            i += 1;
            sum += 100 + pair(/*b*/ sample(50 + i, i), /*a*/ if i < 3 {
                { continue; 0 }
            } else { 4 });
            emit(59);
        }
        return sum;
    }
    fn for_continue() -> number {
        let sum = 0;
        for i in [1, 2, 3] {
            sum += pair(/*b*/ sample(60 + i, i), /*a*/ {
                if i == 2 { continue; }
                1
            });
        }
        return sum;
    }
    fn inner_loop() -> number {
        return pair(/*b*/ sample(70, 2), /*a*/ loop {
            let i = 0;
            while i < 2 { i += 1; if i == 1 { continue; } emit(71); }
            break 3;
        });
    }
    fn first_never() -> number {
        return pair(/*b*/ { emit(80); return 81; 0 }, /*a*/ sample(899, 0));
    }
    fn trapped() -> number {
        return pair(/*b*/ sample(90, 1), /*a*/ {
            emit(91); let a = [1]; a[2]
        });
    }
    fn skipped() -> bool {
        return false && pair(/*b*/ sample(899, 0), /*a*/ {
            let a = [1]; a[2]
        }) == 0;
    }
    fn hidden() -> number {
        return 82;
        pair(/*b*/ { return 83; 0 }, /*a*/ sample(899, 0));
    }
"#;

const EFFECT_SCRIPT: &str = r#"
    assert.equal(e.reordered(), 123);
    assert.deepEqual(events.splice(0), [1, 2, 3, 901]);
    assert.equal(e.prefix(), 456);
    assert.deepEqual(events.splice(0), [4, 5, 6, 902]);
    assert.equal(e.scalar(), 97);
    assert.deepEqual(events.splice(0), [10, 900, 9]);
    assert.equal(e.arrays(), 4189);
    assert.deepEqual(events.splice(0), [11, 903, 40, 8, 99]);
    assert.equal(e.nested(), 352);
    assert.deepEqual(events.splice(0), [12, 13, 900, 14, 15, 901]);
"#;

const TRANSFER_SCRIPT: &str = r#"
    assert.equal(e.returned(), 77);
    assert.deepEqual(events.splice(0), [20, 21]);
    assert.equal(e.value_break(), 18);
    assert.deepEqual(events.splice(0), [30, 31]);
    assert.equal(e.statement_break(), 2);
    assert.deepEqual(events.splice(0), [41, 900, 49, 42]);
    assert.equal(e.continued(), 143);
    assert.deepEqual(events.splice(0), [51, 52, 53, 900, 59]);
    assert.equal(e.for_continue(), 24);
    assert.deepEqual(events.splice(0), [61, 900, 62, 63, 900]);
    assert.equal(e.inner_loop(), 32);
    assert.deepEqual(events.splice(0), [70, 71, 900]);
    assert.equal(e.first_never(), 81);
    assert.deepEqual(events.splice(0), [80]);
    assert.throws(() => e.trapped(), WebAssembly.RuntimeError);
    assert.deepEqual(events.splice(0), [90, 91]);
    assert.equal(e.skipped(), 0);
    assert.deepEqual(events, []);
    assert.equal(e.hidden(), 82);
    assert.deepEqual(events, []);
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
    // Even unused imports belong to the analyzed snapshot and must be supplied.
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler
}

fn source(fixture: &str, named: bool) -> String {
    let mut source = format!("{CALLEES}\n{fixture}");
    if named {
        for name in ["a", "b", "c", "arr", "extra"] {
            source = source.replace(&format!("/*{name}*/"), &format!("{name}:"));
        }
    }
    source
}

fn run_node(bytes: &[u8], assertions: &str) {
    Validator::new().validate_all(bytes).unwrap();
    let script = format!(
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const events = [];
        const module = new WebAssembly.Module(bytes);
        assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
            [['env', 'sample'], ['env', 'emit'], ['env', 'unused']]);
        const e = new WebAssembly.Instance(module, {{ env: {{
            sample(tag, value) {{ events.push(tag); return value; }},
            emit(value) {{ events.push(value); }},
            unused() {{ assert.fail('unused import executed'); }}
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
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_source(fixture: &str, script: &str) {
    let mut compiler = compiler();
    let source = source(fixture, true);
    let baseline_program = compiler.analyze(&source).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let program = compiler.analyze(&source).expect("named source analysis");
        let bytes = compiler.compile(&program).unwrap();
        assert_eq!(compiler.compile_source(&source).unwrap(), bytes);
        assert_eq!(program, baseline_program, "{level:?}");
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        run_node(&bytes, script);
    }
}

#[test]
#[ignore = "requires Node.js on PATH; named source frontend is integrated separately"]
fn named_source_effects_execute_at_all_levels() {
    run_source(EFFECTS, EFFECT_SCRIPT);
}

#[test]
#[ignore = "requires Node.js on PATH; named source frontend is integrated separately"]
fn named_source_transfers_execute_at_all_levels() {
    run_source(TRANSFERS, TRANSFER_SCRIPT);
}

// This fixture adapter is deliberately local to the runtime suite. It bridges the
// approved typed-tree seam while named parsing is developed independently. Only
// these four callees are normalized; host calls retain their legacy path.
fn typed_fixture(compiler: &Compiler, fixture: &str) -> TypedProgram {
    let mut program = compiler.analyze(&source(fixture, false)).unwrap();
    for function in &mut program.functions {
        // Legacy fixture declarations stay below these sparse generated IDs.
        let mut next_id = 10_000;
        normalize_block(&mut function.body, &mut next_id);
    }
    program
}

fn normalize_block(block: &mut TypedBlock, next_id: &mut usize) {
    for statement in block {
        match &mut statement.kind {
            TypedStatementKind::Closure(closure) => {
                let mut helper_next_id = 10_000;
                match &mut closure.body {
                    TypedClosureBody::Expression(expr) => normalize_expr(expr, &mut helper_next_id),
                    TypedClosureBody::Block(block) => {
                        normalize_value_block(block, &mut helper_next_id)
                    }
                }
            }
            TypedStatementKind::Let { value, .. }
            | TypedStatementKind::Expression(value)
            | TypedStatementKind::Return(Some(value))
            | TypedStatementKind::BreakValue { value, .. } => normalize_expr(value, next_id),
            TypedStatementKind::Assignment { target, value }
            | TypedStatementKind::CompoundAssignment { target, value, .. } => {
                normalize_expr(target, next_id);
                normalize_expr(value, next_id);
            }
            TypedStatementKind::If { cond, then, else_ } => {
                normalize_expr(cond, next_id);
                normalize_block(then, next_id);
                if let Some(else_) = else_ {
                    normalize_block(else_, next_id);
                }
            }
            TypedStatementKind::Match { scrutinee, arms } => {
                normalize_expr(scrutinee, next_id);
                for arm in arms {
                    normalize_expr(&mut arm.body, next_id);
                }
            }
            TypedStatementKind::While { cond, body, .. } => {
                normalize_expr(cond, next_id);
                normalize_block(body, next_id);
            }
            TypedStatementKind::For { iterable, body, .. } => {
                normalize_expr(iterable, next_id);
                normalize_block(body, next_id);
            }
            TypedStatementKind::Loop { body, .. } => normalize_block(body, next_id),
            TypedStatementKind::Break { .. }
            | TypedStatementKind::Continue { .. }
            | TypedStatementKind::Return(None) => {}
        }
    }
}

fn normalize_value_block(block: &mut TypedValueBlock, next_id: &mut usize) {
    normalize_block(&mut block.statements, next_id);
    if let Some(tail) = &mut block.tail {
        normalize_expr(tail, next_id);
    }
}

fn normalize_expr(expr: &mut TypedExpr, next_id: &mut usize) {
    match &mut expr.kind {
        TypedExprKind::Unary { expr, .. } => normalize_expr(expr, next_id),
        TypedExprKind::Binary { left, right, .. } => {
            normalize_expr(left, next_id);
            normalize_expr(right, next_id);
        }
        TypedExprKind::Range {
            start, end, step, ..
        } => {
            normalize_expr(start, next_id);
            normalize_expr(end, next_id);
            if let Some(step) = step {
                normalize_expr(step, next_id);
            }
        }
        TypedExprKind::Call { func, name, args } => {
            let mapping: &[usize] = match (func, name.as_str()) {
                (FunctionRef::User(_), "digits") => &[1, 2, 0],
                (FunctionRef::User(_), "mixed") => &[0, 2, 1],
                (FunctionRef::User(_), "pair") => &[1, 0],
                (FunctionRef::User(_), "inspect") => &[0, 1],
                _ => {
                    for arg in args {
                        normalize_expr(arg, next_id);
                    }
                    return;
                }
            };
            assert_eq!(args.len(), mapping.len());
            let values = std::mem::take(args);
            let mut bindings = Vec::with_capacity(values.len());
            for (argument, mut value) in values.into_iter().enumerate() {
                let id = VariableId(*next_id);
                *next_id += 1;
                normalize_expr(&mut value, next_id);
                // All Never operands in these fixtures occupy numeric formals.
                let ty = value.ty.value_type().unwrap_or(Type::Number);
                bindings.push(TypedCallBinding {
                    id,
                    ty,
                    value,
                    origin: CallBindingOrigin::Explicit { argument },
                    span: Span::new(0, 0),
                });
            }
            let formal_to_binding = mapping.iter().map(|&i| bindings[i].id).collect();
            *args = mapping
                .iter()
                .map(|&i| TypedExpr {
                    kind: TypedExprKind::Identifier {
                        id: bindings[i].id,
                        name: "invocation-local".into(),
                    },
                    ty: bindings[i].ty.into(),
                })
                .collect();
            let call = TypedExpr {
                kind: std::mem::replace(&mut expr.kind, TypedExprKind::Bool(false)),
                // The canonical call keeps the signature result even when an
                // initializer transfers and the enclosing sequence is Never.
                ty: Type::Number.into(),
            };
            expr.kind = TypedExprKind::NormalizedCall(TypedNormalizedCall {
                bindings,
                formal_to_binding,
                call: Box::new(call),
            });
        }
        TypedExprKind::Loop { body, .. } => normalize_block(body, next_id),
        TypedExprKind::Block(block) => normalize_value_block(block, next_id),
        TypedExprKind::If { cond, then, else_ } => {
            normalize_expr(cond, next_id);
            normalize_value_block(then, next_id);
            if let Some(else_) = else_ {
                normalize_value_block(else_, next_id);
            }
        }
        TypedExprKind::Match { scrutinee, arms } => {
            normalize_expr(scrutinee, next_id);
            for arm in arms {
                normalize_expr(&mut arm.body, next_id);
            }
        }
        TypedExprKind::LexicalCall { args: values, .. } | TypedExprKind::Array(values, _) => {
            for value in values {
                normalize_expr(value, next_id);
            }
        }
        TypedExprKind::Index { expr, index } => {
            normalize_expr(expr, next_id);
            normalize_expr(index, next_id);
        }
        TypedExprKind::NormalizedCall(_) => panic!("legacy fixture was already normalized"),
        TypedExprKind::Number(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Identifier { .. }
        | TypedExprKind::Captured { .. } => {}
    }
}

fn run_typed(fixture: &str, script: &str) {
    let mut compiler = compiler();
    let program = typed_fixture(&compiler, fixture);
    let snapshot = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile(&program).unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(bytes, baseline, "{level:?}");
        }
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        run_node(&bytes, script);
        assert_eq!(program, snapshot);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn typed_sequences_execute_effects_at_all_levels() {
    run_typed(EFFECTS, EFFECT_SCRIPT);
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn typed_sequences_execute_bounded_transfers_and_traps_at_all_levels() {
    run_typed(TRANSFERS, TRANSFER_SCRIPT);
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
fn typed_sequences_preserve_import_snapshot_export_choices_and_determinism() {
    let mut compiler = compiler();
    let fixture = format!("{EFFECTS}\n{TRANSFERS}");
    let program = typed_fixture(&compiler, &fixture);
    let snapshot = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    let imports: Vec<_> = Parser::new(0)
        .parse_all(&baseline)
        .filter_map(|payload| match payload.unwrap() {
            Payload::ImportSection(section) => Some(
                section
                    .into_imports()
                    .map(|import| {
                        let import = import.unwrap();
                        (import.module.to_owned(), import.name.to_owned())
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        imports,
        [
            ("env".into(), "sample".into()),
            ("env".into(), "emit".into()),
            ("env".into(), "unused".into())
        ]
    );
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for selection in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["reordered".into(), "continued".into()]),
    ] {
        compiler.set_optimization_level(OptimizationLevel::None);
        compiler.set_exports(selection.clone());
        let expected = compiler.compile(&program).unwrap();
        Validator::new().validate_all(&expected).unwrap();
        assert_eq!(
            non_export_sections(&expected),
            non_export_sections(&baseline)
        );
        let mut exports = Vec::new();
        for payload in Parser::new(0).parse_all(&expected) {
            if let Payload::ExportSection(section) = payload.unwrap() {
                exports.extend(
                    section
                        .into_iter()
                        .map(|export| export.unwrap().name.to_owned()),
                );
            }
        }
        let functions: Vec<_> = exports
            .iter()
            .filter(|name| !["memory", "__alloc"].contains(&name.as_str()))
            .cloned()
            .collect();
        match selection {
            ExportOptions::All => assert_eq!(
                functions,
                program
                    .functions
                    .iter()
                    .map(|f| f.name.clone())
                    .collect::<Vec<_>>()
            ),
            ExportOptions::None => assert!(functions.is_empty()),
            ExportOptions::Only(_) => assert_eq!(functions, ["reordered", "continued"]),
        }
        assert!(exports.iter().any(|name| name == "memory"));
        assert!(exports.iter().any(|name| name == "__alloc"));
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let selected_bytes = compiler.compile(&program).unwrap();
            wasmparser::Validator::new()
                .validate_all(&selected_bytes)
                .unwrap();
            if level == OptimizationLevel::None {
                assert_eq!(selected_bytes, expected);
            }
            let expected = selected_bytes;
            assert_eq!(compiler.compile(&program).unwrap(), expected, "{level:?}");
            assert_eq!(program, snapshot);
        }
    }
    // Confirm this fixture actually crosses the Never/signature-result seam.
    let first_never = program
        .functions
        .iter()
        .find(|f| f.name == "first_never")
        .unwrap();
    let TypedStatementKind::Return(Some(expr)) = &first_never.body[0].kind else {
        panic!("first_never return");
    };
    assert_eq!(expr.ty, ExprResult::Never);
    let TypedExprKind::NormalizedCall(sequence) = &expr.kind else {
        panic!("normalized first_never call");
    };
    assert_eq!(sequence.call.ty, Type::Number);
}
