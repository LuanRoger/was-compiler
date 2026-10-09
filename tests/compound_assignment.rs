#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    Compiler, ExportOptions, OptimizationLevel, Type, ast::BinaryOp, typed_ast::TypedStatementKind,
};
use wasmparser::{Parser, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const SOURCE: &str = "
    fn main() -> number {
        let values = [10, 20];
        let alias = values;
        alias[0] += mutate(values);
        let n = 24;
        n -= 2;
        n *= 3;
        n /= 2;
        n %= 10;
        return values[0] + n;
    }
    fn mutate(values: number[]) -> number { values[0] = 100; return 2; }
    fn hidden(n: number) -> void { return; n %= 3; }
";

fn validate(bytes: &[u8]) {
    Validator::new().validate_all(bytes).unwrap();
}

fn sections_without_exports(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
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
fn compound_analysis_compilation_and_export_selection_preserve_owned_input_and_indices() {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("tick", "env", "tick", vec![], Type::Void)
        .unwrap();

    let program = compiler.analyze(SOURCE).unwrap();
    let original = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    validate(&baseline);

    compiler
        .register_host_function("later", "env", "later", vec![], Type::Void)
        .unwrap();

    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["main".into()]),
    ] {
        compiler.set_exports(exports);
        let expected = compiler.compile(&program).unwrap();
        validate(&expected);
        assert_eq!(
            sections_without_exports(&expected),
            sections_without_exports(&baseline)
        );

        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap(), expected);
            assert_eq!(program, original);
        }
    }

    let TypedStatementKind::CompoundAssignment { target, op, value } =
        &program.functions[0].body[2].kind
    else {
        panic!("expected resolved compound assignment");
    };

    assert_eq!(*op, BinaryOp::Add);
    assert_eq!(target.ty, Type::Number);
    assert_eq!(value.ty, Type::Number);
}

#[test]
fn scalar_compounds_match_explicit_arithmetic_bytes_at_every_level() {
    for (compound, binary) in [
        ("+=", "+"),
        ("-=", "-"),
        ("*=", "*"),
        ("/=", "/"),
        ("%=", "%"),
    ] {
        let source =
            format!("fn main(n: number, rhs: number) -> number {{ n {compound} rhs; return n; }}");
        let explicit = format!(
            "fn main(n: number, rhs: number) -> number {{ n = n {binary} rhs; return n; }}"
        );
        let mut compiler = Compiler::new();
        let expected = compiler.compile_source(&explicit).unwrap();
        validate(&expected);

        for level in LEVELS {
            compiler.set_optimization_level(level);
            let program = compiler.analyze(&source).unwrap();
            let bytes = compiler.compile_source(&source).unwrap();
            validate(&bytes);
            assert_eq!(bytes, expected, "{compound}, {level:?}");
            assert_eq!(compiler.compile(&program).unwrap(), expected);
        }
    }
}

#[test]
fn compound_rejections_keep_stage_message_and_span_at_every_level() {
    for body in [
        "let flag = true; flag += 1;",
        "let text = \"hello\"; text *= 2;",
        "let values = [1]; values -= 1;",
        "let n = 1; n %= false;",
        "let values = [1]; values[false] /= 2;",
        "unknown += missing;",
        "let n = 1; return; n += false;",
        "if false { let n = 1; n *= false; }",
        "let n = 1; n += noop();",
        "let n = 1; n +=;",
        "let n = 1; n += 2 += 3;",
        "let n = 1; let result = n += 2;",
        "let values = [1]; len(values) += 2;",
        "let values = [1]; (values[0]) += 2;",
    ] {
        let source = format!("fn main() -> void {{ {body} }} fn noop() -> void {{}}");
        let mut compiler = Compiler::new();
        let expected = compiler.compile_source(&source).unwrap_err();
        let expected_kind = std::mem::discriminant(&expected);
        let expected_message = expected.to_string();
        let expected_span = expected.span();
        assert!(expected_span.is_some(), "{body}");

        for level in LEVELS {
            compiler.set_optimization_level(level);
            let error = compiler.compile_source(&source).unwrap_err();
            assert_eq!(std::mem::discriminant(&error), expected_kind, "{body}");
            assert_eq!(error.to_string(), expected_message, "{body}");
            assert_eq!(error.span(), expected_span, "{body}");
        }
    }
}

fn run_node(bytes: &[u8], script: &str) {
    validate(bytes);
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

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_compounds_observe_old_elements_aliasing_shadowing_loops_and_all_numeric_operators() {
    let mut source = String::from(SOURCE);
    source.push_str(
        "fn flow() -> number {
            let total = 1;
            let i = 0;
            while i < 3 {
                if i == 1 { let total = 10; total *= 2; tick(); }
                total += i;
                i += 1;
            }
            return total;
        }
        fn from_bool(flag: bool) -> number { if flag { return 2; } else { return 0; } }
        fn touch() -> number { tick(); return 1; }
        fn local_lazy(n: number) -> number {
            n += from_bool(false && len([touch()]) == 1);
            return n;
        }
        fn indexed_lazy(a: number[]) -> number {
            a[0] += from_bool(true || len([touch()]) == 1);
            return a[0];
        }",
    );

    for (index, token) in ["+=", "-=", "*=", "/=", "%="].into_iter().enumerate() {
        source.push_str(&format!(
            "fn scalar{index}(n: number, rhs: number) -> number {{ n {token} rhs; return n; }}
             fn indexed{index}(a: number[], rhs: number) -> number {{ a[0] {token} rhs; return a[0]; }}"
        ));
    }

    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        let ticks = 0;
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
            env: { tick() { ticks++; } }
        }).exports;
        assert.equal(e.main(), 15);
        assert.equal(e.flow(), 4);
        assert.equal(ticks, 1);
        const a = e.__alloc(16);
        let view = new DataView(e.memory.buffer);
        view.setUint32(a, a + 8, true);
        view.setUint32(a + 4, 1, true);
        view.setFloat64(a + 8, 10, true);
        assert.equal(e.local_lazy(10), 10);
        assert.equal(e.indexed_lazy(a), 12);
        assert.equal(ticks, 1);
        const operations = [(a,b) => a+b, (a,b) => a-b, (a,b) => a*b, (a,b) => a/b, (a,b) => a%b];
        const cases = [[11,3], [-11,3], [11,-3], [5.5,2], [0,2], [-0,2], [1,0], [1,-0],
            [NaN,1], [1,NaN], [Infinity,2], [-Infinity,2], [2,Infinity], [2,-Infinity],
            [Number.MAX_VALUE,3], [Number.MAX_VALUE,Number.MIN_VALUE],
            [Number.MIN_VALUE,Number.MIN_VALUE*2], [-Number.MIN_VALUE,Number.MIN_VALUE*2]];
        for (let i = 0; i < operations.length; i++) for (const [n,rhs] of cases) {
            const expected = operations[i](n,rhs);
            assert.ok(Object.is(e['scalar' + i](n,rhs), expected));
            view = new DataView(e.memory.buffer);
            view.setFloat64(a + 8, n, true);
            assert.ok(Object.is(e['indexed' + i](a,rhs), expected));
            assert.ok(Object.is(view.getFloat64(a + 8, true), expected));
        }
    "#;

    let mut compiler = Compiler::new();
    compiler
        .register_host_function("tick", "env", "tick", vec![], Type::Void)
        .unwrap();

    for level in LEVELS {
        compiler.set_optimization_level(level);
        run_node(&compiler.compile_source(&source).unwrap(), script);
    }
}
