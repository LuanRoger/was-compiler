#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};
use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type};
use wasmparser::{Parser, Payload, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function(
            "mark",
            "env",
            "mark",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .unwrap();
    compiler
        .register_host_function("bounce", "env", "bounce", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();
    compiler
}

fn run(source: &str, assertions: &str) {
    let mut compiler = compiler();
    let program = compiler.analyze(source).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        assert_eq!(compiler.analyze(source).unwrap(), program);
        let script = format!(
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const events = [];
            let e;
            let corruptHandle = 0;
            let bouncing = false;
            const module = new WebAssembly.Module(bytes);
            assert.deepEqual(WebAssembly.Module.imports(module).map(i => [i.module, i.name]),
                [['env', 'mark'], ['env', 'bounce'], ['env', 'unused']]);
            e = new WebAssembly.Instance(module, {{ env: {{
                mark(id, value) {{
                    events.push(id);
                    if (id === 10 && corruptHandle) {{
                        // Deliberately violate trusted-host metadata to observe the
                        // later permitted descriptor-validation trap.
                        new DataView(e.memory.buffer).setUint32(corruptHandle + 4, 0xffffffff, true);
                    }}
                    return value;
                }},
                bounce() {{
                    events.push(4);
                    if (bouncing) return 4;
                    bouncing = true;
                    const value = e.reenter(2);
                    bouncing = false;
                    events.push(5);
                    return value;
                }},
                unused() {{ throw new Error('unused import executed'); }}
            }} }}).exports;
            {assertions}
        "#
        );
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

const EFFECTS: &str = r#"
    fn dependent(a: number, b: number = a + 1, c: number = b * 2) -> number { return a * 100 + b * 10 + c; }
    fn capture(a: number, b: number = a, c: number = b) -> number { return a * 100 + b * 10 + c; }
    fn written() -> number { return dependent(c: mark(1, 9), a: mark(2, 3)); }
    fn scalar() -> number { let x = 7; let result = capture(a: x, c: { x = 9; mark(3, x) }); return result + x; }
    fn inspect(xs: number[], trigger: number, n: number = len(xs), same: bool = xs == xs) -> number {
        return xs[0] * 100 + n * 10 + if same { 1 } else { 0 };
    }
    fn arrays() -> number {
        let xs = [1, 2]; let alias = xs;
        let result = inspect(xs: xs, trigger: { alias[0] = 8; xs = [99]; mark(6, 0) });
        return result + xs[0];
    }
    fn alias(xs: number[], same: number[] = xs) -> number { same[0] = 5; return xs[0]; }
    fn aliases() -> number { let xs = [1]; return alias(xs); }
    fn text(first: string, second: string = first, same: bool = first == second) -> bool { return same; }
    fn texts() -> bool { return text("é"); }
    fn range_copy(r: range, copy: range = r) -> bool { return 2 in copy; }
    fn ranges() -> bool { return range_copy(0..3); }
    fn recursive(n: number, next: number = n - 1) -> number { if n == 0 { return 0; } return n + recursive(next); }
    fn recursion() -> number { return recursive(4); }
    fn reentrant(a: number, b: number = a + 1, c: number = b + 1) -> number { return a * 100 + b * 10 + c; }
    fn inner() -> number { return dependent(2); }
    fn reenter(a: number) -> number { return reentrant(a: a, c: bounce()); }
    fn signed(x: number = -0) -> number { return x; }
    fn infinity(x: number = 1 / 0) -> number { return x; }
    fn nan(x: number = 0 / 0) -> number { return x; }
    fn arithmetic() -> number { return signed(); }
    fn arithmetic_inf() -> number { return infinity(); }
    fn arithmetic_nan() -> number { return nan(); }
"#;

#[test]
#[ignore = "requires Node.js on PATH"]
fn defaults_capture_values_share_handles_and_use_invocation_local_storage_at_all_levels() {
    // Range signatures stay private even though their defaults may copy handles.
    let mut selected = compiler();
    selected.set_exports(ExportOptions::Only(vec!["ranges".into()]));
    let range_source = "fn copy(r: range, other: range = r) -> bool { return 2 in other; } fn ranges() -> bool { return copy(0..3); }";
    for level in LEVELS {
        selected.set_optimization_level(level);
        let range_bytes = selected.compile_source(range_source).unwrap();
        Validator::new().validate_all(&range_bytes).unwrap();
        let script = r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: {
                mark() { throw new Error('unexpected host call'); },
                bounce() { throw new Error('unexpected host call'); },
                unused() { throw new Error('unexpected host call'); }
            } }).exports;
            assert.equal(e.ranges(), 1);
            assert.deepEqual(Object.keys(e), ['ranges', 'memory', '__alloc']);
        "#;
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&range_bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "{level:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let source = EFFECTS
        .replace(
            "    fn range_copy(r: range, copy: range = r) -> bool { return 2 in copy; }",
            "",
        )
        .replace("    fn ranges() -> bool { return range_copy(0..3); }", "");
    run(
        &source,
        r#"
        assert.equal(e.written(), 349); assert.deepEqual(events.splice(0), [1, 2]);
        assert.equal(e.scalar(), 788); assert.deepEqual(events.splice(0), [3]);
        assert.equal(e.arrays(), 920); assert.deepEqual(events.splice(0), [6]);
        assert.equal(e.aliases(), 5);
        assert.equal(e.texts(), 1);
        assert.equal(e.inner(), 236); assert.equal(e.recursion(), 10);
        assert.equal(e.reenter(7), 1014); assert.deepEqual(events.splice(0), [4, 4, 5]);
        assert.equal(e.dependent.length, 3);
        assert.equal(e.dependent(4, 5, 6), 456);
        assert.equal(e.dependent(4, 0, 0), 400);
        assert.ok(Object.is(e.arithmetic(), -0));
        assert.equal(e.arithmetic_inf(), Infinity);
        assert.ok(Number.isNaN(e.arithmetic_nan()));
        assert.deepEqual(events, []);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn explicit_effects_precede_default_traps_and_transfers_preserve_caller_context() {
    run(
        r#"
        fn consume(xs: number[], trigger: number, n: number = len(xs)) -> number { return n; }
        fn trapped(xs: number[]) -> number { return consume(xs: xs, trigger: mark(10, 0)); }
        fn bypass(xs: number[]) -> number { return consume(xs: xs, trigger: mark(11, 0), n: 7); }
        fn returned(xs: number[]) -> number { return 5 + consume(xs: xs, trigger: { mark(12, 0); return 77; 0 }); }
        fn broken(xs: number[]) -> number { return 10 + loop { consume(xs: xs, trigger: { mark(13, 0); break 8; 0 }); break 99; }; }
        fn continued(xs: number[]) -> number {
            let n = 0; for i in [1, 2, 3] { n += 1; consume(xs: xs, trigger: { mark(20 + i, 0); continue; 0 }); } return n;
        }
        fn skipped(xs: number[]) -> bool { return false && consume(xs: xs, trigger: mark(99, 0)) == 0; }
        fn hidden(xs: number[]) -> number { return 8; consume(xs: xs, trigger: mark(99, 0)); }
    "#,
        r#"
        assert.throws(() => e.trapped(0), WebAssembly.RuntimeError);
        assert.deepEqual(events, []); // Existing parameter-entry validation precedes body effects.
        const handle = e.__alloc(16) >>> 0;
        let view = new DataView(e.memory.buffer);
        view.setUint32(handle, handle + 8, true);
        view.setUint32(handle + 4, 1, true);
        view.setFloat64(handle + 8, 9, true);
        corruptHandle = handle;
        assert.throws(() => e.trapped(handle), WebAssembly.RuntimeError);
        assert.deepEqual(events.splice(0), [10]);
        new DataView(e.memory.buffer).setUint32(handle + 4, 1, true);
        corruptHandle = 0;
        assert.equal(e.bypass(handle), 7); assert.deepEqual(events.splice(0), [11]);
        assert.equal(e.returned(handle), 77); assert.deepEqual(events.splice(0), [12]);
        assert.equal(e.broken(handle), 18); assert.deepEqual(events.splice(0), [13]);
        assert.equal(e.continued(handle), 3); assert.deepEqual(events.splice(0), [21, 22, 23]);
        assert.equal(e.skipped(handle), 0); assert.equal(e.hidden(handle), 8); assert.deepEqual(events, []);
    "#,
    );
}

#[test]
fn full_positional_default_declarations_preserve_bytes_import_snapshot_and_selection() {
    let original = "fn f(a: number, b: number) -> number { return a + b; } fn main() -> number { return f(2, 3); }";
    let defaulted = original.replace("b: number)", "b: number = a + 1)");
    let mut compiler = compiler();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let program = compiler.analyze(&defaulted).unwrap();
    let before = compiler.compile(&program).unwrap();
    compiler
        .register_host_function("later", "later", "later", vec![], Type::Void)
        .unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let selected_bytes = compiler.compile(&program).unwrap();
        wasmparser::Validator::new()
            .validate_all(&selected_bytes)
            .unwrap();
        if level == OptimizationLevel::None {
            assert_eq!(selected_bytes, before);
        }
        let before = selected_bytes;
        assert_eq!(compiler.compile(&program).unwrap(), before);
    }
    let mut legacy = self::compiler();
    legacy.set_exports(ExportOptions::Only(vec!["main".into()]));
    assert_eq!(legacy.compile_source(original).unwrap(), before);
    let exports: Vec<_> = Parser::new(0)
        .parse_all(&before)
        .filter_map(|payload| match payload.unwrap() {
            Payload::ExportSection(reader) => Some(
                reader
                    .into_iter()
                    .map(|entry| entry.unwrap().name.to_owned())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(exports, ["main"]);
}
