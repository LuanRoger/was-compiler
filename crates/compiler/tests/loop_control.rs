#[path = "../../../tests/support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    semantic::SemanticError,
    typed_ast::{LoopId, TypedStatementKind},
};
use wasmparser::{Operator, Parser, Payload, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const SOURCE: &str = "
    fn main(limit: number) -> number {
        let i = 0;
        let total = 0;
        while i < limit {
            i += 1;
            if i == 3 { continue; }
            if i == 7 { break; }
            total += i;
        }
        return total;
    }
    fn hidden() -> void { while false { return; break; continue; } }
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
fn transfers_preserve_optimization_export_and_owned_import_snapshot_contracts() {
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
        for level in LEVELS {
            compiler.set_optimization_level(level);
            compiler.set_exports(ExportOptions::All);
            let all_exports = compiler.compile(&program).unwrap();
            if level == OptimizationLevel::None {
                assert_eq!(all_exports, baseline);
            }
            compiler.set_exports(exports.clone());
            let expected = compiler.compile(&program).unwrap();
            validate(&expected);
            assert_eq!(
                sections_without_exports(&expected),
                sections_without_exports(&all_exports)
            );
            assert_eq!(compiler.compile(&program).unwrap(), expected);
            assert_eq!(program, original);
        }
    }
}

#[test]
fn source_transfers_decode_expected_depths_without_adding_memory_or_helpers() {
    let mut compiler = Compiler::new();
    let expected = compiler.compile_source(SOURCE).unwrap();

    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(SOURCE).unwrap();
        validate(&bytes);
        if level == OptimizationLevel::None {
            assert_eq!(bytes, expected);
        }
        assert_eq!(compiler.compile_source(SOURCE).unwrap(), bytes);
        let program = compiler.analyze(SOURCE).unwrap();
        assert_eq!(compiler.compile(&program).unwrap(), bytes);

        let mut bodies = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            let payload = payload.unwrap();
            if let Some((id, _)) = payload.as_section() {
                assert!(![0, 5, 6, 11, 12].contains(&id));
            }
            if let Payload::CodeSectionEntry(body) = payload {
                assert_eq!(
                    body.get_locals_reader().unwrap().get_count(),
                    if bodies.is_empty() { 2 } else { 0 }
                );
                let branches = body
                    .get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .filter_map(|op| match op.unwrap() {
                        Operator::Br { relative_depth } => Some((false, relative_depth)),
                        Operator::BrIf { relative_depth } => Some((true, relative_depth)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                bodies.push(branches);
            }
        }
        assert_eq!(
            bodies,
            [
                vec![(true, 1), (false, 1), (false, 2), (false, 0)],
                vec![(true, 1), (false, 1), (false, 0), (false, 0)]
            ]
        );
    }
}

#[test]
fn outside_loop_errors_keep_transfer_spans_even_when_source_cannot_execute() {
    for keyword in ["break", "continue"] {
        for body in [
            format!("{keyword};"),
            format!("return; {keyword};"),
            format!("if false {{ {keyword}; }}"),
            format!("while false {{ break; }} {keyword};"),
        ] {
            let source = format!("fn hidden() -> void {{ {body} }} fn main() -> void {{}}");
            let start = source.rfind(keyword).unwrap();
            let span = was_compiler::tokens::Span::new(start, start + keyword.len() + 1);
            let mut compiler = Compiler::new();
            compiler.set_exports(ExportOptions::Only(vec!["main".into()]));

            for level in LEVELS {
                compiler.set_optimization_level(level);
                let error = compiler.compile_source(&source).unwrap_err();
                let expected = if keyword == "break" {
                    SemanticError::BreakOutsideLoop { span }
                } else {
                    SemanticError::ContinueOutsideLoop { span }
                };
                assert_eq!(error.span(), Some(span));
                assert!(matches!(error, CompileError::Semantic(actual) if actual == expected));
            }
        }
    }
}

#[test]
fn transfer_syntax_and_while_still_requires_fallback_returns() {
    for body in [
        "while true { continue 1; }",
        "while true { continue outer; }",
        "while true { break }",
        "while true { let n = continue; }",
    ] {
        let source = format!("fn main() -> void {{ {body} }}");
        assert!(matches!(
            Compiler::new().compile_source(&source),
            Err(CompileError::Parse(_))
        ));
    }

    for body in ["while true { break 1; }", "while true { break outer; }"] {
        let source = format!("fn main() -> void {{ {body} }}");
        assert!(matches!(
            Compiler::new().compile_source(&source),
            Err(CompileError::Semantic(
                SemanticError::BreakValueNotAllowed { .. }
            ))
        ));
    }

    for body in [
        "while true { break; return 1; }",
        "while true { continue; return 1; }",
        "while true { if true { break; } else { return 1; } }",
        "while true { return 1; }",
    ] {
        let source = format!("fn main() -> number {{ {body} }}");
        assert!(matches!(
            Compiler::new().compile_source(&source),
            Err(CompileError::Semantic(SemanticError::MissingReturn { .. }))
        ));
        let source = format!("fn main() -> number {{ {body} return 2; }}");
        validate(&Compiler::new().compile_source(&source).unwrap());
    }
}

#[test]
fn public_typed_transfer_targets_are_checked_in_unexported_and_skipped_code() {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let mut program = compiler
        .analyze("fn hidden() -> void { while false { if false { break; } continue; } } fn main() -> void {}")
        .unwrap();
    let TypedStatementKind::While { body, .. } = &mut program.functions[0].body[0].kind else {
        panic!("expected while");
    };
    let TypedStatementKind::If { then, .. } = &mut body[0].kind else {
        panic!("expected if");
    };
    let span = then[0].span;
    then[0].kind = TypedStatementKind::Break {
        target: LoopId(usize::MAX),
    };
    let snapshot = program.clone();

    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::InvalidLoopTarget {
                function: "hidden".into(),
                target: LoopId(usize::MAX),
                span,
            })
        );
        assert_eq!(program, snapshot);
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_transfers_recheck_conditions_exit_nearest_loop_and_skip_effects_and_traps() {
    let source = format!(
        "{SOURCE}
        fn nested() -> number {{
            let outer = 0;
            let total = 0;
            while poll(outer, 3) {{
                outer += 1;
                let inner = 0;
                while poll(inner, 4) {{
                    inner += 1;
                    if inner == 2 {{ if outer > 0 {{ continue; }} }}
                    if inner == 3 {{ if true {{ break; }} }}
                    mark(outer * 10 + inner);
                    total += 1;
                }}
                mark(outer * 100);
                if outer == 2 {{ continue; }}
                total += 10;
            }}
            return total;
        }}
        fn guards() -> number {{
            let i = 0;
            while poll(i, 3) && (true || dead()) {{
                i += 1;
                continue;
                mark(999);
            }}
            return i;
        }}
        fn recursive(n: number) -> number {{
            while n > 0 {{
                if n == 1 {{ return 1; }}
                return recursive(n - 1) + 1;
                break;
            }}
            return 0;
        }}
        fn items() -> number[] {{
            let a = [0, 0, 0];
            let i = 0;
            while i < 3 {{
                let fresh = [i];
                i += 1;
                if i == 2 {{ continue; }}
                a[i - 1] += fresh[0];
                if i == 3 {{ break; }}
                a[0] += 10;
            }}
            return a;
        }}
        fn skip_traps(a: number[]) -> number {{
            while true {{ break; a[999] = 1; }}
            let i = 0;
            while i < 3 {{ i += 1; continue; a[999] += 1; }}
            return a[0];
        }}"
    );
    let mut compiler = Compiler::new();
    for (name, params, result) in [
        ("poll", vec![Type::Number, Type::Number], Type::Bool),
        ("mark", vec![Type::Number], Type::Void),
        ("dead", vec![], Type::Bool),
    ] {
        compiler
            .register_host_function(name, "env", name, params, result)
            .unwrap();
    }

    let script = r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: {
            poll(i, limit) { events.push('p' + i + ':' + limit); return i < limit ? 1 : 0; },
            mark(n) { events.push(n); },
            dead() { throw new Error('short-circuited call must not execute'); }
        }}).exports;
        assert.equal(e.main(10), 18);
        assert.equal(e.main(0), 0);
        assert.equal(e.nested(), 23);
        assert.deepEqual(events, [
            'p0:3', 'p0:4', 11, 'p1:4', 'p2:4', 100,
            'p1:3', 'p0:4', 21, 'p1:4', 'p2:4', 200,
            'p2:3', 'p0:4', 31, 'p1:4', 'p2:4', 300, 'p3:3'
        ]);
        events.length = 0;
        assert.equal(e.guards(), 3);
        assert.deepEqual(events, ['p0:3', 'p1:3', 'p2:3', 'p3:3']);
        assert.equal(e.recursive(4), 4);
        assert.equal(e.recursive(0), 0);
        const handle = e.items(), view = new DataView(e.memory.buffer);
        assert.equal(view.getUint32(handle + 4, true), 3);
        const data = view.getUint32(handle, true);
        assert.deepEqual([0,1,2].map(i => view.getFloat64(data + i * 8, true)), [10,0,2]);
        assert.equal(e.skip_traps(handle), 10);
    "#;
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(&source).unwrap();
        validate(&bytes);
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "Node.js failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
