use was_compiler::{Compiler, CompilerOptions, OptimizationLevel, codegen};
use wasmparser::{Parser, Payload, ValType, Validator};

#[path = "../../../tests/support/process.rs"]
mod test_process;

const ENABLED: [OptimizationLevel; 3] = [
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn compiler(level: OptimizationLevel) -> Compiler {
    Compiler::with_options(CompilerOptions {
        optimization_level: level,
    })
}

fn instructions(bytes: &[u8]) -> Vec<String> {
    Validator::new().validate_all(bytes).unwrap();
    let mut result = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            for op in body.get_operators_reader().unwrap() {
                result.push(format!("{:?}", op.unwrap()));
            }
        }
    }
    result
}

#[test]
fn proven_subtrees_use_integer_operations_without_folding_or_changing_the_abi() {
    let source = "fn main() -> number { return (10 + 20) - 3; }";
    let baseline = Compiler::new().compile_source(source).unwrap();
    let program = Compiler::new().analyze(source).unwrap();
    assert_eq!(baseline, codegen::compile(&program).unwrap());
    assert!(instructions(&baseline).contains(&"F64Add".into()));

    for level in ENABLED {
        let selected = compiler(level);
        let bytes = selected.compile(&program).unwrap();
        assert_eq!(bytes, selected.compile_source(source).unwrap());
        assert_ne!(bytes, baseline);
        assert_eq!(
            instructions(&bytes),
            [
                "I32Const { value: 10 }",
                "I32Const { value: 20 }",
                "I32Add",
                "I32Const { value: 3 }",
                "I32Sub",
                "F64ConvertI32S",
                "Return",
                "End",
            ]
        );
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::TypeSection(types) => {
                    let types: Vec<_> = types
                        .into_iter_err_on_gc_types()
                        .map(Result::unwrap)
                        .collect();
                    assert_eq!(types.len(), 1);
                    assert!(types[0].params().is_empty());
                    assert_eq!(types[0].results(), &[ValType::F64]);
                }
                Payload::CodeSectionEntry(body) => {
                    assert_eq!(body.get_locals_reader().unwrap().get_count(), 0)
                }
                _ => {}
            }
        }
    }
}

#[test]
fn parsed_values_and_every_intermediate_bound_control_specialization() {
    // Counts are independent expected opcodes, not values computed by the proof.
    for (expr, integer_adds, integer_subs, float_adds, float_subs) in [
        ("2147483647 + 0", 1, 0, 0, 0),
        ("0 - 2147483647 - 1", 0, 2, 0, 0),
        ("(0 - 2147483647 - 1) - (0 - 1)", 0, 4, 0, 0),
        ("(2147483647 + 1) - 1", 0, 0, 1, 1),
        ("(0 - 2147483647 - 1) - 1", 0, 2, 0, 1),
        ("2147483648 - 1", 0, 0, 0, 1),
        ("1.00000000000000001 + 2.000", 1, 0, 0, 0),
        ("2147483647.00000001 + 0", 1, 0, 0, 0),
        ("2147483647.99999999 - 1", 0, 0, 0, 1),
        ("0 - 0", 0, 1, 0, 0),
        ("-0 + 0", 0, 0, 1, 0),
        ("0 - -0", 0, 0, 0, 1),
        ("-0 - 0", 0, 0, 0, 1),
        ("1.5 + (2 + 3)", 1, 0, 1, 0),
        ("(2 + 3) * (4 - 1)", 1, 1, 0, 0),
        ("-(2 + 3)", 1, 0, 0, 0),
    ] {
        let source = format!("fn main() -> number {{ return {expr}; }}");
        let expected = compiler(OptimizationLevel::Basic)
            .compile_source(&source)
            .unwrap();
        for level in ENABLED {
            let bytes = compiler(level).compile_source(&source).unwrap();
            assert_eq!(bytes, expected, "enabled preset identity: {expr}");
            let ops = instructions(&bytes);
            for (op, count) in [
                ("I32Add", integer_adds),
                ("I32Sub", integer_subs),
                ("F64Add", float_adds),
                ("F64Sub", float_subs),
            ] {
                assert_eq!(
                    ops.iter().filter(|actual| actual.as_str() == op).count(),
                    count,
                    "{level:?}: {expr}: {op}"
                );
            }
        }
    }
}

#[test]
fn unknown_inputs_stay_floating_and_local_storage_and_consumers_remain_unchanged() {
    for source in [
        "fn main(x: number) -> number { return x; }",
        "fn main(x: number) -> number { return x * 1.5; }",
        "fn main() -> number { return 2147483648; }",
        "fn main() -> number { return 1.5; }",
    ] {
        let baseline = Compiler::new().compile_source(source).unwrap();
        for level in ENABLED {
            assert_eq!(compiler(level).compile_source(source).unwrap(), baseline);
        }
    }

    for (source, required) in [
        (
            "fn main() -> number { let x = 1 + 2; return x + 3; }",
            "F64Add",
        ),
        (
            "fn main() -> number { return (1 + 2) * (3 + 4); }",
            "F64Mul",
        ),
        (
            "fn main() -> number { return (1 + 2) / (3 + 4); }",
            "F64Div",
        ),
        ("fn main() -> bool { return (1 + 2) < (3 + 4); }", "F64Lt"),
        ("fn main() -> number { return -(1 + 2); }", "F64Neg"),
        (
            "fn main() -> number { let a = [1 + 2]; return a[0] + 3; }",
            "F64Add",
        ),
        (
            "fn main() -> number { let x = 1 + 2; let read = () => x + 3; return read(); }",
            "F64Add",
        ),
        (
            "fn main() -> number { return get() + (1 + 2); } fn get() -> number { return 4; }",
            "F64Add",
        ),
    ] {
        for level in ENABLED {
            let bytes = compiler(level).compile_source(source).unwrap();
            let ops = instructions(&bytes);
            assert!(ops.contains(&"I32Add".into()), "{source}");
            assert!(ops.contains(&required.into()), "{source}: {required}");
            for payload in Parser::new(0).parse_all(&bytes) {
                if let Payload::CodeSectionEntry(body) = payload.unwrap() {
                    // Ordinary numeric locals remain f64. Handle/scratch/capture
                    // locals may still use their existing i32 types.
                    if source.contains("let x =") && !source.contains("let read") {
                        let locals: Vec<_> = body
                            .get_locals_reader()
                            .unwrap()
                            .into_iter()
                            .map(Result::unwrap)
                            .collect();
                        assert_eq!(locals, [(1, ValType::F64)]);
                    }
                }
            }
        }
    }
}

#[test]
fn proof_cache_preserves_forged_tree_errors_and_does_not_survive_compilation() {
    use was_compiler::{
        Type,
        typed_ast::{ExprResult, TypedExprKind, TypedStatementKind},
    };

    let baseline = Compiler::new();
    let original = baseline
        .analyze("fn main() -> number { return 1 + 2; }")
        .unwrap();
    for corruption in 0..6 {
        let mut program = original.clone();
        let TypedStatementKind::Return(Some(expr)) = &mut program.functions[0].body[0].kind else {
            panic!()
        };
        let TypedExprKind::Binary { left, right, .. } = &mut expr.kind else {
            panic!()
        };
        match corruption {
            0 => expr.ty = ExprResult::Never,
            1 => left.ty = Type::Bool.into(),
            2 => right.kind = TypedExprKind::Number("-0".into()),
            3 => right.kind = TypedExprKind::Number("9".repeat(400)),
            4 => right.kind = TypedExprKind::Number("1e3".into()),
            5 => right.ty = ExprResult::Never,
            _ => unreachable!(),
        }
        let error = baseline.compile(&program).unwrap_err();
        for level in ENABLED {
            assert_eq!(
                compiler(level).compile(&program).unwrap_err(),
                error,
                "corruption {corruption}"
            );
        }
    }
    let mut program = original.clone();
    let selected = compiler(OptimizationLevel::Basic);
    let before = selected.compile(&program).unwrap();
    let TypedStatementKind::Return(Some(expr)) = &mut program.functions[0].body[0].kind else {
        panic!()
    };
    let TypedExprKind::Binary { right, .. } = &mut expr.kind else {
        panic!()
    };
    right.kind = TypedExprKind::Number("2147483648".into());
    let after = selected.compile(&program).unwrap();
    assert_ne!(before, after);
    assert!(instructions(&after).contains(&"F64Add".into()));

    for source in [
        "fn main() -> number { return 1; return 2 + 3; }",
        "fn main() -> bool { return false && (1 + 2 == 3); }",
        "fn main() -> number { return 1; } fn unused() -> number { return 2 + 3; }",
    ] {
        let mut program = baseline.analyze(source).unwrap();
        let target = if program.functions.len() > 1 {
            &mut program.functions[1].body[0]
        } else {
            program.functions[0].body.last_mut().unwrap()
        };
        let TypedStatementKind::Return(Some(expr)) = &mut target.kind else {
            panic!()
        };
        // Corrupt a child behind the optimized candidate / skipped RHS.
        let TypedExprKind::Binary { right, .. } = &mut expr.kind else {
            panic!()
        };
        right.ty = Type::String.into();
        let error = baseline.compile(&program).unwrap_err();
        for level in ENABLED {
            assert_eq!(compiler(level).compile(&program).unwrap_err(), error);
        }
    }
}

#[test]
#[ignore = "requires Node.js 24+ on PATH"]
fn bounded_numeric_edges_execute_against_independent_expected_results() {
    // Expected JS literals are worked outcomes, never the production algorithm.
    let cases = [
        ("2147483647 + 0", "2147483647"),
        ("0 - 2147483647 - 1", "-2147483648"),
        ("(2147483647 + 1) - 1", "2147483647"),
        ("((0 - 2147483647) - 1) - 1", "-2147483649"),
        ("(0 - 2147483647 - 1) - (0 - 1)", "-2147483647"),
        ("1.00000000000000001 + 2.000", "3"),
        ("2147483647.00000001 + 0", "2147483647"),
        ("2147483647.99999999 - 1", "2147483647"),
        ("0 - 0", "0"),
        ("-0 + 0", "0"),
        ("0 - -0", "0"),
        ("-0 - 0", "-0"),
        ("-0 + -0", "-0"),
        ("0 / (0 - 0)", "NaN"),
        ("1 / (0 - 0)", "Infinity"),
        ("1 / (-0 - 0)", "-Infinity"),
        ("(2 + 3) * (4 - 1)", "15"),
        ("(7 + 2) / (4 - 1)", "3"),
        ("(7 + 2) % (4 - 1)", "0"),
        ("-(2 + 3)", "-5"),
    ];
    let mut records = Vec::new();
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        for (expr, expected) in cases {
            let source = format!("fn main() -> number {{ return {expr}; }}");
            let bytes = compiler(level).compile_source(&source).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            records.push(format!("[{bytes:?}, {expected:?}]"));
        }
    }
    let input = format!("[{}]", records.join(","));
    let script = r#"
        const assert = require('node:assert/strict');
        const fs = require('node:fs');
        const cases = JSON.parse(fs.readFileSync(0, 'utf8'));
        for (const [bytes, literal] of cases) {
            const expected = Number(literal);
            const instance = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array(bytes)));
            const actual = instance.exports.main();
            assert.ok(Object.is(actual, expected), `${actual} !== ${expected}`);
        }
    "#;
    let output = test_process::output(
        std::process::Command::new("node").args(["-e", script]),
        Some(input.as_bytes()),
        std::time::Duration::from_secs(30),
    );
    assert!(output.status.success(), "{output:?}");
}
