use super::*;

#[test]
fn empty_module_is_valid() {
    let program = TypedProgram {
        functions: vec![],
        host_functions: vec![],
    };
    let module = decode(&compile(&program).expect("compile empty module"));

    assert_eq!(
        module,
        DecodedModule {
            sections: vec![1, 2, 3, 7, 10],
            ..DecodedModule::default()
        }
    );
}

#[test]
fn empty_void_functions_have_only_end_and_are_exported_in_source_order() {
    let program = TypedProgram {
        functions: vec![user(0, "z", &[], Type::Void), user(1, "a", &[], Type::Void)],
        host_functions: vec![],
    };

    let module = decode(&compile(&program).expect("compile void functions"));

    assert_eq!(module.signatures, vec![(vec![], vec![])]);
    assert_eq!(module.functions, vec![0, 0]);
    assert_eq!(module.locals, vec![vec![], vec![]]);
    assert_eq!(
        module.exports,
        vec![
            ("z".to_owned(), ExternalKind::Func, 0),
            ("a".to_owned(), ExternalKind::Func, 1),
        ]
    );
    assert_eq!(
        module.bodies,
        vec![vec![DecodedInstruction::End], vec![DecodedInstruction::End]],
    );
}

#[test]
fn signatures_are_deduplicated_across_imports_and_users_with_import_first_indices() {
    let program = TypedProgram {
        host_functions: vec![
            host(0, "shared", &[Type::Number, Type::Bool], Type::Void),
            host(1, "numeric", &[Type::Bool, Type::Number], Type::Number),
            host(2, "predicate", &[], Type::Bool),
            host(3, "again", &[Type::Number, Type::Bool], Type::Void),
        ],
        functions: vec![
            user(0, "shared", &[Type::Number, Type::Bool], Type::Void),
            user(1, "empty", &[], Type::Void),
            user(2, "again", &[Type::Number, Type::Bool], Type::Void),
        ],
    };

    let module = decode(&compile(&program).expect("compile mixed module"));

    assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
    assert_eq!(
        module.signatures,
        vec![
            (vec![ValType::F64, ValType::I32], vec![]),
            (vec![ValType::I32, ValType::F64], vec![ValType::F64]),
            (vec![], vec![ValType::I32]),
            (vec![], vec![]),
        ]
    );
    assert_eq!(
        module.imports,
        vec![
            ("env".to_owned(), "host_shared".to_owned(), 0),
            ("env".to_owned(), "host_numeric".to_owned(), 1),
            ("env".to_owned(), "host_predicate".to_owned(), 2),
            ("env".to_owned(), "host_again".to_owned(), 0),
        ]
    );
    assert_eq!(module.functions, vec![0, 3, 0]);
    assert_eq!(
        module.exports,
        vec![
            ("shared".to_owned(), ExternalKind::Func, 4),
            ("empty".to_owned(), ExternalKind::Func, 5),
            ("again".to_owned(), ExternalKind::Func, 6),
        ]
    );
    assert_eq!(
        module.bodies,
        vec![
            vec![DecodedInstruction::End],
            vec![DecodedInstruction::End],
            vec![DecodedInstruction::End],
        ],
    );
}

#[test]
fn import_only_module_has_no_exports_or_bodies() {
    let mut other = host(1, "same", &[Type::Number], Type::Bool);

    other.module = "other".to_owned();
    other.field = "predicate".to_owned();

    let program = TypedProgram {
        functions: vec![],
        host_functions: vec![host(0, "same", &[Type::Number], Type::Bool), other],
    };

    let module = decode(&compile(&program).expect("compile import-only module"));

    assert_eq!(
        module.signatures,
        vec![(vec![ValType::F64], vec![ValType::I32])]
    );
    assert_eq!(
        module.imports,
        vec![
            ("env".to_owned(), "host_same".to_owned(), 0),
            ("other".to_owned(), "predicate".to_owned(), 0),
        ]
    );
    assert!(module.functions.is_empty());
    assert!(module.exports.is_empty());
    assert!(module.bodies.is_empty());
}

#[test]
fn output_is_deterministic() {
    let program = TypedProgram {
        functions: vec![
            user(0, "first", &[Type::Number], Type::Void),
            user(1, "second", &[], Type::Void),
        ],
        host_functions: vec![host(0, "unused", &[Type::Bool], Type::Number)],
    };

    let expected = compile(&program).expect("compile module");

    assert_eq!(
        decode(&expected).bodies,
        vec![vec![DecodedInstruction::End], vec![DecodedInstruction::End]],
    );

    for _ in 0..20 {
        assert_eq!(compile(&program).expect("repeat compilation"), expected);
    }
}

#[test]
fn signatures_distinguish_parameter_order_and_return_types() {
    let program = TypedProgram {
        functions: vec![],
        host_functions: vec![
            host(0, "nb", &[Type::Number, Type::Bool], Type::Void),
            host(1, "bn", &[Type::Bool, Type::Number], Type::Void),
            host(2, "number", &[Type::Number, Type::Bool], Type::Number),
            host(3, "bool", &[Type::Number, Type::Bool], Type::Bool),
        ],
    };

    let module = decode(&compile(&program).expect("compile signatures"));

    assert_eq!(
        module.signatures,
        vec![
            (vec![ValType::F64, ValType::I32], vec![]),
            (vec![ValType::I32, ValType::F64], vec![]),
            (vec![ValType::F64, ValType::I32], vec![ValType::F64]),
            (vec![ValType::F64, ValType::I32], vec![ValType::I32]),
        ]
    );
}

#[test]
fn memory_parameter_and_return_abis_use_i32_for_hosts_and_users() {
    for ty in [
        Type::String,
        Type::Array(crate::ast::ArrayElementType::Number),
    ] {
        for is_host in [false, true] {
            for position in ["parameter", "return"] {
                let params = if position == "parameter" {
                    vec![ty]
                } else {
                    vec![]
                };
                let return_type = if position == "return" { ty } else { Type::Void };
                let mut function = user(0, "bad", &params, return_type);
                if position == "return" {
                    function.body.push(TypedStatement {
                        kind: TypedStatementKind::Return(Some(TypedExpr {
                            kind: if ty == Type::String {
                                TypedExprKind::String("value".to_owned())
                            } else {
                                TypedExprKind::Array(
                                    vec![number()],
                                    crate::ast::ArrayElementType::Number,
                                )
                            },
                            ty: ty.into(),
                        })),
                        span: Span::new(12, 34),
                    });
                }
                let program = if is_host {
                    TypedProgram {
                        functions: vec![],
                        host_functions: vec![host(0, "bad", &params, return_type)],
                    }
                } else {
                    TypedProgram {
                        functions: vec![function],
                        host_functions: vec![],
                    }
                };
                let module = decode(&compile(&program).expect("compile memory ABI"));
                let type_index = if is_host {
                    module.imports[0].2
                } else {
                    module.functions[0]
                };
                assert_eq!(
                    module.signatures[type_index as usize],
                    (
                        if position == "parameter" {
                            vec![ValType::I32]
                        } else {
                            vec![]
                        },
                        if position == "return" {
                            vec![ValType::I32]
                        } else {
                            vec![]
                        },
                    )
                );
            }
        }
    }
}

#[test]
fn numeric_returns_preserve_parameter_signatures_and_import_index_offsets() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function(
            "numeric",
            "env",
            "numeric",
            vec![Type::Number, Type::Bool],
            Type::Number,
        )
        .unwrap();
    compiler
        .register_host_function("ready", "env", "ready", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn value(n: number, flag: bool) -> number { return 42; }
             fn decimal(flag: bool, n: number) -> number { return 0.25; }
             fn noop() -> void {}",
        )
        .expect("compile numeric signatures with unused imports");
    let module = decode(&bytes);

    assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
    assert_eq!(
        module.signatures,
        vec![
            (vec![ValType::F64, ValType::I32], vec![ValType::F64]),
            (vec![], vec![ValType::I32]),
            (vec![ValType::I32, ValType::F64], vec![ValType::F64]),
            (vec![], vec![]),
        ],
    );
    assert_eq!(
        module.imports,
        vec![
            ("env".to_owned(), "numeric".to_owned(), 0),
            ("env".to_owned(), "ready".to_owned(), 1),
        ],
    );
    assert_eq!(module.functions, vec![0, 2, 3]);
    assert_eq!(
        module.exports,
        vec![
            ("value".to_owned(), ExternalKind::Func, 2),
            ("decimal".to_owned(), ExternalKind::Func, 3),
            ("noop".to_owned(), ExternalKind::Func, 4),
        ],
    );
    assert_eq!(
        module.bodies,
        vec![
            vec![
                DecodedInstruction::F64Const(42.0),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![
                DecodedInstruction::F64Const(0.25),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![DecodedInstruction::End],
        ],
    );
}

#[test]
fn forged_unused_host_signatures_are_checked_before_export_selection() {
    use crate::{codegen::compile_with_exports, exports::ExportOptions};

    for index in [0, 1, 2] {
        let mut params = vec![Type::Number, Type::String];
        params.insert(index, Type::Void);

        let program = TypedProgram {
            functions: vec![user(0, "visible", &[], Type::Void)],
            host_functions: vec![
                host(0, "valid", &[Type::Bool], Type::Void),
                host(1, "unused", &params, Type::Void),
            ],
        };

        for exports in [
            ExportOptions::All,
            ExportOptions::None,
            ExportOptions::Only(vec!["visible".into()]),
            ExportOptions::Only(vec!["missing".into()]),
        ] {
            let error = compile_with_exports(&program, &exports).unwrap_err();

            assert_eq!(
                error,
                CodegenError::VoidParameter {
                    function: "unused".into(),
                    index,
                }
            );
            assert_eq!(error.span(), None);
            assert_eq!(
                error.to_string(),
                format!("parameter {index} in function `unused` cannot have type void")
            );
        }
    }
}

#[test]
fn internal_signatures_are_checked_even_when_not_exported() {
    use crate::{codegen::compile_with_exports, exports::ExportOptions};

    let program = TypedProgram {
        functions: vec![
            user(0, "visible", &[], Type::Void),
            user(1, "hidden", &[Type::Number, Type::Void], Type::Void),
        ],
        host_functions: vec![],
    };

    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["visible".into()]),
        ExportOptions::Only(vec!["missing".into()]),
    ] {
        assert_eq!(
            compile_with_exports(&program, &exports),
            Err(CodegenError::VoidParameter {
                function: "hidden".into(),
                index: 1,
            })
        );
    }
}

#[test]
fn external_selection_preserves_internal_signatures_snapshots_and_helpers() {
    use crate::exports::ExportOptions;

    let mut compiler = Compiler::new();
    let params = vec![
        Type::Number,
        Type::Bool,
        Type::String,
        Type::Array(crate::ast::ArrayElementType::Number),
    ];

    for (name, ty) in [
        ("numeric", Type::Number),
        ("boolean", Type::Bool),
        ("text", Type::String),
        ("array", Type::Array(crate::ast::ArrayElementType::Number)),
        ("nothing", Type::Void),
    ] {
        compiler
            .register_host_function(format!("host_{name}"), "env", name, params.clone(), ty)
            .unwrap();
    }

    let program = compiler
        .analyze(
            "fn numeric(n: number, b: bool, s: string, a: number[]) -> number { return n % 2; }
             fn boolean(n: number, b: bool, s: string, a: number[]) -> bool { return b; }
             fn text(n: number, b: bool, s: string, a: number[]) -> string { return s; }
             fn array(n: number, b: bool, s: string, a: number[]) -> number[] { return a; }
             fn nothing(n: number, b: bool, s: string, a: number[]) -> void {}",
        )
        .unwrap();
    compiler
        .register_host_function("later", "other", "later", vec![], Type::Void)
        .unwrap();

    let baseline = decode(&compiler.compile(&program).unwrap());
    let raw_params = vec![ValType::F64, ValType::I32, ValType::I32, ValType::I32];

    assert_eq!(
        &baseline.signatures[..5],
        &[
            (raw_params.clone(), vec![ValType::F64]),
            (raw_params.clone(), vec![ValType::I32]),
            (raw_params, vec![]),
            (vec![ValType::I32], vec![ValType::I32]),
            (vec![ValType::I32, ValType::F64], vec![ValType::I32]),
        ]
    );
    assert_eq!(baseline.functions.len(), 13); // Five users, seven memory helpers, remainder.
    assert_eq!(baseline.functions[..5], [0, 1, 1, 1, 2]);
    assert_eq!(
        baseline.imports,
        vec![
            ("env".into(), "numeric".into(), 0),
            ("env".into(), "boolean".into(), 1),
            ("env".into(), "text".into(), 1),
            ("env".into(), "array".into(), 1),
            ("env".into(), "nothing".into(), 2),
        ]
    );
    assert!(baseline.bodies[0].contains(&DecodedInstruction::Call(17)));

    for (exports, expected) in [
        (
            ExportOptions::All,
            vec![
                ("numeric", 5),
                ("boolean", 6),
                ("text", 7),
                ("array", 8),
                ("nothing", 9),
            ],
        ),
        (ExportOptions::None, vec![]),
        (
            ExportOptions::Only(vec!["array".into(), "numeric".into()]),
            vec![("numeric", 5), ("array", 8)],
        ),
    ] {
        compiler.set_exports(exports.clone());
        compiler.set_optimization_level(OptimizationLevel::None);
        let expected_bytes = compiler.compile(&program).unwrap();
        assert_eq!(
            expected_bytes,
            super::super::compile_with_exports(&program, &exports).unwrap()
        );
        let mut enabled_bytes = None;

        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            compiler.set_optimization_level(level);
            let bytes = compiler.compile(&program).unwrap();
            assert_eq!(compiler.compile(&program).unwrap(), bytes);
            if level == OptimizationLevel::None {
                assert_eq!(bytes, expected_bytes);
            } else if let Some(expected) = &enabled_bytes {
                assert_eq!(&bytes, expected);
            } else {
                enabled_bytes = Some(bytes.clone());
            }
            let module = decode(&bytes);
            let mut expected_exports: Vec<_> = expected
                .iter()
                .map(|(name, index)| ((*name).to_owned(), ExternalKind::Func, *index))
                .collect();
            expected_exports.extend([
                ("memory".into(), ExternalKind::Memory, 0),
                ("__alloc".into(), ExternalKind::Func, 10),
            ]);

            assert_eq!(module.exports, expected_exports);
            assert_eq!(module.signatures, baseline.signatures);
            assert_eq!(module.imports, baseline.imports);
            assert_eq!(module.functions, baseline.functions);
            assert_eq!(module.locals, baseline.locals);
            assert_eq!(module.bodies.len(), baseline.bodies.len());
            use DecodedInstruction::*;
            let mut expected_numeric = vec![
                LocalGet(2),
                Call(11),
                Drop,
                LocalGet(3),
                Call(12),
                Drop,
                LocalGet(0),
            ];
            if level == OptimizationLevel::None {
                expected_numeric.push(F64Const(2.0));
            } else {
                expected_numeric.extend([I32Const(2), Runtime("F64ConvertI32S".into())]);
            }
            expected_numeric.extend([Call(17), Return, End]);
            assert_eq!(module.bodies[0], expected_numeric);
            for (body, original) in module.bodies[1..].iter().zip(&baseline.bodies[1..]) {
                assert_eq!(body.len(), original.len());
                for (instruction, original) in body.iter().zip(original) {
                    match (instruction, original) {
                        (
                            DecodedInstruction::F64Const(value),
                            DecodedInstruction::F64Const(expected),
                        ) => {
                            assert_eq!(value.to_bits(), expected.to_bits());
                        }
                        _ => assert_eq!(instruction, original),
                    }
                }
            }
        }
    }
}

#[test]
fn u32_limits_are_checked_without_truncation() {
    assert_eq!(checked_u32(0, "index"), Ok(0));
    assert_eq!(checked_u32(u32::MAX as usize, "index"), Ok(u32::MAX));

    if let Some(too_large) = (u32::MAX as usize).checked_add(1) {
        assert_eq!(
            checked_u32(too_large, "index"),
            Err(CodegenError::LimitExceeded { what: "index" })
        );
    }
}
