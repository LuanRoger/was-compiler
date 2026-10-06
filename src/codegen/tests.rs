use crate::{
    ast::{BinaryOp, Type},
    host::{HostFunction, HostFunctionId},
    tokens::Span,
    typed_ast::{
        FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedFunction, TypedParam, TypedProgram,
        TypedStatement, TypedStatementKind, VariableId,
    },
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

use super::{CodegenError, checked_u32, compile};

#[derive(Debug, Default, PartialEq, Eq)]
struct DecodedModule {
    sections: Vec<u8>,
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    body_count: usize,
}

fn decode(bytes: &[u8]) -> DecodedModule {
    Validator::new()
        .validate_all(bytes)
        .expect("valid Wasm module");

    let mut module = DecodedModule::default();

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("parse Wasm payload");

        if let Some((id, _)) = payload.as_section() {
            module.sections.push(id);
        }

        match payload {
            Payload::TypeSection(reader) => {
                for group in reader {
                    for ty in group.expect("type group").into_types() {
                        let function = ty.unwrap_func();

                        module
                            .signatures
                            .push((function.params().to_vec(), function.results().to_vec()));
                    }
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.expect("import");
                    let TypeRef::Func(index) = import.ty else {
                        panic!("expected function import");
                    };

                    module
                        .imports
                        .push((import.module.to_owned(), import.name.to_owned(), index));
                }
            }
            Payload::FunctionSection(reader) => {
                module.functions = reader
                    .into_iter()
                    .map(|ty| ty.expect("function type"))
                    .collect();
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.expect("export");

                    module
                        .exports
                        .push((export.name.to_owned(), export.kind, export.index));
                }
            }
            Payload::CodeSectionEntry(body) => {
                assert_eq!(body.get_locals_reader().expect("locals").get_count(), 0);

                let mut operators = body.get_operators_reader().expect("operators");

                assert!(matches!(
                    operators.read().expect("end instruction"),
                    Operator::End
                ));
                assert!(operators.eof());

                module.body_count += 1;
            }
            _ => {}
        }
    }

    module
}

fn user(id: usize, name: &str, params: &[Type], return_type: Type) -> TypedFunction {
    TypedFunction {
        id: FunctionId(id),
        name: name.to_owned(),
        params: params
            .iter()
            .enumerate()
            .map(|(id, ty)| TypedParam {
                id: VariableId(id),
                name: format!("p{id}"),
                ty: *ty,
                span: Span::new(0, 0),
            })
            .collect(),
        return_type,
        body: Vec::new(),
        span: Span::new(0, 0),
    }
}

fn host(id: usize, name: &str, params: &[Type], return_type: Type) -> HostFunction {
    HostFunction {
        id: HostFunctionId(id),
        name: name.to_owned(),
        module: "env".to_owned(),
        field: format!("host_{name}"),
        params: params.to_vec(),
        return_type,
    }
}

fn number() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number("1".to_owned()),
        ty: Type::Number,
    }
}

fn boolean() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Bool(true),
        ty: Type::Bool,
    }
}

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
    assert_eq!(
        module.exports,
        vec![
            ("z".to_owned(), ExternalKind::Func, 0),
            ("a".to_owned(), ExternalKind::Func, 1),
        ]
    );
    assert_eq!(module.body_count, 2);
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
    assert_eq!(module.body_count, 3);
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
    assert_eq!(module.body_count, 0);
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

    decode(&expected);

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
fn unsupported_parameter_and_return_abis_are_rejected_for_hosts_and_users() {
    for ty in [Type::String, Type::NumberArray] {
        for is_host in [false, true] {
            for position in ["parameter", "return"] {
                let params = if position == "parameter" {
                    vec![ty]
                } else {
                    vec![]
                };
                let return_type = if position == "return" { ty } else { Type::Void };
                let program = if is_host {
                    TypedProgram {
                        functions: vec![],
                        host_functions: vec![host(0, "bad", &params, return_type)],
                    }
                } else {
                    TypedProgram {
                        functions: vec![user(0, "bad", &params, return_type)],
                        host_functions: vec![],
                    }
                };

                assert_eq!(
                    compile(&program),
                    Err(CodegenError::UnsupportedAbiType {
                        function: "bad".to_owned(),
                        position,
                        ty,
                    })
                );
            }
        }
    }
}

#[test]
fn void_parameters_are_rejected_for_hosts_and_users() {
    for is_host in [false, true] {
        let program = if is_host {
            TypedProgram {
                functions: vec![],
                host_functions: vec![host(0, "bad", &[Type::Number, Type::Void], Type::Void)],
            }
        } else {
            TypedProgram {
                functions: vec![user(0, "bad", &[Type::Number, Type::Void], Type::Void)],
                host_functions: vec![],
            }
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::VoidParameter {
                function: "bad".to_owned(),
                index: 1,
            })
        );
    }
}

#[test]
fn empty_result_functions_are_rejected_without_placeholder_bodies() {
    for return_type in [Type::Number, Type::Bool] {
        let program = TypedProgram {
            functions: vec![user(0, "bad", &[], return_type)],
            host_functions: vec![],
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::MissingReturnValue {
                function: "bad".to_owned(),
                return_type,
            })
        );
    }
}

#[test]
fn every_statement_kind_is_rejected_with_its_span() {
    let kinds = vec![
        (
            "let",
            TypedStatementKind::Let {
                id: VariableId(0),
                name: "x".to_owned(),
                ty: Type::Number,
                value: number(),
            },
        ),
        (
            "assignment",
            TypedStatementKind::Assignment {
                target: number(),
                value: number(),
            },
        ),
        (
            "if",
            TypedStatementKind::If {
                cond: Box::new(boolean()),
                then: vec![],
                else_: None,
            },
        ),
        (
            "while",
            TypedStatementKind::While {
                cond: Box::new(boolean()),
                body: vec![],
            },
        ),
        ("return", TypedStatementKind::Return(None)),
        ("return", TypedStatementKind::Return(Some(number()))),
        ("expression", TypedStatementKind::Expression(number())),
    ];

    for (kind_name, kind) in kinds {
        let mut function = user(0, "bad", &[], Type::Void);

        function.body.push(TypedStatement {
            kind,
            span: Span::new(12, 34),
        });

        let program = TypedProgram {
            functions: vec![function],
            host_functions: vec![],
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnsupportedStatement {
                function: "bad".to_owned(),
                kind: kind_name,
                span: Span::new(12, 34),
            })
        );
    }
}

#[test]
fn arithmetic_and_host_calls_are_not_emitted() {
    let expressions = vec![
        TypedExpr {
            kind: TypedExprKind::Binary {
                left: Box::new(number()),
                op: BinaryOp::Add,
                right: Box::new(number()),
            },
            ty: Type::Number,
        },
        TypedExpr {
            kind: TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(0)),
                name: "host".to_owned(),
                args: vec![],
            },
            ty: Type::Void,
        },
    ];

    for expression in expressions {
        let mut function = user(0, "bad", &[], Type::Void);

        function.body.push(TypedStatement {
            kind: TypedStatementKind::Expression(expression),
            span: Span::new(5, 9),
        });

        let program = TypedProgram {
            functions: vec![function],
            host_functions: vec![host(0, "host", &[], Type::Void)],
        };

        assert!(
            matches!(compile(&program), Err(CodegenError::UnsupportedStatement {
            kind: "expression", span, ..
        }) if span == Span::new(5, 9))
        );
    }
}

#[test]
fn invalid_user_ids_are_rejected() {
    for ids in [vec![1], vec![0, 0], vec![0, 2], vec![1, 0]] {
        let expected = ids
            .iter()
            .enumerate()
            .find(|(expected, actual)| *expected != **actual)
            .unwrap()
            .0;
        let actual = ids[expected];
        let program = TypedProgram {
            functions: ids
                .iter()
                .enumerate()
                .map(|(position, id)| user(*id, &format!("u{position}"), &[], Type::Void))
                .collect(),
            host_functions: vec![],
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidFunctionId {
                function: format!("u{expected}"),
                expected,
                actual,
            })
        );
    }
}

#[test]
fn invalid_host_ids_are_rejected() {
    for ids in [vec![1], vec![0, 0], vec![0, 2], vec![1, 0]] {
        let expected = ids
            .iter()
            .enumerate()
            .find(|(expected, actual)| *expected != **actual)
            .unwrap()
            .0;
        let actual = ids[expected];
        let program = TypedProgram {
            functions: vec![],
            host_functions: ids
                .iter()
                .enumerate()
                .map(|(position, id)| host(*id, &format!("h{position}"), &[], Type::Void))
                .collect(),
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidHostFunctionId {
                function: format!("h{expected}"),
                expected,
                actual,
            })
        );
    }
}

#[test]
fn duplicate_export_names_are_rejected() {
    let program = TypedProgram {
        functions: vec![
            user(0, "duplicate", &[], Type::Void),
            user(1, "duplicate", &[], Type::Void),
        ],
        host_functions: vec![],
    };

    assert_eq!(
        compile(&program),
        Err(CodegenError::DuplicateExportName {
            name: "duplicate".to_owned()
        })
    );
}

#[test]
fn errors_from_later_functions_propagate_without_partial_module_output() {
    let program = TypedProgram {
        functions: vec![
            user(0, "valid", &[], Type::Void),
            user(1, "bad", &[], Type::Number),
        ],
        host_functions: vec![],
    };

    let error = compile(&program).expect_err("reject second function");

    assert_eq!(
        error,
        CodegenError::MissingReturnValue {
            function: "bad".to_owned(),
            return_type: Type::Number
        }
    );
    assert!(error.to_string().contains("bad"));
    assert!(error.to_string().contains("Number"));
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
