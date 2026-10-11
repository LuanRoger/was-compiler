use crate::{
    ast::Type,
    compiler::Compiler,
    exports::ExportOptions,
    typed_ast::{TypedExpr, TypedExprKind, TypedStatementKind},
};
use wasmparser::{Operator, Parser, Payload, Validator};

use super::{CodegenError, compile_with_exports};

fn bodies(bytes: &[u8]) -> Vec<Vec<Operator<'_>>> {
    Validator::new().validate_all(bytes).unwrap();
    Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| {
            if let Payload::CodeSectionEntry(body) = payload.unwrap() {
                Some(
                    body.get_operators_reader()
                        .unwrap()
                        .into_iter()
                        .map(Result::unwrap)
                        .collect(),
                )
            } else {
                None
            }
        })
        .collect()
}

fn construction() -> TypedExpr {
    let number = |value: &str| TypedExpr {
        kind: TypedExprKind::Number(value.into()),
        ty: Type::Number.into(),
    };
    TypedExpr {
        kind: TypedExprKind::Range {
            start: Box::new(number("1")),
            end: Box::new(number("9")),
            step: Some(Box::new(number("2"))),
            inclusive: true,
        },
        ty: Type::Range.into(),
    }
}

#[test]
fn range_construction_uses_private_integer_record_and_appended_helpers() {
    let mut program = Compiler::new()
        .analyze("fn make() -> number { return 1; }")
        .unwrap();
    program.functions[0].return_type = Type::Range;
    program.functions[0].body[0].kind = TypedStatementKind::Return(Some(construction()));
    let bytes = compile_with_exports(&program, &ExportOptions::None).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            bodies.push(
                body.get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>(),
            );
        }
    }
    // One user, seven legacy memory helpers, then construction and validation.
    assert_eq!(bodies.len(), 10);
    assert!(matches!(
        bodies[0].as_slice(),
        [
            Operator::F64Const { .. },
            Operator::F64Const { .. },
            Operator::F64Const { .. },
            Operator::I32Const { value: 1 },
            Operator::Call { function_index: 8 },
            Operator::Return,
            Operator::End
        ]
    ));
    let stores = bodies[8]
        .iter()
        .filter_map(|op| match op {
            Operator::I64Store { memarg } => Some((64, memarg.offset, memarg.align)),
            Operator::I32Store { memarg } => Some((32, memarg.offset, memarg.align)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stores,
        [
            (64, 0, 3),
            (64, 8, 3),
            (64, 16, 3),
            (32, 24, 2),
            (32, 28, 2)
        ]
    );
    assert!(
        bodies[8]
            .iter()
            .any(|op| matches!(op, Operator::I32Const { value: 32 }))
    );
    assert!(
        bodies[8]
            .iter()
            .any(|op| matches!(op, Operator::I64TruncF64S))
    );
}

#[test]
fn internal_range_parameters_are_validated_as_records_not_descriptors() {
    let mut program = Compiler::new()
        .analyze("fn identity(value: number) -> number { return value; }")
        .unwrap();
    let function = &mut program.functions[0];
    function.params[0].ty = Type::Range;
    function.return_type = Type::Range;
    let TypedStatementKind::Return(Some(value)) = &mut function.body[0].kind else {
        unreachable!()
    };
    value.ty = Type::Range.into();
    let bytes = compile_with_exports(&program, &ExportOptions::None).unwrap();
    let bodies = bodies(&bytes);
    assert!(matches!(
        bodies[0].as_slice(),
        [
            Operator::LocalGet { local_index: 0 },
            Operator::Call { function_index: 9 },
            Operator::Drop,
            Operator::LocalGet { local_index: 0 },
            Operator::Return,
            Operator::End
        ]
    ));
    let loads = bodies[9]
        .iter()
        .filter_map(|op| match op {
            Operator::I64Load { memarg } => Some((64, memarg.offset)),
            Operator::I32Load { memarg } => Some((32, memarg.offset)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        loads,
        [
            (64, 0),
            (64, 0),
            (64, 8),
            (64, 8),
            (64, 16),
            (64, 16),
            (64, 16),
            (32, 24),
            (32, 28)
        ]
    );
    assert!(
        bodies[9]
            .iter()
            .any(|op| matches!(op, Operator::I64ExtendI32U))
    );
    assert!(
        bodies[9]
            .iter()
            .any(|op| matches!(op, Operator::I64Const { value: 32 }))
    );
    assert!(bodies[9].iter().any(|op| matches!(
        op,
        Operator::I64Const {
            value: -9007199254740991
        }
    )));
    assert!(bodies[9].iter().any(|op| matches!(
        op,
        Operator::I64Const {
            value: 9007199254740991
        }
    )));
    // The internal signature remains a single i32 handle, including its result.
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::TypeSection(reader) = payload.unwrap() {
            let types = reader
                .into_iter_err_on_gc_types()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            assert_eq!(types[0].params(), [wasmparser::ValType::I32]);
            assert_eq!(types[0].results(), [wasmparser::ValType::I32]);
        }
    }
    for exports in [
        ExportOptions::All,
        ExportOptions::Only(vec!["identity".into()]),
    ] {
        let error = compile_with_exports(&program, &exports).unwrap_err();
        assert!(matches!(error, CodegenError::RangeExport { .. }));
        assert!(error.to_string().contains("explicit export selection"));
        assert_eq!(error.span(), None);
    }
}

#[test]
fn range_helpers_follow_remainder_even_when_only_hidden_construction_needs_them() {
    let mut program = Compiler::new()
        .analyze("fn test() -> void { return; 5 % 2; 1; }")
        .unwrap();
    program.functions[0].body[2].kind = TypedStatementKind::Expression(construction());
    let bytes = compile_with_exports(&program, &ExportOptions::None).unwrap();
    let bodies = bodies(&bytes);
    assert_eq!(bodies.len(), 11);
    let calls = bodies[0]
        .iter()
        .filter_map(|op| match op {
            Operator::Call { function_index } => Some(*function_index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, [8, 9]);
    assert!(
        bodies[9]
            .iter()
            .any(|op| matches!(op, Operator::I64TruncF64S))
    );
    assert!(
        bodies[10]
            .iter()
            .any(|op| matches!(op, Operator::I64Load { .. }))
    );
}

#[test]
fn range_operands_preserve_nested_local_and_temporary_planning() {
    let program = Compiler::new()
        .analyze(
            "fn test() -> void {
            let r = { let a = [1]; len(a) } .. { let b = [2]; len(b) } by { let c = [3]; len(c) };
        }",
        )
        .unwrap();
    let bytes = compile_with_exports(&program, &ExportOptions::None).unwrap();
    let decoded = bodies(&bytes);
    let allocations = decoded[0]
        .iter()
        .filter(|op| matches!(op, Operator::Call { function_index: 1 }))
        .count();
    assert_eq!(allocations, 3);
    assert!(
        decoded[0]
            .iter()
            .any(|op| matches!(op, Operator::Call { function_index: 8 }))
    );
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            let locals = body
                .get_locals_reader()
                .unwrap()
                .into_iter()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            // r, a, b, c, then one unreused array temporary per operand.
            assert_eq!(locals.iter().map(|(count, _)| *count).sum::<u32>(), 7);
            assert!(locals.iter().all(|(_, ty)| *ty == wasmparser::ValType::I32));
            break;
        }
    }
}

#[test]
fn range_value_regions_and_divergent_operands_produce_valid_wasm() {
    for source in [
        "fn test(flag: bool) -> void { let r = if flag { 0..=3 } else { 3..0 by -1 }; let copy = r; }",
        "fn test() -> void { let r = (loop { break 0..3; }); }",
        "fn test() -> void { let r: range = (loop {}) .. 3; }",
        "fn test() -> void { let r: range = 0 .. (loop {}); }",
        "fn test() -> void { let r: range = 0 .. 3 by (loop {}); }",
    ] {
        let program = Compiler::new().analyze(source).unwrap();
        let bytes = compile_with_exports(&program, &ExportOptions::None).unwrap();
        Validator::new().validate_all(&bytes).unwrap();
    }
}

#[test]
fn forged_range_equality_is_rejected_without_defining_handle_equality() {
    for op in [crate::ast::BinaryOp::Equal, crate::ast::BinaryOp::NotEqual] {
        let mut program = Compiler::new()
            .analyze("fn hidden() -> void { return; true; }")
            .unwrap();
        program.functions[0].body[1].kind = TypedStatementKind::Expression(TypedExpr {
            kind: TypedExprKind::Binary {
                left: Box::new(construction()),
                op,
                right: Box::new(construction()),
            },
            ty: Type::Bool.into(),
        });
        assert!(matches!(
            compile_with_exports(&program, &ExportOptions::None),
            Err(CodegenError::UnsupportedOperator { .. })
        ));
    }
}

#[test]
fn forged_range_host_snapshots_are_rejected_even_when_unused() {
    for (params, return_type) in [(vec![Type::Range], Type::Void), (vec![], Type::Range)] {
        let mut program = Compiler::new().analyze("fn test() -> void {}").unwrap();
        program.host_functions.push(crate::host::HostFunction {
            id: crate::host::HostFunctionId(0),
            name: "unused".into(),
            module: "env".into(),
            field: "unused".into(),
            params,
            return_type,
        });
        assert!(matches!(
            compile_with_exports(&program, &ExportOptions::None),
            Err(CodegenError::UnsupportedAbiType {
                ty: Type::Range,
                ..
            })
        ));
    }
}

#[test]
fn forged_range_operand_is_checked_even_when_hidden() {
    let mut program = Compiler::new()
        .analyze("fn hidden() -> void { return; 1; }")
        .unwrap();
    let mut range = construction();
    let TypedExprKind::Range { end, .. } = &mut range.kind else {
        unreachable!()
    };
    **end = TypedExpr {
        kind: TypedExprKind::Bool(true),
        ty: Type::Bool.into(),
    };
    program.functions[0].body[1].kind = TypedStatementKind::Expression(range);
    assert!(matches!(
        compile_with_exports(&program, &ExportOptions::None),
        Err(CodegenError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            ..
        })
    ));
}
