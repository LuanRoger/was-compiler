#[path = "support/process.rs"]
mod test_process;

use std::{process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, OptimizationLevel, Type,
    ast::{Expr, StatementKind},
    lexer::Lexer,
    parser::Parser as SourceParser,
    semantic::SemanticError,
    tokens::Span,
    typed_ast::{ExprResult, LoopId, TypedExpr, TypedExprKind, TypedStatementKind},
};
use wasmparser::{BlockType, ExternalKind, Operator, Parser, Payload, ValType, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const SOURCE: &str = r#"
    fn numeric(flag: bool) -> number {
        return loop { if flag { break forward(4); } else { mark(2); break 8 % 3; } };
    }
    fn boolean(flag: bool) -> bool { return loop { break flag; }; }
    fn text() -> string { return { loop { break "λ"; } }; }
    fn items() -> number[] { return loop { let a = [1, 2]; break a; }; }
    fn forward(n: number) -> number { return n + 1; }
    fn hidden() -> void { return; (loop { continue; break true; break 1; }); }
"#;

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "effects", "mark", vec![Type::Number], Type::Void)
        .unwrap();
    compiler
        .register_host_function("unused", "other", "unused", vec![], Type::Bool)
        .unwrap();
    compiler
}

fn validate(bytes: &[u8]) {
    Validator::new().validate_all(bytes).unwrap();
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

fn span(source: &str, text: &str) -> Span {
    let start = source.find(text).unwrap();
    Span::new(start, start + text.len())
}

fn semantic_rejection(source: &str, expected: SemanticError) {
    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::None);
    for level in LEVELS {
        compiler.set_optimization_level(level);
        for error in [
            compiler.analyze(source).unwrap_err(),
            compiler.compile_source(source).unwrap_err(),
        ] {
            assert_eq!(error.span(), expected.span(), "{source}, {level:?}");
            assert!(
                matches!(error, CompileError::Semantic(actual) if actual == expected),
                "{source}, {level:?}"
            );
        }
    }
}

#[test]
fn public_pipeline_preserves_owned_snapshots_exports_and_o0_through_o3_bytes() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let original = program.clone();
    let baseline = compiler.compile(&program).unwrap();
    validate(&baseline);

    for level in LEVELS {
        compiler.set_optimization_level(level);
        for _ in 0..2 {
            assert_eq!(compiler.analyze(SOURCE).unwrap(), original);
            assert_eq!(compiler.compile_source(SOURCE).unwrap(), baseline);
            assert_eq!(compiler.compile(&program).unwrap(), baseline);
            assert_eq!(program, original);
        }
    }

    compiler
        .register_host_function("later", "later", "added", vec![], Type::Void)
        .unwrap();
    assert_ne!(compiler.compile_source(SOURCE).unwrap(), baseline);

    for (selection, names) in [
        (
            ExportOptions::All,
            vec!["numeric", "boolean", "text", "items", "forward", "hidden"],
        ),
        (ExportOptions::None, vec![]),
        (
            ExportOptions::Only(vec!["items".into(), "numeric".into()]),
            vec!["numeric", "items"],
        ),
    ] {
        compiler.set_exports(selection);
        let expected = compiler.compile(&program).unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            validate(&bytes);
            assert_eq!(bytes, expected);
            assert_eq!(non_export_sections(&bytes), non_export_sections(&baseline));
            let mut imports = Vec::new();
            let mut exports = Vec::new();
            for payload in Parser::new(0).parse_all(&bytes) {
                match payload.unwrap() {
                    Payload::ImportSection(section) => {
                        for import in section.into_imports() {
                            let import = import.unwrap();
                            imports.push((import.module.to_owned(), import.name.to_owned()));
                        }
                    }
                    Payload::ExportSection(section) => {
                        for export in section {
                            let export = export.unwrap();
                            exports.push((export.name.to_owned(), export.kind, export.index));
                        }
                    }
                    _ => {}
                }
            }
            assert_eq!(
                imports,
                [
                    ("effects".into(), "mark".into()),
                    ("other".into(), "unused".into())
                ]
            );
            let user_exports = exports
                .iter()
                .filter(|(name, _, _)| program.functions.iter().any(|f| f.name == *name))
                .collect::<Vec<_>>();
            assert_eq!(
                user_exports
                    .iter()
                    .map(|(name, _, _)| name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            for (name, kind, index) in user_exports {
                let source_index = program
                    .functions
                    .iter()
                    .position(|f| f.name == *name)
                    .unwrap();
                assert_eq!(*kind, ExternalKind::Func);
                assert_eq!(*index, u32::try_from(source_index + 2).unwrap());
            }
            assert!(exports.contains(&("memory".into(), ExternalKind::Memory, 0)));
            assert!(exports.contains(&("__alloc".into(), ExternalKind::Func, 8)));
            assert_eq!(program, original);
        }
    }
}

#[test]
fn all_four_source_results_decode_mvp_value_exits_and_empty_backedges() {
    for (name, ty, literal, raw) in [
        ("number", Type::Number, "1", ValType::F64),
        ("bool", Type::Bool, "true", ValType::I32),
        ("string", Type::String, "\"λ\"", ValType::I32),
        ("number[]", Type::NumberArray, "[1]", ValType::I32),
    ] {
        let source = format!(
            "fn main(flag: bool) -> {name} {{ let value = loop {{ if flag {{ break {literal}; }} break {literal}; }}; return value; }}"
        );
        let mut compiler = Compiler::new();
        let program = compiler.analyze(&source).unwrap();
        let TypedStatementKind::Let {
            ty: storage, value, ..
        } = &program.functions[0].body[0].kind
        else {
            panic!("expected local");
        };
        assert_eq!(*storage, ty);
        assert_eq!(value.ty, ExprResult::Value(ty));
        let TypedExprKind::Loop { id, body } = &value.kind else {
            panic!("expected value loop");
        };
        assert!(
            matches!(&body[1].kind, TypedStatementKind::BreakValue { target, value } if target == id && value.ty == ty)
        );
        let baseline = compiler.compile(&program).unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile_source(&source).unwrap();
            validate(&bytes);
            assert_eq!(bytes, baseline);
            let body = Parser::new(0)
                .parse_all(&bytes)
                .find_map(|payload| match payload.unwrap() {
                    Payload::CodeSectionEntry(body) => Some(body),
                    _ => None,
                })
                .unwrap();
            let mut ops = body.get_operators_reader().unwrap();
            assert!(
                matches!(ops.read().unwrap(), Operator::Block { blockty } if blockty == BlockType::Type(raw))
            );
            assert!(matches!(
                ops.read().unwrap(),
                Operator::Loop {
                    blockty: BlockType::Empty
                }
            ));
            assert!(
                ops.into_iter()
                    .any(|op| matches!(op.unwrap(), Operator::Br { relative_depth: 1 }))
            );
        }
    }
}

#[test]
fn source_contexts_preserve_raw_breaks_statement_loops_and_value_block_tails() {
    let source = "fn main() -> number { loop { break; } loop { break; }; (loop { break 1; }); return { loop { break; } loop { break; }; loop { break 2; } }; }";
    let tokens = Lexer::new(source).tokenize();
    let raw = SourceParser::new(&tokens).parse_program().unwrap();
    let body = &raw.functions[0].body;
    for statement in &body[..2] {
        let StatementKind::Loop { body } = &statement.kind else {
            panic!("expected statement loop");
        };
        assert_eq!(body[0].kind, StatementKind::Break);
    }
    assert!(
        matches!(&body[2].kind, StatementKind::Expression(Expr::Loop { body }) if matches!(body[0].kind, StatementKind::BreakValue(Expr::Number(_))))
    );
    let StatementKind::Return(Some(Expr::Block(block))) = &body[3].kind else {
        panic!("expected returned block");
    };
    assert_eq!(block.statements.len(), 2);
    assert!(
        block
            .statements
            .iter()
            .all(|s| matches!(s.kind, StatementKind::Loop { .. }))
    );
    assert!(
        matches!(block.tail.as_deref(), Some(Expr::Loop { body }) if matches!(body[0].kind, StatementKind::BreakValue(_)))
    );
    validate(&Compiler::new().compile_source(source).unwrap());

    for source in [
        "fn main() -> void { loop }",
        "fn main() -> void { loop { break 1 } }",
        "fn main() -> void { loop { continue 1; } }",
        "fn main() -> void { (loop { break 1; }) }",
    ] {
        assert!(
            matches!(
                Compiler::new().compile_source(source),
                Err(CompileError::Parse(_))
            ),
            "{source}"
        );
    }
}

#[test]
fn break_kind_void_and_reachable_conflicts_keep_public_stage_and_byte_spans() {
    for (body, text, kind) in [
        ("(loop { continue; break; });", "break;", 0),
        ("let x = { loop { break; } };", "break;", 0),
        ("loop { return; break 1; };", "break 1;", 1),
        ("while false { break 1; }", "break 1;", 1),
        ("(loop { break 1; break {}; });", "break {};", 2),
    ] {
        let source = format!("fn hidden() -> void {{ /* λ */ {body} }} fn main() -> void {{}}");
        let span = span(&source, text);
        let expected = match kind {
            0 => SemanticError::MissingBreakValue { span },
            1 => SemanticError::BreakValueNotAllowed { span },
            _ => SemanticError::InvalidBreakValue { span },
        };
        semantic_rejection(&source, expected);
    }
    let statement = "let x = loop { if false { break \"s\"; } else { break []; } };";
    let source = format!("fn hidden() -> void {{ /* é */ {statement} }} fn main() -> void {{}}");
    semantic_rejection(
        &source,
        SemanticError::TypeMismatch {
            expected: Type::String,
            found: Type::NumberArray,
            span: span(&source, statement),
        },
    );
}

#[test]
fn unreachable_valid_break_mismatches_do_not_join_but_their_children_are_checked() {
    for (name, ty, first, second) in [
        ("number", Type::Number, "1", "true"),
        ("bool", Type::Bool, "true", "\"unused\""),
        ("string", Type::String, "\"s\"", "[2]"),
        ("number[]", Type::NumberArray, "[]", "false"),
    ] {
        let source = format!(
            "fn main() -> {name} {{ let x = loop {{ break {first}; break {second}; }}; return x; }}"
        );
        let mut compiler = Compiler::new();
        let program = compiler.analyze(&source).unwrap();
        assert!(
            matches!(&program.functions[0].body[0].kind, TypedStatementKind::Let { value, .. } if value.ty == ty)
        );
        let baseline = compiler.compile(&program).unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile_source(&source).unwrap();
            validate(&bytes);
            assert_eq!(bytes, baseline);
        }
    }
    let source =
        "fn hidden() -> number { return loop { break 1; break missing; }; } fn main() -> void {}";
    semantic_rejection(
        source,
        SemanticError::UndefinedVariable {
            name: "missing".into(),
            span: span(source, "break missing;"),
        },
    );
}

#[test]
fn nested_loop_result_types_and_targets_are_independent() {
    let source = r#"
        fn main() -> number {
            return loop {
                loop { break; }
                let flag = loop { break true; };
                let text = loop { break "λ"; };
                let items = loop { break [2]; };
                while false { break; }
                break if flag { len(items) } else { (loop { break 3; }) };
            };
        }
    "#;
    let mut compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    let TypedStatementKind::Return(Some(TypedExpr {
        kind: TypedExprKind::Loop { id, body },
        ty,
    })) = &program.functions[0].body[0].kind
    else {
        panic!("expected outer loop");
    };
    assert_eq!(*ty, ExprResult::Value(Type::Number));
    let mut ids = vec![*id];
    for (statement, expected) in
        body[1..4]
            .iter()
            .zip([Type::Bool, Type::String, Type::NumberArray])
    {
        let TypedStatementKind::Let { value, ty, .. } = &statement.kind else {
            panic!("expected nested local");
        };
        assert_eq!(*ty, expected);
        assert_eq!(value.ty, ExprResult::Value(expected));
        let TypedExprKind::Loop { id, body } = &value.kind else {
            panic!("expected nested loop");
        };
        assert!(!ids.contains(id));
        ids.push(*id);
        assert!(
            matches!(body[0].kind, TypedStatementKind::BreakValue { target, .. } if target == *id)
        );
    }
    assert!(
        matches!(body.last().unwrap().kind, TypedStatementKind::BreakValue { target, .. } if target == *id)
    );
    let baseline = compiler.compile(&program).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        validate(&bytes);
        assert_eq!(bytes, baseline);
    }
}

#[test]
fn no_exit_is_never_with_concrete_annotated_storage_and_explicit_divergence_completion() {
    // These modules are compile-only: calling their exports would never terminate.
    for (name, ty) in [
        ("number", Type::Number),
        ("bool", Type::Bool),
        ("string", Type::String),
        ("number[]", Type::NumberArray),
    ] {
        let source = format!(
            "fn main() -> number {{ let x: {name} = loop {{ continue; break true; break 1; }}; x; }}"
        );
        let mut compiler = Compiler::new();
        let program = compiler.analyze(&source).unwrap();
        assert!(
            matches!(&program.functions[0].body[0].kind, TypedStatementKind::Let { ty: storage, value, .. } if *storage == ty && value.ty == ExprResult::Never)
        );
        let baseline = compiler.compile(&program).unwrap();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile_source(&source).unwrap();
            validate(&bytes);
            assert_eq!(bytes, baseline);
            validate(
                &compiler
                    .compile_source(&format!("fn main() -> {name} {{ loop {{}} }}"))
                    .unwrap(),
            );
        }
    }
    for initializer in ["loop {}", "loop { continue; }", "loop { break loop {}; }"] {
        let statement = format!("let x = {initializer};");
        let source = format!("fn main() -> number {{ {statement} }}");
        semantic_rejection(
            &source,
            SemanticError::CannotInferNeverType {
                name: "x".into(),
                span: span(&source, &statement),
            },
        );
    }
    for body in [
        "while true {}",
        "while true { loop {} }",
        "loop { break; }",
        "(loop { break 1; });",
    ] {
        let source = format!("fn main() -> number {{ {body} }}");
        let tokens = Lexer::new(&source).tokenize();
        let raw = SourceParser::new(&tokens).parse_program().unwrap();
        semantic_rejection(
            &source,
            SemanticError::MissingReturn {
                name: "main".into(),
                span: raw.functions[0].span,
            },
        );
    }
}

#[test]
fn forged_loop_target_kind_and_result_are_checked_in_hidden_and_unreachable_bodies() {
    for prefix in ["", "return;"] {
        let mut compiler = Compiler::new();
        let source = format!(
            "fn hidden() -> void {{ {prefix} (loop {{ break true; }}); }} fn main() -> void {{}}"
        );
        let original = compiler.analyze(&source).unwrap();
        for corruption in 0..4 {
            let mut program = original.clone();
            let statement = program.functions[0].body.last_mut().unwrap();
            let loop_span = statement.span;
            let TypedStatementKind::Expression(value) = &mut statement.kind else {
                panic!("expected loop expression");
            };
            let TypedExprKind::Loop { id, body } = &mut value.kind else {
                panic!("expected value loop");
            };
            let break_span = body[0].span;
            let expected = match corruption {
                0 => {
                    let TypedStatementKind::BreakValue { target, .. } = &mut body[0].kind else {
                        panic!("expected valued break");
                    };
                    *target = LoopId(usize::MAX);
                    CodegenError::InvalidLoopTarget {
                        function: "hidden".into(),
                        target: *target,
                        span: break_span,
                    }
                }
                1 => {
                    body[0].kind = TypedStatementKind::Break { target: *id };
                    CodegenError::MissingBreakValue {
                        function: "hidden".into(),
                        span: break_span,
                    }
                }
                2 => {
                    value.ty = ExprResult::Never;
                    CodegenError::ExprResultMismatch {
                        function: "hidden".into(),
                        expected: Type::Bool.into(),
                        found: ExprResult::Never,
                        span: loop_span,
                    }
                }
                _ => {
                    value.ty = Type::String.into();
                    CodegenError::TypeMismatch {
                        function: "hidden".into(),
                        expected: Type::Bool,
                        found: Type::String,
                        span: loop_span,
                    }
                }
            };
            let snapshot = program.clone();
            for exports in [
                ExportOptions::None,
                ExportOptions::Only(vec!["main".into()]),
            ] {
                compiler.set_exports(exports);
                for level in LEVELS {
                    compiler.set_optimization_level(level);
                    assert_eq!(compiler.compile(&program).unwrap_err(), expected);
                    assert_eq!(program, snapshot);
                }
            }
        }

        let source =
            format!("fn hidden() -> void {{ {prefix} loop {{ break; }} }} fn main() -> void {{}}");
        let mut program = compiler.analyze(&source).unwrap();
        let TypedStatementKind::Loop { id, body } =
            &mut program.functions[0].body.last_mut().unwrap().kind
        else {
            panic!("expected statement loop");
        };
        let span = body[0].span;
        body[0].kind = TypedStatementKind::BreakValue {
            target: *id,
            value: TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty: Type::Bool.into(),
            },
        };
        let snapshot = program.clone();
        for level in LEVELS {
            compiler.set_optimization_level(level);
            assert_eq!(
                compiler.compile(&program),
                Err(CodegenError::BreakValueNotAllowed {
                    function: "hidden".into(),
                    span
                })
            );
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn legacy_empty_void_signature_and_statement_if_bytes_remain_unchanged() {
    // Independent MVP bytes: () -> (), no imports, one empty exported body.
    const EMPTY_VOID: &[u8] = &[
        0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 2, 1, 0, 3, 2, 1, 0, 7, 8, 1, 4, 109, 97,
        105, 110, 0, 0, 10, 4, 1, 2, 0, 11,
    ];
    validate(EMPTY_VOID);
    let mut compiler = Compiler::new();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        assert_eq!(
            compiler.compile_source("fn main() -> void {}").unwrap(),
            EMPTY_VOID
        );
    }
    compiler
        .register_host_function("marker", "env", "marker", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function("signal", "env", "signal", vec![Type::Bool], Type::Bool)
        .unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler
            .compile_source(include_str!("fixtures/emitter/control.was"))
            .unwrap();
        validate(&bytes);
        assert_eq!(bytes, include_bytes!("fixtures/emitter/control.wasm"));
    }
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_value_loops_preserve_results_aliases_effects_and_abrupt_stack_prefixes() {
    let source = r#"
        fn numeric(flag: bool) -> number {
            let n = 0;
            return loop {
                n += 1;
                mark(n);
                if n < 3 { continue; }
                break if flag { -0 } else { 3.5 };
            };
        }
        fn boolean(flag: bool) -> bool {
            return { loop { break if flag { true } else { false }; } };
        }
        fn text(flag: bool) -> string {
            return loop { break if flag { "λ" } else { "世界" }; };
        }
        fn items() -> number[] {
            let a = [1, 2];
            let alias = loop { break a; };
            alias[0] = 7;
            return loop { break a; };
        }
        fn identity(a: number[]) -> number[] { return loop { break a; }; }
        fn prefixes() -> number {
            return sum(touch(10), loop {
                break touch(20) + { break touch(3); };
                mark(999);
            });
        }
        fn elements() -> number[] { return [touch(30), loop { break touch(4); }]; }
        fn places() -> number[] {
            let original = [10, 20];
            let target = original;
            let index = 0;
            let n = 0;
            target[touch(index)] += loop {
                n += 1;
                mark(n);
                if n == 1 { continue; }
                original[0] = 50;
                target = [99, 88];
                index = 1;
                break if true { 2 } else { 9 };
            };
            return [original[0], original[1], target[0], index];
        }
        fn transfers(a: number[], early: bool) -> number {
            let n = 0;
            return touch(100) + loop {
                n += 1;
                a[0] += touch(20) + if n == 1 { continue; } else {
                    if early { return touch(9); } else { break touch(6); }
                };
                mark(999);
            };
        }
        fn break_rhs() -> number {
            return loop { break touch(40) + { break touch(8); }; };
        }
        fn return_rhs() -> number {
            return touch(50) + loop { break touch(60) + { return touch(7); }; };
        }
        fn scalar() -> number {
            let x = 4;
            x += loop { x = 100; break 3; };
            return x;
        }
        fn checked(a: number[]) -> number {
            a[touch(999)] += loop { mark(998); break 1; };
            return 0;
        }
        fn bare() -> number {
            let n = 0;
            loop { n += 1; if n < 3 { continue; } break; };
            return n;
        }
    "#;
    let script = r#"
        const assert = require('node:assert/strict');
        const events = [];
        const e = new WebAssembly.Instance(new WebAssembly.Module(require('node:fs').readFileSync(0)), {
            effects: { mark(n) { events.push(n); } },
            other: { unused() { return 0; } },
            env: {
                touch(n) { events.push(n); return n; },
                sum(a, b) { events.push('sum'); return a + b; }
            }
        }).exports;
        function check(expected) { assert.deepEqual(events, expected); events.length = 0; }
        function descriptor(h) {
            const view = new DataView(e.memory.buffer);
            return [view.getUint32(h, true), view.getUint32(h + 4, true)];
        }
        function array(h) {
            const [p, n] = descriptor(h);
            return Array.from({length:n}, (_, i) => new DataView(e.memory.buffer).getFloat64(p + i * 8, true));
        }
        function text(h) {
            const [p, n] = descriptor(h);
            return new TextDecoder().decode(new Uint8Array(e.memory.buffer, p, n));
        }
        assert.ok(Object.is(e.numeric(1), -0)); check([1,2,3]);
        assert.equal(e.numeric(0), 3.5); check([1,2,3]);
        assert.equal(e.boolean(1), 1); assert.equal(e.boolean(0), 0);
        assert.equal(text(e.text(1)), 'λ'); assert.equal(text(e.text(0)), '世界');
        const a = e.items(); assert.deepEqual(array(a), [7,2]);
        assert.equal(e.identity(a), a);
        const [p] = descriptor(e.identity(a));
        new DataView(e.memory.buffer).setFloat64(p, 11, true);
        assert.deepEqual(array(a), [11,2]);
        assert.notEqual(e.items(), a);
        assert.equal(e.prefixes(), 13); check([10,20,3,'sum']);
        assert.deepEqual(array(e.elements()), [30,4]); check([30,4]);
        assert.deepEqual(array(e.places()), [12,20,99,1]); check([0,1,2]);
        assert.equal(e.transfers(a, 0), 106); check([100,20,20,6]);
        assert.deepEqual(array(a), [11,2]);
        assert.equal(e.transfers(a, 1), 9); check([100,20,20,9]);
        assert.deepEqual(array(a), [11,2]);
        assert.equal(e.break_rhs(), 8); check([40,8]);
        assert.equal(e.return_rhs(), 7); check([50,60,7]);
        assert.equal(e.scalar(), 7);
        assert.throws(() => e.checked(a), WebAssembly.RuntimeError); check([999]);
        assert.equal(e.bare(), 3);
    "#;
    let mut compiler = compiler();
    compiler
        .register_host_function("touch", "env", "touch", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
        .register_host_function(
            "sum",
            "env",
            "sum",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .unwrap();
    let baseline = compiler.compile_source(source).unwrap();
    for level in LEVELS {
        compiler.set_optimization_level(level);
        let bytes = compiler.compile_source(source).unwrap();
        validate(&bytes);
        assert_eq!(bytes, baseline);
        let output = test_process::output(
            Command::new("node").arg("-e").arg(script),
            Some(&bytes),
            Duration::from_secs(30),
        );
        assert!(
            output.status.success(),
            "Node.js failed at {level:?}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
