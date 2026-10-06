use std::{
    io::Write,
    process::{Command, Stdio},
};

use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    compiler::Compiler,
    host::{HostFunction, HostFunctionId},
    tokens::Span,
    typed_ast::{
        FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedFunction, TypedParam, TypedProgram,
        TypedStatement, TypedStatementKind, VariableId,
    },
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

use super::{CodegenError, checked_u32, compile};

#[derive(Debug, PartialEq)]
enum DecodedInstruction {
    F64Const(f64),
    Return,
    End,
}

#[derive(Debug, Default, PartialEq)]
struct DecodedModule {
    sections: Vec<u8>,
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    bodies: Vec<Vec<DecodedInstruction>>,
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

                let mut instructions = Vec::new();

                while !operators.eof() {
                    instructions.push(match operators.read().expect("instruction") {
                        Operator::F64Const { value } => {
                            DecodedInstruction::F64Const(f64::from_bits(value.bits()))
                        }
                        Operator::Return => DecodedInstruction::Return,
                        Operator::End => DecodedInstruction::End,
                        operator => panic!("unexpected instruction: {operator:?}"),
                    });
                }

                module.bodies.push(instructions);
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
    number_literal("1")
}

fn number_literal(literal: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number(literal.to_owned()),
        ty: Type::Number,
    }
}

fn return_program(value: Option<TypedExpr>, return_type: Type, span: Span) -> TypedProgram {
    let mut function = user(0, "bad", &[], return_type);

    function.body.push(TypedStatement {
        kind: TypedStatementKind::Return(value),
        span,
    });

    TypedProgram {
        functions: vec![function],
        host_functions: vec![],
    }
}

fn run_node(bytes: &[u8], script: &str) {
    let mut child = Command::new("node")
        .args(["-e", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Node.js");

    child
        .stdin
        .take()
        .expect("Node.js stdin")
        .write_all(bytes)
        .expect("write Wasm to Node.js stdin");

    let output = child.wait_with_output().expect("wait for Node.js");

    assert!(
        output.status.success(),
        "Node.js failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
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
fn every_unsupported_statement_kind_is_rejected_with_its_span_even_after_return() {
    for after_return in [false, true] {
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
            ("expression", TypedStatementKind::Expression(number())),
        ];

        for (kind_name, kind) in kinds {
            let mut function = user(0, "bad", &[], Type::Void);

            if after_return {
                function.body.push(TypedStatement {
                    kind: TypedStatementKind::Return(None),
                    span: Span::new(0, 7),
                });
            }

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
fn source_numeric_literals_emit_f64_const_return_and_end() {
    for (literal, expected) in [
        ("42", 42.0),
        ("0.25", 0.25),
        ("0", 0.0),
        ("0.0", 0.0),
        ("00042.00", 42.0),
        ("9007199254740993", 9007199254740992.0),
        ("100000000000000000000", 1e20),
    ] {
        let source = format!("fn value() -> number {{ return {literal}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile numeric return");
        let module = decode(&bytes);

        assert_eq!(module.signatures, vec![(vec![], vec![ValType::F64])]);
        assert_eq!(module.functions, vec![0]);
        assert_eq!(
            module.exports,
            vec![("value".to_owned(), ExternalKind::Func, 0)],
        );
        assert_eq!(
            module.bodies,
            vec![vec![
                DecodedInstruction::F64Const(expected),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ]],
            "literal {literal}",
        );
        assert_eq!(
            Compiler::new().compile_source(&source).unwrap(),
            bytes,
            "deterministic literal {literal}",
        );
    }
}

#[test]
fn valid_underflowing_literals_may_round_to_zero() {
    let literal = format!("0.{}1", "0".repeat(400));
    let source = format!("fn value() -> number {{ return {literal}; }}");
    let program = return_program(
        Some(number_literal(&literal)),
        Type::Number,
        Span::new(12, 34),
    );

    for bytes in [
        compile(&program).expect("compile underflowing typed literal"),
        Compiler::new()
            .compile_source(&source)
            .expect("compile underflowing source literal"),
    ] {
        assert_eq!(
            decode(&bytes).bodies,
            vec![vec![
                DecodedInstruction::F64Const(0.0),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ]],
        );
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
fn bare_void_returns_are_emitted_including_unreachable_returns() {
    let bytes = Compiler::new()
        .compile_source(
            "fn once() -> void { return; }
             fn twice(n: number) -> void { return; return; }",
        )
        .expect("compile bare void returns");

    assert_eq!(
        decode(&bytes).bodies,
        vec![
            vec![DecodedInstruction::Return, DecodedInstruction::End],
            vec![
                DecodedInstruction::Return,
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        ],
    );
}

#[test]
fn all_numeric_returns_are_emitted_in_source_order_even_when_unreachable() {
    let bytes = Compiler::new()
        .compile_source("fn value() -> number { return 42; return 0.25; return 0; }")
        .expect("compile multiple numeric returns");

    assert_eq!(
        decode(&bytes).bodies,
        vec![vec![
            DecodedInstruction::F64Const(42.0),
            DecodedInstruction::Return,
            DecodedInstruction::F64Const(0.25),
            DecodedInstruction::Return,
            DecodedInstruction::F64Const(0.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]],
    );
}

#[test]
fn malformed_typed_numeric_literals_are_rejected_with_the_statement_span() {
    let span = Span::new(12, 34);

    for literal in [
        "", "+1", "-1", "-0", "1e3", "1E3", "1e-3", " 1", "1 ", "\t1", "1\n", "NaN", "nan", "inf",
        "Infinity", "-inf", ".25", "1.", ".", "1.2.3", "1_000", "0x10", "１２", "1\0",
    ] {
        let program = return_program(Some(number_literal(literal)), Type::Number, span);

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: literal.to_owned(),
                span,
            }),
            "literal {literal:?}",
        );
    }
}

#[test]
fn overflowing_numeric_literals_are_rejected_in_source_and_typed_ast() {
    let literal = "9".repeat(400);
    let span = Span::new(12, 34);
    let program = return_program(Some(number_literal(&literal)), Type::Number, span);

    assert_eq!(
        compile(&program),
        Err(CodegenError::NumberLiteralOutOfRange {
            function: "bad".to_owned(),
            literal: literal.clone(),
            span,
        }),
    );

    let source = format!("fn bad() -> number {{ return {literal}; }}");
    let compiler = Compiler::new();
    let program = compiler
        .analyze(&source)
        .expect("analyze overflowing literal");

    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::NumberLiteralOutOfRange {
            function: "bad".to_owned(),
            literal,
            span: program.functions[0].body[0].span,
        }),
    );
}

#[test]
fn every_other_expression_kind_is_rejected_in_returns_with_the_statement_span() {
    let expressions = vec![
        ("string", TypedExprKind::String("text".to_owned())),
        ("bool", TypedExprKind::Bool(true)),
        (
            "identifier",
            TypedExprKind::Identifier {
                id: VariableId(0),
                name: "x".to_owned(),
            },
        ),
        (
            "unary",
            TypedExprKind::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(number()),
            },
        ),
        (
            "binary",
            TypedExprKind::Binary {
                left: Box::new(number()),
                op: BinaryOp::Add,
                right: Box::new(number()),
            },
        ),
        (
            "call",
            TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(0)),
                name: "numeric".to_owned(),
                args: vec![],
            },
        ),
        ("array", TypedExprKind::Array(vec![number()])),
        (
            "index",
            TypedExprKind::Index {
                expr: Box::new(TypedExpr {
                    kind: TypedExprKind::Array(vec![number()]),
                    ty: Type::NumberArray,
                }),
                index: Box::new(number_literal("0")),
            },
        ),
    ];
    let span = Span::new(12, 34);

    for (kind_name, kind) in expressions {
        let mut program = return_program(
            Some(TypedExpr {
                kind,
                ty: Type::Number,
            }),
            Type::Number,
            span,
        );

        program
            .host_functions
            .push(host(0, "numeric", &[], Type::Number));
        program.functions[0].params.push(TypedParam {
            id: VariableId(0),
            name: "x".to_owned(),
            ty: Type::Number,
            span: Span::new(0, 1),
        });

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnsupportedExpression {
                function: "bad".to_owned(),
                kind: kind_name,
                span,
            }),
        );
    }
}

#[test]
fn source_returns_reject_unary_arithmetic_identifiers_calls_and_bools() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
        .unwrap();

    for (expression, return_type, kind) in [
        ("-1", "number", "unary"),
        ("1 + 2", "number", "binary"),
        ("1 - 2", "number", "binary"),
        ("1 * 2", "number", "binary"),
        ("1 / 2", "number", "binary"),
        ("1 % 2", "number", "binary"),
        ("n", "number", "identifier"),
        ("numeric()", "number", "call"),
        ("len([1])", "number", "call"),
        ("true", "bool", "bool"),
        ("!true", "bool", "unary"),
    ] {
        let source = format!("fn bad(n: number) -> {return_type} {{ return {expression}; }}");
        let program = compiler
            .analyze(&source)
            .expect("analyze unsupported return expression");

        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::UnsupportedExpression {
                function: "bad".to_owned(),
                kind,
                span: program.functions[0].body[0].span,
            }),
            "expression {expression}",
        );
    }
}

#[test]
fn invalid_unreachable_returns_are_not_skipped() {
    let span = Span::new(40, 60);
    let overflow = "9".repeat(400);
    let cases = vec![
        (
            Some(number_literal("-1")),
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "-1".to_owned(),
                span,
            },
        ),
        (
            Some(number_literal(&overflow)),
            CodegenError::NumberLiteralOutOfRange {
                function: "bad".to_owned(),
                literal: overflow,
                span,
            },
        ),
        (
            Some(boolean()),
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            Some(TypedExpr {
                kind: TypedExprKind::Unary {
                    op: UnaryOp::Negate,
                    expr: Box::new(number()),
                },
                ty: Type::Number,
            }),
            CodegenError::UnsupportedExpression {
                function: "bad".to_owned(),
                kind: "unary",
                span,
            },
        ),
        (
            None,
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Void,
                span,
            },
        ),
    ];

    for (value, expected) in cases {
        let mut program = return_program(Some(number()), Type::Number, Span::new(12, 34));

        program.functions[0].body.push(TypedStatement {
            kind: TypedStatementKind::Return(value),
            span,
        });

        assert_eq!(compile(&program), Err(expected));
    }
}

#[test]
fn malformed_return_types_and_numeric_expression_types_are_checked() {
    let span = Span::new(12, 34);

    for return_type in [Type::Number, Type::Bool] {
        assert_eq!(
            compile(&return_program(None, return_type, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: return_type,
                found: Type::Void,
                span,
            }),
        );
    }

    for (value, return_type, expected, found) in [
        (boolean(), Type::Number, Type::Number, Type::Bool),
        (number(), Type::Bool, Type::Bool, Type::Number),
        (
            TypedExpr {
                kind: TypedExprKind::Number("42".to_owned()),
                ty: Type::Bool,
            },
            Type::Bool,
            Type::Number,
            Type::Bool,
        ),
    ] {
        assert_eq!(
            compile(&return_program(Some(value), return_type, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected,
                found,
                span,
            }),
        );
    }
}

#[test]
fn void_returns_reject_any_value_including_void_typed_expressions() {
    let span = Span::new(12, 34);

    for value in [
        number(),
        boolean(),
        number_literal("NaN"),
        TypedExpr {
            kind: TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(0)),
                name: "noop".to_owned(),
                args: vec![],
            },
            ty: Type::Void,
        },
    ] {
        let mut program = return_program(Some(value), Type::Void, span);

        program
            .host_functions
            .push(host(0, "noop", &[], Type::Void));

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnexpectedReturnValue {
                function: "bad".to_owned(),
                span,
            }),
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_numeric_literal_returns() {
    let bytes = Compiler::new()
        .compile_source(
            "fn answer() -> number { return 42; return 0; }
             fn decimal() -> number { return 0.25; }",
        )
        .expect("compile runtime numeric returns");

    decode(&bytes);
    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));

        assert.equal(instance.exports.answer(), 42);
        assert.equal(instance.exports.decimal(), 0.25);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_numeric_exports_with_unused_provider_imports() {
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
        .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn answer(n: number, flag: bool) -> number { return 42; }
             fn decimal(flag: bool, n: number) -> number { return 0.25; }
             fn noop() -> void { return; }",
        )
        .expect("compile runtime imports and numeric exports");
    let module = decode(&bytes);

    assert_eq!(
        module.exports,
        vec![
            ("answer".to_owned(), ExternalKind::Func, 2),
            ("decimal".to_owned(), ExternalKind::Func, 3),
            ("noop".to_owned(), ExternalKind::Func, 4),
        ],
    );
    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        let calls = 0;
        const imports = {
            env: { numeric: (n, flag) => { calls += 1; return n + flag; } },
            provider: { ready: () => { calls += 1; return 1; } },
        };
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), imports);

        assert.equal(instance.exports.answer(100, 1), 42);
        assert.equal(instance.exports.decimal(0, 100), 0.25);
        assert.equal(instance.exports.noop(), undefined);
        assert.equal(calls, 0);
        "#,
    );
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
