use was_compiler::{
    CodegenError, Compiler, ExportOptions, OptimizationLevel, Type, codegen,
    typed_ast::{
        CallBindingOrigin, TypedExpr, TypedExprKind, TypedNormalizedCall, TypedProgram,
        TypedStatementKind,
    },
};
use wasmparser::{Operator, Parser, Payload, ValType, Validator};

fn inject(value: &mut TypedExpr) {
    let sequence = normalized(value);
    sequence.bindings[0].origin = CallBindingOrigin::PipelineInput;
    for binding in &mut sequence.bindings[1..] {
        if let CallBindingOrigin::Explicit { argument } = &mut binding.origin {
            *argument -= 1;
        }
    }
}

fn assert_invalid(program: &TypedProgram, reason: &'static str, span: was_compiler::tokens::Span) {
    let expected = CodegenError::InvalidNormalizedCall { reason, span };
    assert_eq!(codegen::compile(program).unwrap_err(), expected);
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        compiler.set_exports(ExportOptions::None);
        assert_eq!(compiler.compile(program).unwrap_err(), expected);
    }
}

fn nested_fixture() -> TypedProgram {
    let mut program = Compiler::new()
        .analyze(
            "fn target(a: number, b: number, c: number, d: number = a + b) -> number { return d; }
         fn main() -> number { return target(target(5, c: 7, b: 6), c: 33, b: 22); }",
        )
        .unwrap();
    let value = returned(&mut program);
    inject(value);
    inject(&mut normalized(value).bindings[0].value);
    program
}

fn returned(program: &mut TypedProgram) -> &mut TypedExpr {
    let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[0].kind else {
        panic!("return")
    };
    value
}

fn normalized(value: &mut TypedExpr) -> &mut TypedNormalizedCall {
    let TypedExprKind::NormalizedCall(sequence) = &mut value.kind else {
        panic!("normalized call")
    };
    sequence
}

fn fixture() -> TypedProgram {
    let mut program = Compiler::new()
        .analyze(
            "fn target(a: number, b: number, c: number, d: number = a + b) -> number { return d; }
             fn main() -> number { return target(11, c: 33, b: 22); }",
        )
        .unwrap();
    let sequence = normalized(returned(&mut program));
    sequence.bindings[0].origin = CallBindingOrigin::PipelineInput;
    sequence.bindings[1].origin = CallBindingOrigin::Explicit { argument: 0 };
    sequence.bindings[2].origin = CallBindingOrigin::Explicit { argument: 1 };
    program
}

fn multiple_default_fixture(call: &str) -> TypedProgram {
    let mut program = Compiler::new()
        .analyze(&format!(
            "fn target(a: number = 7, b: number = a + 1,
                c: number = b + 2, d: number = c + 3,
                e: number = d + 4, f: number = e + 5) -> number {{ return f; }}
             fn main() -> number {{ return {call}; }}"
        ))
        .unwrap();
    inject(returned(&mut program));
    program
}

#[test]
fn named_mixed_and_default_pipeline_sources_match_checked_normalization() {
    for (ordinary, pipeline) in [
        ("target(11, e: 55, b: 22)", "11 |> target(e: 55, b: 22)"),
        ("target(11, 22, e: 55)", "11 |> target(22, e: 55)"),
        ("target(11)", "11 |> target"),
        ("target(11)", "11 |> target()"),
    ] {
        let declarations = "fn target(a: number = 7, b: number = a + 1,
            c: number = b + 2, d: number = c + 3,
            e: number = d + 4, f: number = e + 5) -> number { return f; }";
        let mut expected = Compiler::new()
            .analyze(&format!(
                "{declarations} fn main() -> number {{ return {ordinary}; }}"
            ))
            .unwrap();
        inject(returned(&mut expected));
        let program = Compiler::new()
            .analyze(&format!(
                "{declarations} fn main() -> number {{ return {pipeline}; }}"
            ))
            .unwrap();
        let expected_bytes = Compiler::new().compile(&expected).unwrap();
        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let mut compiler = Compiler::new();
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            assert_eq!(bytes, expected_bytes, "{pipeline}, {level:?}");
        }
    }
}

#[test]
fn multiple_pipeline_defaults_store_once_in_formal_order_after_written_arguments() {
    for (call, b_slot, e_slot) in [
        ("target(11, e: 55, b: 22)", 2, 1),
        ("target(11, 22, e: 55)", 1, 2),
    ] {
        let program = multiple_default_fixture(call);
        let snapshot = program.clone();
        let bytes = Compiler::new().compile(&program).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let body = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|payload| match payload.unwrap() {
                Payload::CodeSectionEntry(body) => Some(body),
                Payload::TableSection(_) | Payload::ElementSection(_) => {
                    panic!("no function table")
                }
                _ => None,
            })
            .nth(1)
            .unwrap();
        let locals: Vec<_> = body
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(locals, [(1, ValType::F64); 6]);
        let ops: Vec<_> = body
            .get_operators_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect();
        let written = if b_slot == 2 {
            [55f64, 22f64]
        } else {
            [22f64, 55f64]
        };
        assert!(
            matches!(ops.as_slice(), [
            Operator::F64Const { value: input }, Operator::LocalSet { local_index: 0 },
            Operator::F64Const { value: first }, Operator::LocalSet { local_index: 1 },
            Operator::F64Const { value: second }, Operator::LocalSet { local_index: 2 },
            Operator::LocalGet { local_index: c_read }, Operator::F64Const { value: two },
            Operator::F64Add, Operator::LocalSet { local_index: 3 },
            Operator::LocalGet { local_index: 3 }, Operator::F64Const { value: three },
            Operator::F64Add, Operator::LocalSet { local_index: 4 },
            Operator::LocalGet { local_index: f_read }, Operator::F64Const { value: five },
            Operator::F64Add, Operator::LocalSet { local_index: 5 },
            Operator::LocalGet { local_index: 0 }, Operator::LocalGet { local_index: b_arg },
            Operator::LocalGet { local_index: 3 }, Operator::LocalGet { local_index: 4 },
            Operator::LocalGet { local_index: e_arg }, Operator::LocalGet { local_index: 5 },
            Operator::Call { function_index: 0 }, Operator::Return, Operator::End,
        ] if input.bits() == 11f64.to_bits()
            && first.bits() == written[0].to_bits() && second.bits() == written[1].to_bits()
            && two.bits() == 2f64.to_bits() && three.bits() == 3f64.to_bits() && five.bits() == 5f64.to_bits()
            && *c_read == b_slot && *b_arg == b_slot && *f_read == e_slot && *e_arg == e_slot),
            "{call}: {ops:?}"
        );
        assert_eq!(program, snapshot);
        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let mut compiler = Compiler::new();
            compiler.set_optimization_level(level);
            assert_eq!(compiler.compile(&program).unwrap(), bytes);
        }
    }
}

#[test]
fn omitted_pipeline_defaults_capture_injection_then_each_dependent_default_once() {
    let program = multiple_default_fixture("target(11)");
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let body = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .nth(1)
        .unwrap();
    let ops: Vec<_> = body
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(
        matches!(&ops[..2], [Operator::F64Const { value }, Operator::LocalSet { local_index: 0 }] if value.bits() == 11f64.to_bits())
    );
    for (chunk, (read, store, increment)) in ops[2..22].as_chunks::<4>().0.iter().zip([
        (0, 1, 1f64),
        (1, 2, 2f64),
        (2, 3, 3f64),
        (3, 4, 4f64),
        (4, 5, 5f64),
    ]) {
        assert!(matches!(chunk, [
            Operator::LocalGet { local_index: actual_read }, Operator::F64Const { value },
            Operator::F64Add, Operator::LocalSet { local_index: actual_store },
        ] if *actual_read == read && *actual_store == store && value.bits() == increment.to_bits()));
    }
    assert!(matches!(
        &ops[22..],
        [
            Operator::LocalGet { local_index: 0 },
            Operator::LocalGet { local_index: 1 },
            Operator::LocalGet { local_index: 2 },
            Operator::LocalGet { local_index: 3 },
            Operator::LocalGet { local_index: 4 },
            Operator::LocalGet { local_index: 5 },
            Operator::Call { function_index: 0 },
            Operator::Return,
            Operator::End,
        ]
    ));
}

#[test]
fn pipeline_default_origins_and_named_formal_mappings_remain_checked() {
    for case in 0..7 {
        let mut program = multiple_default_fixture("target(11, e: 55, b: 22)");
        let span = program.functions[1].body[0].span;
        let sequence = normalized(returned(&mut program));
        let (reason, span) = match case {
            0 => {
                sequence.bindings[3].origin = CallBindingOrigin::Default { formal: 4 };
                (
                    "default origin must match its formal mapping",
                    sequence.bindings[3].span,
                )
            }
            1 => {
                sequence.bindings.swap(3, 4);
                (
                    "defaults must follow explicit arguments in increasing formal order",
                    sequence.bindings[4].span,
                )
            }
            2 => {
                sequence.bindings[4].origin = CallBindingOrigin::Explicit { argument: 2 };
                (
                    "bindings must have contiguous written-order explicit origins",
                    sequence.bindings[4].span,
                )
            }
            3 => {
                sequence.bindings[2].origin = CallBindingOrigin::Explicit { argument: 0 };
                (
                    "bindings must have contiguous written-order explicit origins",
                    sequence.bindings[2].span,
                )
            }
            4 => {
                sequence.formal_to_binding[1] = sequence.formal_to_binding[4];
                let TypedExprKind::Call { args, .. } = &mut sequence.call.kind else {
                    panic!("call")
                };
                args[1] = args[4].clone();
                ("formal mapping must be a bijection", span)
            }
            5 => {
                let TypedExprKind::Call { args, .. } = &mut sequence.call.kind else {
                    panic!("call")
                };
                args.swap(1, 4);
                (
                    "final arguments must read owned bindings in formal order",
                    span,
                )
            }
            6 => {
                sequence.bindings[3].origin = CallBindingOrigin::PipelineInput;
                (
                    "pipeline input must be the first and only injected binding",
                    sequence.bindings[3].span,
                )
            }
            _ => unreachable!(),
        };
        assert_invalid(&program, reason, span);
    }
}

#[test]
fn pipeline_defaults_read_only_earlier_formals_not_earlier_storage_slots() {
    for (default, allowed) in [
        (3, &[0, 2][..]),
        (4, &[0, 2, 3][..]),
        (5, &[0, 2, 3, 4, 1][..]),
    ] {
        for owned in 0..7 {
            let mut program = multiple_default_fixture("target(11, e: 55, b: 22)");
            let sequence = normalized(returned(&mut program));
            let id = if owned == 6 {
                was_compiler::typed_ast::VariableId(999)
            } else {
                sequence.bindings[owned].id
            };
            let span = sequence.bindings[default].span;
            sequence.bindings[default].value = TypedExpr {
                kind: TypedExprKind::Identifier {
                    id,
                    name: "forged".into(),
                },
                ty: Type::Number.into(),
            };
            if allowed.contains(&owned) {
                let bytes = Compiler::new().compile(&program).unwrap();
                Validator::new().validate_all(&bytes).unwrap();
            } else {
                assert_invalid(
                    &program,
                    "defaults may only read owned earlier-formal bindings",
                    span,
                );
            }
        }
    }
}

fn multiple_default_chain() -> TypedProgram {
    let mut program = multiple_default_fixture(
        "target(target(target(11, e: 55, b: 22), 22, e: 55), e: 55, b: 22)",
    );
    let middle = &mut normalized(returned(&mut program)).bindings[0].value;
    inject(middle);
    inject(&mut normalized(middle).bindings[0].value);
    program
}

#[test]
fn nested_named_default_stages_have_distinct_once_only_storage_and_direct_calls() {
    let program = multiple_default_chain();
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let body = Parser::new(0)
        .parse_all(&bytes)
        .filter_map(|payload| match payload.unwrap() {
            Payload::CodeSectionEntry(body) => Some(body),
            Payload::TableSection(_) | Payload::ElementSection(_) => panic!("no function table"),
            _ => None,
        })
        .nth(1)
        .unwrap();
    let locals: Vec<_> = body
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(locals, [(1, ValType::F64); 18]);
    let mut sets = Vec::new();
    let mut gets = Vec::new();
    let mut calls = Vec::new();
    let mut constants = Vec::new();
    for op in body.get_operators_reader().unwrap() {
        match op.unwrap() {
            Operator::LocalSet { local_index } => sets.push(local_index),
            Operator::LocalGet { local_index } => gets.push(local_index),
            Operator::Call { function_index } => calls.push(function_index),
            Operator::CallIndirect { .. } => panic!("no indirect call"),
            Operator::F64Const { value } => constants.push(f64::from_bits(value.bits())),
            _ => {}
        }
    }
    assert_eq!(
        sets,
        [2, 3, 4, 5, 6, 7, 1, 8, 9, 10, 11, 12, 0, 13, 14, 15, 16, 17]
    );
    assert_eq!(
        gets,
        [
            4, 5, 3, 2, 4, 5, 6, 3, 7, 8, 10, 9, 1, 8, 10, 11, 9, 12, 14, 15, 13, 0, 14, 15, 16,
            13, 17
        ]
    );
    assert_eq!(calls, [0, 0, 0]);
    assert_eq!(
        constants,
        [
            11.0, 55.0, 22.0, 2.0, 3.0, 5.0, 22.0, 55.0, 2.0, 3.0, 5.0, 55.0, 22.0, 2.0, 3.0, 5.0
        ]
    );
}

#[test]
fn named_pipeline_ownership_checks_include_all_defaults_and_nested_scopes() {
    for initializer in 0..3 {
        for owned in 0..6 {
            let mut program = multiple_default_fixture("target(11, e: 55, b: 22)");
            let sequence = normalized(returned(&mut program));
            sequence.bindings[initializer].value = TypedExpr {
                kind: TypedExprKind::Identifier {
                    id: sequence.bindings[owned].id,
                    name: "owned".into(),
                },
                ty: Type::Number.into(),
            };
            for level in [
                OptimizationLevel::None,
                OptimizationLevel::Basic,
                OptimizationLevel::Standard,
                OptimizationLevel::Aggressive,
            ] {
                let mut compiler = Compiler::new();
                compiler.set_optimization_level(level);
                assert!(
                    matches!(
                        compiler.compile(&program),
                        Err(CodegenError::InvalidNormalizedCall { .. })
                    ),
                    "initializer {initializer}, owned {owned}"
                );
            }
        }
    }
    for escape in [false, true] {
        let mut program = multiple_default_chain();
        let outer = normalized(returned(&mut program));
        let outer_id = outer.bindings[0].id;
        let middle = normalized(&mut outer.bindings[0].value);
        let inner = normalized(&mut middle.bindings[0].value);
        let inner_id = inner.bindings[0].id;
        let binding = if escape {
            &mut outer.bindings[3]
        } else {
            &mut inner.bindings[3]
        };
        let span = binding.span;
        binding.value = TypedExpr {
            kind: TypedExprKind::Identifier {
                id: if escape { inner_id } else { outer_id },
                name: "cross_scope".into(),
            },
            ty: Type::Number.into(),
        };
        assert_invalid(
            &program,
            "defaults may only read owned earlier-formal bindings",
            span,
        );
    }
}

#[test]
fn later_named_stage_defaults_and_children_are_checked_after_never() {
    use was_compiler::typed_ast::{ExprResult, LoopId};

    for divergent in [0, 1] {
        for case in 0..4 {
            let mut program = multiple_default_chain();
            let value = returned(&mut program);
            value.ty = ExprResult::Never;
            let outer = normalized(value);
            outer.bindings[0].value.ty = ExprResult::Never;
            let middle = normalized(&mut outer.bindings[0].value);
            middle.bindings[0].value.ty = ExprResult::Never;
            let inner = normalized(&mut middle.bindings[0].value);
            inner.bindings[divergent].value = TypedExpr {
                kind: TypedExprKind::Loop {
                    id: LoopId(77),
                    body: vec![],
                },
                ty: ExprResult::Never,
            };
            let bytes = Compiler::new().compile(&program).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            let outer = normalized(returned(&mut program));
            let binding = if case == 3 { 1 } else { 3 };
            let span = outer.bindings[binding].span;
            match case {
                0 | 3 => {
                    outer.bindings[binding].value.kind = TypedExprKind::Number("invalid".into());
                    for level in [
                        OptimizationLevel::None,
                        OptimizationLevel::Basic,
                        OptimizationLevel::Standard,
                        OptimizationLevel::Aggressive,
                    ] {
                        let mut compiler = Compiler::new();
                        compiler.set_optimization_level(level);
                        compiler.set_exports(ExportOptions::None);
                        assert!(
                            matches!(compiler.compile(&program), Err(CodegenError::InvalidNumberLiteral { span: actual, .. }) if actual == span)
                        );
                    }
                }
                1 => {
                    outer.bindings[3].value = TypedExpr {
                        kind: TypedExprKind::Identifier {
                            id: outer.bindings[1].id,
                            name: "later_formal".into(),
                        },
                        ty: Type::Number.into(),
                    };
                    assert_invalid(
                        &program,
                        "defaults may only read owned earlier-formal bindings",
                        span,
                    );
                }
                2 => {
                    outer.bindings[3].origin = CallBindingOrigin::Default { formal: 4 };
                    assert_invalid(
                        &program,
                        "default origin must match its formal mapping",
                        span,
                    );
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn pipeline_helpers_follow_users_memory_remainder_and_range_order() {
    use wasmparser::ExternalKind;

    let mut compiler = Compiler::new();
    compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
    let program = compiler
        .analyze(
            "fn make(n: number) -> range { return 0..n; }
         fn inspect(r: range) -> number {
             let values = [5, 2];
             if (1 in r) { return values[0] % values[1]; }
             return len(values);
         }
         fn main() -> number { return 4 |> make |> inspect; }",
        )
        .unwrap();
    let bytes = compiler.compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();

    let mut signatures = Vec::new();
    let mut functions = Vec::new();
    let mut exports = Vec::new();
    let mut calls = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::TypeSection(reader) => {
                for signature in reader.into_iter_err_on_gc_types() {
                    let signature = signature.unwrap();
                    signatures.push((signature.params().to_vec(), signature.results().to_vec()));
                }
            }
            Payload::FunctionSection(reader) => {
                functions.extend(reader.into_iter().map(Result::unwrap))
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.unwrap();
                    exports.push((export.name, export.kind, export.index));
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut direct = Vec::new();
                for op in body.get_operators_reader().unwrap() {
                    match op.unwrap() {
                        Operator::Call { function_index } => direct.push(function_index),
                        Operator::CallIndirect { .. } => panic!("no indirect call"),
                        _ => {}
                    }
                }
                calls.push(direct);
            }
            Payload::ImportSection(reader) => assert_eq!(reader.count(), 0, "no imports"),
            Payload::TableSection(_) | Payload::ElementSection(_) => panic!("no function table"),
            _ => {}
        }
    }

    use ValType::{F64, I32};
    let function_signatures: Vec<_> = functions
        .iter()
        .map(|index| signatures[usize::try_from(*index).unwrap()].clone())
        .collect();
    assert_eq!(
        function_signatures,
        [
            (vec![F64], vec![I32]),                // 0: make
            (vec![I32], vec![F64]),                // 1: inspect
            (vec![], vec![F64]),                   // 2: main
            (vec![I32], vec![I32]),                // 3: Alloc
            (vec![I32], vec![I32]),                // 4: ValidateString
            (vec![I32], vec![I32]),                // 5: ValidateArray
            (vec![I32, F64], vec![I32]),           // 6: ArrayAddress
            (vec![I32], vec![F64]),                // 7: ArrayLen
            (vec![I32, I32], vec![I32]),           // 8: StringEqual
            (vec![I32, I32], vec![I32]),           // 9: ArrayEqual
            (vec![F64, F64], vec![F64]),           // 10: floating remainder
            (vec![F64, F64, F64, I32], vec![I32]), // 11: Construct
            (vec![I32], vec![I32]),                // 12: Validate
            (vec![F64, I32], vec![I32]),           // 13: Contains
        ]
    );
    assert_eq!(
        calls,
        [
            vec![11],                     // make constructs the private range
            vec![12, 3, 13, 6, 6, 10, 7], // inspect validates, allocates, tests, indexes, computes
            vec![0, 1],                   // pipeline stages remain direct user calls
            vec![],
            vec![],
            vec![],
            vec![5],
            vec![5], // array address and length validate arrays
            vec![4, 4],
            vec![5, 5], // equality validates both complete descriptors
            vec![],     // floating remainder needs no helpers
            vec![3],
            vec![],
            vec![12], // range construction allocates; membership validates
        ]
    );
    assert_eq!(exports.len(), 3);
    assert!(exports.contains(&("main", ExternalKind::Func, 2)));
    assert!(exports.contains(&("memory", ExternalKind::Memory, 0)));
    assert!(exports.contains(&("__alloc", ExternalKind::Func, 3)));
}

#[test]
fn injection_precedes_written_arguments_and_defaults_with_direct_formal_order_call() {
    let program = fixture();
    let snapshot = program.clone();
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::TableSection(_) | Payload::ElementSection(_) => panic!("no function table"),
            Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    let locals: Vec<_> = bodies[1]
        .get_locals_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(locals, [(1, ValType::F64); 4]);
    let ops: Vec<_> = bodies[1]
        .get_operators_reader()
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(matches!(ops.as_slice(), [
        Operator::F64Const { value: input }, Operator::LocalSet { local_index: 0 },
        Operator::F64Const { value: c }, Operator::LocalSet { local_index: 1 },
        Operator::F64Const { value: b }, Operator::LocalSet { local_index: 2 },
        Operator::LocalGet { local_index: 0 }, Operator::LocalGet { local_index: 2 },
        Operator::F64Add, Operator::LocalSet { local_index: 3 },
        Operator::LocalGet { local_index: 0 }, Operator::LocalGet { local_index: 2 },
        Operator::LocalGet { local_index: 1 }, Operator::LocalGet { local_index: 3 },
        Operator::Call { function_index: 0 }, Operator::Return, Operator::End,
    ] if input.bits() == 11f64.to_bits() && c.bits() == 33f64.to_bits() && b.bits() == 22f64.to_bits()));
    assert_eq!(program, snapshot);
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::new();
        compiler.set_optimization_level(level);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
    }
}

#[test]
fn duplicate_misplaced_and_wrongly_mapped_injections_are_rejected() {
    for case in 0..5 {
        let mut program = fixture();
        let sequence = normalized(returned(&mut program));
        let (reason, span) = match case {
            0 => {
                sequence.bindings[1].origin = CallBindingOrigin::PipelineInput;
                (
                    "pipeline input must be the first and only injected binding",
                    sequence.bindings[1].span,
                )
            }
            1 => {
                sequence.bindings.swap(0, 1);
                sequence.bindings[0].origin = CallBindingOrigin::Explicit { argument: 0 };
                (
                    "pipeline input must be the first and only injected binding",
                    sequence.bindings[1].span,
                )
            }
            2 => {
                sequence.formal_to_binding.swap(0, 1);
                (
                    "pipeline input must map to formal zero",
                    sequence.bindings[0].span,
                )
            }
            3 => {
                sequence.bindings[1].origin = CallBindingOrigin::Explicit { argument: 1 };
                (
                    "bindings must have contiguous written-order explicit origins",
                    sequence.bindings[1].span,
                )
            }
            4 => {
                sequence.bindings[3].origin = CallBindingOrigin::PipelineInput;
                (
                    "pipeline input must be the first and only injected binding",
                    sequence.bindings[3].span,
                )
            }
            _ => unreachable!(),
        };
        assert_invalid(&program, reason, span);
    }
}

#[test]
fn nested_injections_use_distinct_slots_and_only_direct_calls() {
    let program = nested_fixture();
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::TableSection(_) | Payload::ElementSection(_) => panic!("no function table"),
            Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    let mut sets = Vec::new();
    let mut gets = Vec::new();
    let mut calls = Vec::new();
    let mut constants = Vec::new();
    for op in bodies[1].get_operators_reader().unwrap() {
        match op.unwrap() {
            Operator::LocalSet { local_index } => sets.push(local_index),
            Operator::LocalGet { local_index } => gets.push(local_index),
            Operator::Call { function_index } => calls.push(function_index),
            Operator::CallIndirect { .. } => panic!("no indirect call"),
            Operator::F64Const { value } => constants.push(f64::from_bits(value.bits())),
            _ => {}
        }
    }
    assert_eq!(sets, [1, 2, 3, 4, 0, 5, 6, 7]);
    assert_eq!(gets, [1, 3, 1, 3, 2, 4, 0, 6, 0, 6, 5, 7]);
    assert_eq!(calls, [0, 0]);
    assert_eq!(constants, [5.0, 7.0, 6.0, 33.0, 22.0]);
}

#[test]
fn injection_and_explicit_initializers_cannot_read_owned_bindings_even_when_nested() {
    for nested in [false, true] {
        for initializer in 0..3 {
            for owned in 0..4 {
                let mut program = if nested { nested_fixture() } else { fixture() };
                let sequence = normalized(returned(&mut program));
                let id = sequence.bindings[owned].id;
                let value = &mut sequence.bindings[initializer].value;
                let value = if nested && initializer == 0 {
                    &mut normalized(value).bindings[0].value
                } else {
                    value
                };
                value.kind = TypedExprKind::Identifier {
                    id,
                    name: "forged".into(),
                };
                value.ty = Type::Number.into();
                assert!(
                    matches!(
                        Compiler::new().compile(&program),
                        Err(CodegenError::InvalidNormalizedCall { .. })
                    ),
                    "nested {nested}, initializer {initializer}, owned {owned}"
                );
            }
        }
    }
}

#[test]
fn nested_binding_ids_cannot_collide_or_escape_to_outer_arguments() {
    let mut program = nested_fixture();
    let outer = normalized(returned(&mut program));
    let outer_id = outer.bindings[0].id;
    normalized(&mut outer.bindings[0].value).bindings[0].id = outer_id;
    assert!(matches!(
        Compiler::new().compile(&program),
        Err(CodegenError::DuplicateVariableId { .. })
    ));

    let mut program = nested_fixture();
    let outer = normalized(returned(&mut program));
    let inner_id = normalized(&mut outer.bindings[0].value).bindings[0].id;
    outer.bindings[1].value = TypedExpr {
        kind: TypedExprKind::Identifier {
            id: inner_id,
            name: "escaped".into(),
        },
        ty: Type::Number.into(),
    };
    assert!(matches!(
        Compiler::new().compile(&program),
        Err(CodegenError::InvalidNormalizedCall { .. })
    ));
}

#[test]
fn injections_accept_caller_locals_and_each_concrete_storage_type() {
    for (ty, initializer, storage) in [
        ("number", "11", ValType::F64),
        ("bool", "true", ValType::I32),
        ("string", "\"value\"", ValType::I32),
        ("number[]", "[1, 2]", ValType::I32),
        ("range", "1..3", ValType::I32),
    ] {
        let source = format!(
            "fn target(a: {ty}, b: number = 1) -> {ty} {{ return a; }}
             fn main() -> {ty} {{ let input = {initializer}; return target(input); }}"
        );
        let mut program = Compiler::new().analyze(&source).unwrap();
        let TypedStatementKind::Return(Some(value)) = &mut program.functions[1].body[1].kind else {
            panic!("return")
        };
        inject(value);
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::None);
        let bytes = compiler.compile(&program).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        let body = Parser::new(0)
            .parse_all(&bytes)
            .filter_map(|payload| match payload.unwrap() {
                Payload::CodeSectionEntry(body) => Some(body),
                Payload::TableSection(_) | Payload::ElementSection(_) => {
                    panic!("no function table")
                }
                _ => None,
            })
            .nth(1)
            .unwrap();
        let locals: Vec<_> = body
            .get_locals_reader()
            .unwrap()
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            &locals[..3],
            [(1, storage), (1, storage), (1, ValType::F64)],
            "{ty}"
        );
        let mut calls = Vec::new();
        for op in body.get_operators_reader().unwrap() {
            match op.unwrap() {
                Operator::Call { function_index } => calls.push(function_index),
                Operator::CallIndirect { .. } => panic!("no indirect call"),
                _ => {}
            }
        }
        assert_eq!(calls.last(), Some(&0), "{ty}");
    }
}

#[test]
fn injected_transfer_uses_the_caller_context_with_a_live_stack_prefix() {
    use was_compiler::{
        ast::BinaryOp,
        typed_ast::{ExprResult, TypedStatement, TypedValueBlock},
    };

    let mut program = fixture();
    let value = returned(&mut program);
    let sequence = normalized(value);
    let span = sequence.bindings[0].span;
    sequence.bindings[0].value = TypedExpr {
        kind: TypedExprKind::Block(TypedValueBlock {
            statements: vec![TypedStatement {
                kind: TypedStatementKind::Return(Some(TypedExpr {
                    kind: TypedExprKind::Number("5".into()),
                    ty: Type::Number.into(),
                })),
                span,
            }],
            tail: None,
            span,
        }),
        ty: ExprResult::Never,
    };
    value.ty = ExprResult::Never;
    *value = TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(TypedExpr {
                kind: TypedExprKind::Number("99".into()),
                ty: Type::Number.into(),
            }),
            op: BinaryOp::Add,
            right: Box::new(value.clone()),
        },
        ty: ExprResult::Never,
    };
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn never_injection_preserves_signature_result_and_checks_unreachable_children() {
    use was_compiler::typed_ast::{ExprResult, LoopId};

    let mut program = fixture();
    let value = returned(&mut program);
    value.ty = ExprResult::Never;
    let sequence = normalized(value);
    sequence.bindings[0].value = TypedExpr {
        kind: TypedExprKind::Loop {
            id: LoopId(77),
            body: vec![],
        },
        ty: ExprResult::Never,
    };
    assert_eq!(sequence.call.ty, ExprResult::Value(Type::Number));
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();

    let mut forged = program.clone();
    returned(&mut forged).ty = Type::Number.into();
    assert!(Compiler::new().compile(&forged).is_err());

    let sequence = normalized(returned(&mut program));
    let span = sequence.bindings[2].span;
    sequence.bindings[2].value.kind = TypedExprKind::Number("invalid".into());
    assert!(
        matches!(Compiler::new().compile(&program), Err(CodegenError::InvalidNumberLiteral { span: actual, .. }) if actual == span)
    );
}

#[test]
fn final_void_call_is_valid_but_void_cannot_be_injected() {
    use was_compiler::typed_ast::ExprResult;

    let mut program = Compiler::new()
        .analyze("fn target(a: number, b: number = a) -> void {} fn main() -> void { target(5); }")
        .unwrap();
    let TypedStatementKind::Expression(value) = &mut program.functions[1].body[0].kind else {
        panic!("expression")
    };
    inject(value);
    let void_value = value.clone();
    let bytes = Compiler::new().compile(&program).unwrap();
    Validator::new().validate_all(&bytes).unwrap();

    let mut receiving = fixture();
    normalized(returned(&mut receiving)).bindings[0].value = void_value;
    assert!(Compiler::new().compile(&receiving).is_err());

    let TypedStatementKind::Expression(value) = &mut program.functions[1].body[0].kind else {
        panic!("expression")
    };
    value.ty = ExprResult::Value(Type::Number);
    assert!(Compiler::new().compile(&program).is_err());
}

#[test]
fn skipped_and_unexported_injections_still_check_origins_and_default_reads() {
    use was_compiler::{ast::BinaryOp, typed_ast::TypedStatement};

    for skipped in [false, true] {
        for default_read in [false, true] {
            let mut program = fixture();
            let span = program.functions[1].body[0].span;
            let sequence = normalized(returned(&mut program));
            let (reason, error_span) = if default_read {
                let id = sequence.bindings[3].id;
                sequence.bindings[3].value = TypedExpr {
                    kind: TypedExprKind::Identifier {
                        id,
                        name: "self".into(),
                    },
                    ty: Type::Number.into(),
                };
                (
                    "defaults may only read owned earlier-formal bindings",
                    sequence.bindings[3].span,
                )
            } else {
                sequence.bindings[2].origin = CallBindingOrigin::PipelineInput;
                (
                    "pipeline input must be the first and only injected binding",
                    sequence.bindings[2].span,
                )
            };
            let value = returned(&mut program).clone();
            let expression = if skipped {
                TypedExpr {
                    kind: TypedExprKind::Binary {
                        left: Box::new(TypedExpr {
                            kind: TypedExprKind::Bool(false),
                            ty: Type::Bool.into(),
                        }),
                        op: BinaryOp::And,
                        right: Box::new(TypedExpr {
                            kind: TypedExprKind::Binary {
                                left: Box::new(value),
                                op: BinaryOp::Equal,
                                right: Box::new(TypedExpr {
                                    kind: TypedExprKind::Number("0".into()),
                                    ty: Type::Number.into(),
                                }),
                            },
                            ty: Type::Bool.into(),
                        }),
                    },
                    ty: Type::Bool.into(),
                }
            } else {
                value
            };
            let statement = TypedStatement {
                kind: TypedStatementKind::Expression(expression),
                span,
            };
            if skipped {
                program.functions[1].body.insert(0, statement);
            } else {
                program.functions[1].body.push(statement);
                *returned(&mut program) = TypedExpr {
                    kind: TypedExprKind::Number("0".into()),
                    ty: Type::Number.into(),
                };
            }
            // The original return must not duplicate the generated binding IDs.
            if skipped {
                let TypedStatementKind::Return(Some(value)) =
                    &mut program.functions[1].body[1].kind
                else {
                    panic!("return")
                };
                *value = TypedExpr {
                    kind: TypedExprKind::Number("0".into()),
                    ty: Type::Number.into(),
                };
            }
            assert_invalid(&program, reason, error_span);
        }
    }
}

#[test]
fn injection_retains_mapping_type_result_and_default_whitelist_checks() {
    for case in 0..8 {
        let mut program = nested_fixture();
        let outer = normalized(returned(&mut program));
        let inner = normalized(&mut outer.bindings[0].value);
        match case {
            0 => {
                inner.bindings[0].value = TypedExpr {
                    kind: TypedExprKind::Bool(true),
                    ty: Type::Bool.into(),
                }
            }
            1 => inner.bindings[0].ty = Type::Bool,
            2 => inner.call.ty = Type::Bool.into(),
            3 => inner.formal_to_binding[1] = inner.formal_to_binding[0],
            4 => inner.bindings[3].value = *inner.call.clone(),
            5 => inner.bindings[3].origin = CallBindingOrigin::Default { formal: 0 },
            6 => {
                let TypedExprKind::Call { args, .. } = &mut inner.call.kind else {
                    panic!("call")
                };
                args.swap(0, 1);
            }
            7 => outer.bindings[0].value.ty = Type::Bool.into(),
            _ => unreachable!(),
        }
        let mut compiler = Compiler::new();
        compiler.set_exports(ExportOptions::None);
        assert!(compiler.compile(&program).is_err(), "case {case}");
    }
}
