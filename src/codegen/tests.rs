use std::{
    io::Write,
    process::{Command, Stdio},
};

use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    compiler::{CompileError, Compiler, CompilerOptions},
    host::{HostFunction, HostFunctionId},
    optimization::OptimizationLevel,
    tokens::Span,
    typed_ast::{
        FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedFunction, TypedParam, TypedProgram,
        TypedStatement, TypedStatementKind, VariableId,
    },
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

use super::{CodegenError, checked_u32, compile, locals::LocalLayout};

#[derive(Debug, PartialEq)]
enum DecodedInstruction {
    F64Const(f64),
    I32Const(i32),
    LocalGet(u32),
    LocalSet(u32),
    Drop,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Neg,
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
    locals: Vec<Vec<ValType>>,
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
                let mut locals = Vec::new();

                for local in body.get_locals_reader().expect("locals") {
                    let (count, ty) = local.expect("local declaration");

                    locals.extend(std::iter::repeat_n(ty, count as usize));
                }

                module.locals.push(locals);

                let mut operators = body.get_operators_reader().expect("operators");

                let mut instructions = Vec::new();

                while !operators.eof() {
                    instructions.push(match operators.read().expect("instruction") {
                        Operator::F64Const { value } => {
                            DecodedInstruction::F64Const(f64::from_bits(value.bits()))
                        }
                        Operator::I32Const { value } => DecodedInstruction::I32Const(value),
                        Operator::LocalGet { local_index } => {
                            DecodedInstruction::LocalGet(local_index)
                        }
                        Operator::LocalSet { local_index } => {
                            DecodedInstruction::LocalSet(local_index)
                        }
                        Operator::Drop => DecodedInstruction::Drop,
                        Operator::F64Add => DecodedInstruction::F64Add,
                        Operator::F64Sub => DecodedInstruction::F64Sub,
                        Operator::F64Mul => DecodedInstruction::F64Mul,
                        Operator::F64Div => DecodedInstruction::F64Div,
                        Operator::F64Neg => DecodedInstruction::F64Neg,
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

fn unary(op: UnaryOp, expr: TypedExpr) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Unary {
            op,
            expr: Box::new(expr),
        },
        ty: Type::Number,
    }
}

fn binary(left: TypedExpr, op: BinaryOp, right: TypedExpr) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
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

fn identifier(id: usize, name: &str, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Identifier {
            id: VariableId(id),
            name: name.to_owned(),
        },
        ty,
    }
}

fn declaration(id: usize, ty: Type, value: TypedExpr, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::Let {
            id: VariableId(id),
            name: "x".to_owned(),
            ty,
            value,
        },
        span,
    }
}

fn assignment(target: TypedExpr, value: TypedExpr, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::Assignment { target, value },
        span,
    }
}

fn local_program(body: Vec<TypedStatement>) -> TypedProgram {
    let mut function = user(0, "bad", &[], Type::Void);

    function.body = body;

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
fn host_call_statements_are_rejected_including_void_calls() {
    let expressions = [Type::Number, Type::Bool, Type::Void].map(|ty| TypedExpr {
        kind: TypedExprKind::Call {
            func: FunctionRef::Host(HostFunctionId(0)),
            name: "host".to_owned(),
            args: vec![],
        },
        ty,
    });

    for expression in expressions {
        let expression_type = expression.ty;
        let mut function = user(0, "bad", &[], Type::Void);

        function.body.push(TypedStatement {
            kind: TypedStatementKind::Expression(expression),
            span: Span::new(5, 9),
        });

        let program = TypedProgram {
            functions: vec![function],
            host_functions: vec![host(0, "host", &[], expression_type)],
        };

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnsupportedExpression {
                function: "bad".to_owned(),
                kind: "call",
                span: Span::new(5, 9),
            }),
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
fn source_arithmetic_emits_operands_before_each_f64_opcode() {
    use DecodedInstruction::*;

    for (expression, instruction) in [
        ("8 + 2", F64Add),
        ("8 - 2", F64Sub),
        ("8 * 2", F64Mul),
        ("8 / 2", F64Div),
    ] {
        let source = format!("fn value() -> number {{ return {expression}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile arithmetic return");
        let module = decode(&bytes);

        assert_eq!(module.signatures, vec![(vec![], vec![ValType::F64])]);
        assert_eq!(module.functions, vec![0]);
        assert_eq!(
            module.exports,
            vec![("value".to_owned(), ExternalKind::Func, 0)],
        );
        assert_eq!(
            module.bodies,
            vec![vec![F64Const(8.0), F64Const(2.0), instruction, Return, End]],
            "expression {expression}",
        );
    }
}

#[test]
fn source_unary_negation_emits_f64_neg_without_folding() {
    use DecodedInstruction::*;

    for (expression, instructions) in [
        ("-42", vec![F64Const(42.0), F64Neg]),
        ("-0", vec![F64Const(0.0), F64Neg]),
        ("-(-2)", vec![F64Const(2.0), F64Neg, F64Neg]),
        (
            "-(1 + 2)",
            vec![F64Const(1.0), F64Const(2.0), F64Add, F64Neg],
        ),
    ] {
        let source = format!("fn value() -> number {{ return {expression}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile unary negation");
        let mut expected = instructions;

        expected.extend([Return, End]);

        assert_eq!(decode(&bytes).bodies, vec![expected], "{expression}");
    }
}

#[test]
fn nested_arithmetic_preserves_precedence_associativity_and_operand_order() {
    use DecodedInstruction::*;

    for (expression, instructions) in [
        (
            "1 + 2 * 3",
            vec![F64Const(1.0), F64Const(2.0), F64Const(3.0), F64Mul, F64Add],
        ),
        (
            "(1 + 2) * 3",
            vec![F64Const(1.0), F64Const(2.0), F64Add, F64Const(3.0), F64Mul],
        ),
        (
            "10 - 3 - 2",
            vec![F64Const(10.0), F64Const(3.0), F64Sub, F64Const(2.0), F64Sub],
        ),
        (
            "20 / 2 / 5",
            vec![F64Const(20.0), F64Const(2.0), F64Div, F64Const(5.0), F64Div],
        ),
        (
            "20 / (2 + 3)",
            vec![F64Const(20.0), F64Const(2.0), F64Const(3.0), F64Add, F64Div],
        ),
        (
            "1 / (2 / 4)",
            vec![F64Const(1.0), F64Const(2.0), F64Const(4.0), F64Div, F64Div],
        ),
        (
            "10 - (3 - 2)",
            vec![F64Const(10.0), F64Const(3.0), F64Const(2.0), F64Sub, F64Sub],
        ),
        (
            "(8 - 2) / (1 + 2)",
            vec![
                F64Const(8.0),
                F64Const(2.0),
                F64Sub,
                F64Const(1.0),
                F64Const(2.0),
                F64Add,
                F64Div,
            ],
        ),
    ] {
        let source = format!("fn value() -> number {{ return {expression}; }}");
        let bytes = Compiler::new()
            .compile_source(&source)
            .expect("compile nested arithmetic");
        let mut expected = instructions;

        expected.extend([Return, End]);

        assert_eq!(decode(&bytes).bodies, vec![expected], "{expression}");
    }
}

#[test]
fn arithmetic_preserves_import_indices_and_is_not_folded_at_any_optimization_level() {
    use DecodedInstruction::*;

    let mut expected_bytes = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
            .unwrap();
        compiler
            .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
            .unwrap();

        let source = "fn value(n: number, flag: bool) -> number { return -(1 + 2 * 3); }
                      fn quotient() -> number { return 1 / (2 / 4); }";
        let program = compiler.analyze(source).expect("analyze arithmetic");
        let original = program.clone();
        let bytes = compiler.compile(&program).expect("compile arithmetic");
        let module = decode(&bytes);

        assert_eq!(program, original);
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
        assert_eq!(
            module.signatures,
            vec![
                (vec![], vec![ValType::F64]),
                (vec![], vec![ValType::I32]),
                (vec![ValType::F64, ValType::I32], vec![ValType::F64]),
            ],
        );
        assert_eq!(
            module.imports,
            vec![
                ("env".to_owned(), "numeric".to_owned(), 0),
                ("provider".to_owned(), "ready".to_owned(), 1),
            ],
        );
        assert_eq!(module.functions, vec![2, 0]);
        assert_eq!(
            module.exports,
            vec![
                ("value".to_owned(), ExternalKind::Func, 2),
                ("quotient".to_owned(), ExternalKind::Func, 3),
            ],
        );
        assert_eq!(
            module.bodies,
            vec![
                vec![
                    F64Const(1.0),
                    F64Const(2.0),
                    F64Const(3.0),
                    F64Mul,
                    F64Add,
                    F64Neg,
                    Return,
                    End,
                ],
                vec![
                    F64Const(1.0),
                    F64Const(2.0),
                    F64Const(4.0),
                    F64Div,
                    F64Div,
                    Return,
                    End
                ],
            ],
            "optimization level {level:?}",
        );

        if let Some(expected) = &expected_bytes {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            expected_bytes = Some(bytes);
        }
    }
}

#[test]
fn arithmetic_requires_numeric_roots_and_operands_even_in_nested_typed_ast() {
    let span = Span::new(12, 34);
    let mut roots = vec![unary(UnaryOp::Negate, number())];

    for op in [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
    ] {
        roots.push(binary(number(), op, number()));
    }

    for mut value in roots {
        value.ty = Type::Bool;

        assert_eq!(
            compile(&return_program(Some(value), Type::Bool, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            }),
        );
    }

    for found in [Type::Bool, Type::String, Type::NumberArray, Type::Void] {
        let mut operand = number();

        operand.ty = found;

        let mut expressions = vec![unary(UnaryOp::Negate, operand.clone())];

        for op in [
            BinaryOp::Add,
            BinaryOp::Subtract,
            BinaryOp::Multiply,
            BinaryOp::Divide,
        ] {
            expressions.push(binary(operand.clone(), op, number()));
            expressions.push(binary(number(), op, operand.clone()));
        }

        for expression in expressions {
            for value in [
                expression.clone(),
                binary(expression.clone(), BinaryOp::Add, number()),
                binary(number(), BinaryOp::Divide, expression.clone()),
                unary(UnaryOp::Negate, expression),
            ] {
                assert_eq!(
                    compile(&return_program(Some(value), Type::Number, span)),
                    Err(CodegenError::TypeMismatch {
                        function: "bad".to_owned(),
                        expected: Type::Number,
                        found,
                        span,
                    }),
                );
            }
        }
    }

    let mut nested_root = binary(number(), BinaryOp::Multiply, number());

    nested_root.ty = Type::Bool;

    for value in [
        unary(UnaryOp::Negate, nested_root.clone()),
        binary(nested_root.clone(), BinaryOp::Subtract, number()),
        binary(number(), BinaryOp::Divide, nested_root),
    ] {
        assert_eq!(
            compile(&return_program(Some(value), Type::Number, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            }),
        );
    }
}

#[test]
fn nested_numeric_literal_errors_preserve_the_enclosing_statement_span() {
    let span = Span::new(12, 34);
    let overflow = "9".repeat(400);

    for literal in ["-1", "NaN", "1e3", "1.", overflow.as_str()] {
        for value in [
            unary(UnaryOp::Negate, number_literal(literal)),
            binary(
                unary(UnaryOp::Negate, number_literal(literal)),
                BinaryOp::Multiply,
                number(),
            ),
            binary(
                number(),
                BinaryOp::Divide,
                binary(number(), BinaryOp::Add, number_literal(literal)),
            ),
        ] {
            let expected = if literal == overflow {
                CodegenError::NumberLiteralOutOfRange {
                    function: "bad".to_owned(),
                    literal: literal.to_owned(),
                    span,
                }
            } else {
                CodegenError::InvalidNumberLiteral {
                    function: "bad".to_owned(),
                    literal: literal.to_owned(),
                    span,
                }
            };

            assert_eq!(
                compile(&return_program(Some(value), Type::Number, span)),
                Err(expected),
                "literal {literal:?}",
            );
        }
    }

    for expression in [
        format!("-({overflow})"),
        format!("{overflow} + 1"),
        format!("1 / (2 + {overflow})"),
    ] {
        let source = format!("fn bad() -> number {{ return {expression}; }}");
        let compiler = Compiler::new();
        let program = compiler.analyze(&source).expect("analyze nested overflow");

        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::NumberLiteralOutOfRange {
                function: "bad".to_owned(),
                literal: overflow.clone(),
                span: program.functions[0].body[0].span,
            }),
        );
    }
}

#[test]
fn unsupported_operators_are_rejected_before_their_root_or_operand_type_checks() {
    let span = Span::new(12, 34);
    let mut expressions = vec![("!", unary(UnaryOp::Not, boolean()))];

    for (op, label) in [
        (BinaryOp::Remainder, "%"),
        (BinaryOp::Equal, "=="),
        (BinaryOp::NotEqual, "!="),
        (BinaryOp::LessThan, "<"),
        (BinaryOp::LessThanOrEqual, "<="),
        (BinaryOp::GreaterThan, ">"),
        (BinaryOp::GreaterThanOrEqual, ">="),
        (BinaryOp::And, "&&"),
        (BinaryOp::Or, "||"),
    ] {
        expressions.push((label, binary(boolean(), op, boolean())));
    }

    for (operator, expression) in expressions {
        for ty in [Type::Number, Type::Bool] {
            let mut value = expression.clone();

            value.ty = ty;

            assert_eq!(
                compile(&return_program(Some(value), ty, span)),
                Err(CodegenError::UnsupportedOperator {
                    function: "bad".to_owned(),
                    operator,
                    span,
                }),
                "operator {operator}, root type {ty:?}",
            );
        }
    }
}

#[test]
fn source_returns_reject_every_nonarithmetic_operator_with_its_label() {
    for (expression, return_type, operator) in [
        ("1 % 2", "number", "%"),
        ("1 == 2", "bool", "=="),
        ("1 != 2", "bool", "!="),
        ("1 < 2", "bool", "<"),
        ("1 <= 2", "bool", "<="),
        ("1 > 2", "bool", ">"),
        ("1 >= 2", "bool", ">="),
        ("true && false", "bool", "&&"),
        ("true || false", "bool", "||"),
        ("!true", "bool", "!"),
        ("flag && true", "bool", "&&"),
        ("flag || false", "bool", "||"),
        ("flag == true", "bool", "=="),
        ("!flag", "bool", "!"),
    ] {
        let source = format!("fn bad(flag: bool) -> {return_type} {{ return {expression}; }}");
        let compiler = Compiler::new();
        let program = compiler
            .analyze(&source)
            .expect("analyze unsupported operator");

        assert_eq!(
            compiler.compile(&program),
            Err(CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator,
                span: program.functions[0].body[0].span,
            }),
            "expression {expression}",
        );
    }
}

#[test]
fn nested_unsupported_expressions_and_operators_propagate_in_left_to_right_order() {
    let span = Span::new(12, 34);
    let cases = vec![
        (
            unary(UnaryOp::Not, number()),
            CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator: "!",
                span,
            },
        ),
        (
            binary(number(), BinaryOp::Remainder, number()),
            CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator: "%",
                span,
            },
        ),
        (
            TypedExpr {
                kind: TypedExprKind::Identifier {
                    id: VariableId(99),
                    name: "x".to_owned(),
                },
                ty: Type::Number,
            },
            CodegenError::UnknownVariable {
                function: "bad".to_owned(),
                id: VariableId(99),
                span,
            },
        ),
        (
            TypedExpr {
                kind: TypedExprKind::Call {
                    func: FunctionRef::Host(HostFunctionId(0)),
                    name: "numeric".to_owned(),
                    args: vec![],
                },
                ty: Type::Number,
            },
            CodegenError::UnsupportedExpression {
                function: "bad".to_owned(),
                kind: "call",
                span,
            },
        ),
    ];

    for (expression, expected) in cases {
        for value in [
            unary(UnaryOp::Negate, expression.clone()),
            binary(expression.clone(), BinaryOp::Add, number()),
            binary(number(), BinaryOp::Subtract, expression.clone()),
            binary(
                binary(expression, BinaryOp::Multiply, number()),
                BinaryOp::Divide,
                number_literal("NaN"),
            ),
        ] {
            let mut program = return_program(Some(value), Type::Number, span);

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
                compile(&program).expect_err("reject nested expression"),
                expected,
            );
        }
    }
}

#[test]
fn source_arithmetic_with_wrong_types_is_still_rejected_by_semantic_analysis() {
    for expression in [
        "-true",
        "1 + true",
        "true - 1",
        "1 * false",
        "false / 2",
        "1 + (2 * true)",
        "1 + \"text\"",
        "[1] / 2",
    ] {
        let source = format!("fn bad() -> number {{ return {expression}; }}");

        assert!(
            matches!(
                Compiler::new().compile_source(&source),
                Err(CompileError::Semantic(_))
            ),
            "expression {expression}",
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
fn unsupported_expression_kinds_are_rejected_with_the_statement_span() {
    let expressions = vec![
        ("string", TypedExprKind::String("text".to_owned())),
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
        let value = TypedExpr {
            kind,
            ty: Type::Number,
        };

        for is_statement in [false, true] {
            let mut program = if is_statement {
                let mut value = value.clone();

                value.ty = match kind_name {
                    "string" => Type::String,
                    "array" => Type::NumberArray,
                    _ => Type::Number,
                };

                local_program(vec![TypedStatement {
                    kind: TypedStatementKind::Expression(value),
                    span,
                }])
            } else {
                return_program(Some(value.clone()), Type::Number, span)
            };

            program
                .host_functions
                .push(host(0, "numeric", &[], Type::Number));

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
}

#[test]
fn source_returns_reject_calls() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
        .unwrap();

    for (expression, return_type, kind) in [
        ("numeric()", "number", "call"),
        ("len([1])", "number", "call"),
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
                    op: UnaryOp::Not,
                    expr: Box::new(number()),
                },
                ty: Type::Number,
            }),
            CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator: "!",
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
#[ignore = "requires Node.js"]
fn node_executes_nested_arithmetic_with_unused_provider_imports() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn precedence(n: number, flag: bool) -> number { return 1 + 2 * 3; }
             fn parentheses() -> number { return (1 + 2) * 3; }
             fn negate() -> number { return -(1 + 2); }
             fn subtract() -> number { return 10 - 3 - 2; }
             fn divide() -> number { return 20 / (2 + 3); }
             fn rightdivide() -> number { return 1 / (2 / 4); }
             fn rightsubtract() -> number { return 10 - (3 - 2); }
             fn noop() -> void {}",
        )
        .expect("compile runtime nested arithmetic");
    let module = decode(&bytes);

    assert_eq!(
        module.exports[0],
        ("precedence".to_owned(), ExternalKind::Func, 2)
    );
    assert_eq!(
        module.exports[7],
        ("noop".to_owned(), ExternalKind::Func, 9)
    );

    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        let calls = 0;
        const imports = {
            env: { numeric: () => { calls += 1; return 99; } },
            provider: { ready: () => { calls += 1; return 1; } },
        };
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), imports);

        assert.equal(instance.exports.precedence(100, 1), 7);
        assert.equal(instance.exports.parentheses(), 9);
        assert.equal(instance.exports.negate(), -3);
        assert.equal(instance.exports.subtract(), 5);
        assert.equal(instance.exports.divide(), 4);
        assert.equal(instance.exports.rightdivide(), 2);
        assert.equal(instance.exports.rightsubtract(), 9);
        assert.equal(instance.exports.noop(), undefined);
        assert.equal(calls, 0);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_arithmetic_with_ieee754_rounding_and_special_results() {
    let finite = format!("1{}", "0".repeat(200));
    let source = format!(
        "fn decimal() -> number {{ return 0.1 + 0.2; }}
         fn fraction() -> number {{ return 1 / 10; }}
         fn rounded() -> number {{ return 9007199254740993 + 1; }}
         fn nan() -> number {{ return 0 / 0; }}
         fn positiveinfinity() -> number {{ return 1 / 0; }}
         fn negativeinfinity() -> number {{ return -1 / 0; }}
         fn negativezero() -> number {{ return -0; }}
         fn dividedzero() -> number {{ return 0 / -1; }}
         fn positivezero() -> number {{ return -0 * -1; }}
         fn overflow() -> number {{ return {finite} * {finite}; }}",
    );
    let bytes = Compiler::new()
        .compile_source(&source)
        .expect("compile runtime IEEE 754 arithmetic");
    let module = decode(&bytes);

    assert_eq!(
        module.bodies[9],
        vec![
            DecodedInstruction::F64Const(1e200),
            DecodedInstruction::F64Const(1e200),
            DecodedInstruction::F64Mul,
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ],
    );

    run_node(
        &bytes,
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes));

        assert.equal(instance.exports.decimal(), 0.30000000000000004);
        assert.equal(instance.exports.fraction(), 0.1);
        assert.equal(instance.exports.rounded(), 9007199254740992);
        assert.ok(Number.isNaN(instance.exports.nan()));
        assert.equal(instance.exports.positiveinfinity(), Infinity);
        assert.equal(instance.exports.negativeinfinity(), -Infinity);
        assert.ok(Object.is(instance.exports.negativezero(), -0));
        assert.ok(Object.is(instance.exports.dividedzero(), -0));
        assert.ok(Object.is(instance.exports.positivezero(), 0));
        assert.equal(instance.exports.overflow(), Infinity);
        "#,
    );
}

#[test]
fn scalar_parameters_locals_and_mutations_emit_dense_slots_at_all_optimization_levels() {
    use DecodedInstruction::*;

    let source = "fn update(n: number, flag: bool) -> number {
                      let value: number = n * 2 + 1;
                      let ready: bool = flag;
                      n = value - 3;
                      ready = false;
                      value = -n / 2;
                      return value;
                  }";
    let mut expected_bytes = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });
        let program = compiler.analyze(source).expect("analyze scalar mutations");
        let original = program.clone();
        let bytes = compiler
            .compile(&program)
            .expect("compile scalar mutations");
        let module = decode(&bytes);

        assert_eq!(program, original);
        assert_eq!(compiler.compile_source(source).unwrap(), bytes);
        assert_eq!(
            module.signatures,
            vec![(vec![ValType::F64, ValType::I32], vec![ValType::F64])],
        );
        assert_eq!(module.locals, vec![vec![ValType::F64, ValType::I32]]);
        assert_eq!(
            module.bodies,
            vec![vec![
                LocalGet(0),
                F64Const(2.0),
                F64Mul,
                F64Const(1.0),
                F64Add,
                LocalSet(2),
                LocalGet(1),
                LocalSet(3),
                LocalGet(2),
                F64Const(3.0),
                F64Sub,
                LocalSet(0),
                I32Const(0),
                LocalSet(3),
                LocalGet(0),
                F64Neg,
                F64Const(2.0),
                F64Div,
                LocalSet(2),
                LocalGet(2),
                Return,
                End,
            ]],
            "optimization level {level:?}",
        );

        if let Some(expected) = &expected_bytes {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            expected_bytes = Some(bytes);
        }
    }
}

#[test]
fn bool_literals_variables_and_returns_preserve_mixed_local_order_and_reset_function_ids() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn first(flag: bool, n: number) -> bool {
                 let value: number = n;
                 let ready: bool = true;
                 let other: number = value;
                 let copy: bool = flag;
                 ready = copy;
                 flag = false;
                 return ready;
             }
             fn second(flag: bool) -> bool {
                 let ready: bool = flag;
                 ready = false;
                 return ready;
             }
             fn yes() -> bool { return true; }
             fn no() -> bool { return false; }",
        )
        .expect("compile bool bindings and returns");
    let module = decode(&bytes);

    assert_eq!(
        module.locals,
        vec![
            vec![ValType::F64, ValType::I32, ValType::F64, ValType::I32],
            vec![ValType::I32],
            vec![],
            vec![],
        ],
    );
    assert_eq!(
        module.bodies,
        vec![
            vec![
                LocalGet(1),
                LocalSet(2),
                I32Const(1),
                LocalSet(3),
                LocalGet(2),
                LocalSet(4),
                LocalGet(0),
                LocalSet(5),
                LocalGet(5),
                LocalSet(3),
                I32Const(0),
                LocalSet(0),
                LocalGet(3),
                Return,
                End,
            ],
            vec![
                LocalGet(0),
                LocalSet(1),
                I32Const(0),
                LocalSet(1),
                LocalGet(1),
                Return,
                End,
            ],
            vec![I32Const(1), Return, End],
            vec![I32Const(0), Return, End],
        ],
    );
}

#[test]
fn scalar_expression_statements_drop_values_even_after_return() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn discard(n: number, flag: bool) -> void {
                 n + 1;
                 true;
                 flag;
                 return;
                 let value: number = n;
                 value = 2;
                 value;
                 false;
             }",
        )
        .expect("compile discarded scalar expressions");
    let module = decode(&bytes);

    assert_eq!(module.locals, vec![vec![ValType::F64]]);
    assert_eq!(
        module.bodies,
        vec![vec![
            LocalGet(0),
            F64Const(1.0),
            F64Add,
            Drop,
            I32Const(1),
            Drop,
            LocalGet(1),
            Drop,
            Return,
            LocalGet(0),
            LocalSet(2),
            F64Const(2.0),
            LocalSet(2),
            LocalGet(2),
            Drop,
            I32Const(0),
            Drop,
            End,
        ]],
    );
}

#[test]
fn sparse_variable_ids_and_diagnostic_names_do_not_determine_wasm_indices() {
    use DecodedInstruction::*;

    let span = Span::new(12, 34);
    let mut function = user(0, "sparse", &[Type::Number, Type::Bool], Type::Number);

    function.params[0].id = VariableId(90);
    function.params[1].id = VariableId(3);
    function.body = vec![
        declaration(
            usize::MAX,
            Type::Bool,
            identifier(3, "renamed", Type::Bool),
            span,
        ),
        declaration(
            7,
            Type::Number,
            identifier(90, "not_p0", Type::Number),
            span,
        ),
        assignment(
            identifier(90, "renamed", Type::Number),
            identifier(7, "not_x", Type::Number),
            span,
        ),
        TypedStatement {
            kind: TypedStatementKind::Return(Some(identifier(7, "p0", Type::Number))),
            span,
        },
    ];

    let layout = LocalLayout::new(&function).expect("layout sparse variable IDs");

    assert_eq!(
        layout.types,
        vec![wasm_encoder::ValType::I32, wasm_encoder::ValType::F64]
    );

    for (id, index, ty) in [
        (90, 0, Type::Number),
        (3, 1, Type::Bool),
        (usize::MAX, 2, Type::Bool),
        (7, 3, Type::Number),
    ] {
        let local = layout.get(VariableId(id)).expect("mapped variable");
        let copied = local;

        assert_eq!((local.index, local.ty), (index, ty));
        assert_eq!((copied.index, copied.ty), (index, ty));
    }

    assert!(layout.get(VariableId(0)).is_none());

    let module = decode(
        &compile(&TypedProgram {
            functions: vec![function],
            host_functions: vec![],
        })
        .expect("compile sparse variable IDs"),
    );

    assert_eq!(module.locals, vec![vec![ValType::I32, ValType::F64]]);
    assert_eq!(
        module.bodies,
        vec![vec![
            LocalGet(1),
            LocalSet(2),
            LocalGet(0),
            LocalSet(3),
            LocalGet(3),
            LocalSet(0),
            LocalGet(3),
            Return,
            End,
        ]]
    );
}

#[test]
fn local_layout_collects_nested_shadow_bindings_in_preorder_without_emitting_control_flow() {
    fn check_binding(layout: &LocalLayout, expr: &TypedExpr, index: u32, ty: Type) {
        let TypedExprKind::Identifier { id, .. } = &expr.kind else {
            panic!("expected resolved identifier");
        };
        let local = layout.get(*id).expect("shadow binding");

        assert_eq!((local.index, local.ty), (index, ty));
    }

    fn collect(block: &[TypedStatement], ids: &mut Vec<(VariableId, Type)>) {
        for statement in block {
            match &statement.kind {
                TypedStatementKind::Let { id, ty, .. } => ids.push((*id, *ty)),
                TypedStatementKind::If { then, else_, .. } => {
                    collect(then, ids);

                    if let Some(else_) = else_ {
                        collect(else_, ids);
                    }
                }
                TypedStatementKind::While { body, .. } => collect(body, ids),
                _ => {}
            }
        }
    }

    let program = Compiler::new()
        .analyze(
            "fn shadow(value: number) -> number {
             let copy: number = value;
             if true {
                 let value: bool = true;
                 if value {
                     let copy: number = 2;
                     copy;
                 } else {
                     let copy: bool = false;
                     copy;
                 }
                 value;
             } else {
                 let value: number = 3;
                 value;
             }
             while false {
                 let value: bool = false;
                 value;
             }
             let tail: number = copy;
             return value;
         }",
        )
        .expect("analyze nested shadow bindings");
    let function = &program.functions[0];
    let layout = LocalLayout::new(function).expect("layout nested shadow bindings");
    let mut ids = vec![(function.params[0].id, Type::Number)];

    collect(&function.body, &mut ids);

    assert_eq!(
        layout.types,
        vec![
            wasm_encoder::ValType::F64,
            wasm_encoder::ValType::I32,
            wasm_encoder::ValType::F64,
            wasm_encoder::ValType::I32,
            wasm_encoder::ValType::F64,
            wasm_encoder::ValType::I32,
            wasm_encoder::ValType::F64,
        ]
    );
    assert_eq!(ids.len(), 8);

    for (index, (id, ty)) in ids.into_iter().enumerate() {
        let local = layout.get(id).expect("preorder local slot");

        assert_eq!((local.index, local.ty), (index as u32, ty));
    }

    let TypedStatementKind::If {
        then,
        else_: Some(else_),
        ..
    } = &function.body[1].kind
    else {
        panic!("expected outer if");
    };
    let TypedStatementKind::If {
        cond,
        then: inner_then,
        else_: Some(inner_else),
    } = &then[1].kind
    else {
        panic!("expected nested if");
    };

    check_binding(&layout, cond, 2, Type::Bool);

    for (statement, index, ty) in [
        (&inner_then[1], 3, Type::Number),
        (&inner_else[1], 4, Type::Bool),
        (&then[2], 2, Type::Bool),
        (&else_[1], 5, Type::Number),
    ] {
        let TypedStatementKind::Expression(expr) = &statement.kind else {
            panic!("expected shadow expression");
        };

        check_binding(&layout, expr, index, ty);
    }

    let TypedStatementKind::While { body, .. } = &function.body[2].kind else {
        panic!("expected while");
    };
    let TypedStatementKind::Expression(expr) = &body[1].kind else {
        panic!("expected loop shadow expression");
    };

    check_binding(&layout, expr, 6, Type::Bool);

    let TypedStatementKind::Let { value, .. } = &function.body[3].kind else {
        panic!("expected trailing declaration");
    };

    check_binding(&layout, value, 1, Type::Number);

    let TypedStatementKind::Return(Some(value)) = &function.body[4].kind else {
        panic!("expected restored parameter return");
    };

    check_binding(&layout, value, 0, Type::Number);
}

#[test]
fn duplicate_variable_ids_are_rejected_at_the_repeated_parameter_or_declaration() {
    let first = Span::new(1, 4);
    let repeated = Span::new(12, 34);
    let mut parameters = user(0, "bad", &[Type::Number, Type::Bool], Type::Void);

    parameters.params[0].id = VariableId(77);
    parameters.params[1].id = VariableId(77);
    parameters.params[1].span = repeated;

    let mut parameter_local = user(0, "bad", &[Type::Number], Type::Void);

    parameter_local.params[0].id = VariableId(77);
    parameter_local
        .body
        .push(declaration(77, Type::Bool, boolean(), repeated));

    let mut locals = user(0, "bad", &[], Type::Void);

    locals.body = vec![
        declaration(77, Type::Number, number(), first),
        TypedStatement {
            kind: TypedStatementKind::Return(None),
            span: first,
        },
        declaration(77, Type::Bool, boolean(), repeated),
    ];

    let mut nested = user(0, "bad", &[], Type::Void);

    nested.body = vec![TypedStatement {
        kind: TypedStatementKind::If {
            cond: Box::new(boolean()),
            then: vec![declaration(77, Type::Number, number(), first)],
            else_: Some(vec![TypedStatement {
                kind: TypedStatementKind::While {
                    cond: Box::new(boolean()),
                    body: vec![declaration(77, Type::Bool, boolean(), repeated)],
                },
                span: first,
            }]),
        },
        span: first,
    }];

    for function in [parameters, parameter_local, locals, nested] {
        let expected = CodegenError::DuplicateVariableId {
            function: "bad".to_owned(),
            id: VariableId(77),
            span: repeated,
        };

        assert_eq!(LocalLayout::new(&function).err(), Some(expected));
        assert_eq!(
            compile(&TypedProgram {
                functions: vec![function],
                host_functions: vec![],
            }),
            Err(CodegenError::DuplicateVariableId {
                function: "bad".to_owned(),
                id: VariableId(77),
                span: repeated,
            })
        );
    }
}

#[test]
fn unknown_ids_are_rejected_for_reads_and_assignments_without_name_or_cross_function_lookup() {
    let span = Span::new(12, 34);

    for ty in [Type::Number, Type::Bool, Type::String, Type::Void] {
        for kind in [
            TypedStatementKind::Expression(identifier(99, "p0", ty)),
            TypedStatementKind::Assignment {
                target: identifier(99, "p0", ty),
                value: number_literal("NaN"),
            },
        ] {
            let mut previous = user(0, "previous", &[], Type::Void);

            previous
                .body
                .push(declaration(99, Type::Number, number(), Span::new(0, 1)));

            let mut function = user(1, "bad", &[Type::Number], Type::Void);

            function.body.push(TypedStatement { kind, span });

            assert_eq!(
                compile(&TypedProgram {
                    functions: vec![previous, function],
                    host_functions: vec![],
                }),
                Err(CodegenError::UnknownVariable {
                    function: "bad".to_owned(),
                    id: VariableId(99),
                    span,
                })
            );
        }
    }
}

#[test]
fn forward_reads_self_initializers_and_assignments_before_declaration_are_uninitialized() {
    let span = Span::new(12, 34);
    let later = Span::new(40, 60);
    let cases = vec![
        vec![
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(7, "x", Type::Number)),
                span,
            },
            declaration(7, Type::Number, number(), later),
        ],
        vec![
            declaration(8, Type::Number, identifier(7, "x", Type::Number), span),
            declaration(7, Type::Number, number(), later),
        ],
        vec![declaration(
            7,
            Type::Number,
            identifier(7, "renamed", Type::Number),
            span,
        )],
        vec![
            assignment(
                identifier(7, "x", Type::Number),
                number_literal("NaN"),
                span,
            ),
            declaration(7, Type::Number, number(), later),
        ],
        vec![
            TypedStatement {
                kind: TypedStatementKind::Return(Some(identifier(7, "x", Type::Number))),
                span,
            },
            declaration(7, Type::Number, number(), later),
        ],
        vec![declaration(
            7,
            Type::Bool,
            identifier(7, "x", Type::Bool),
            span,
        )],
        vec![
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(7, "x", Type::Bool)),
                span,
            },
            declaration(7, Type::Bool, boolean(), later),
        ],
        vec![
            assignment(identifier(7, "x", Type::Bool), boolean(), span),
            declaration(7, Type::Bool, boolean(), later),
        ],
    ];

    for body in cases {
        for after_return in [false, true] {
            let mut program = local_program(body.clone());

            if matches!(body[0].kind, TypedStatementKind::Return(_)) {
                program.functions[0].return_type = Type::Number;
            }

            if after_return {
                let value = if program.functions[0].return_type == Type::Number {
                    Some(number())
                } else {
                    None
                };

                program.functions[0].body.insert(
                    0,
                    TypedStatement {
                        kind: TypedStatementKind::Return(value),
                        span: Span::new(0, 7),
                    },
                );
            }

            assert_eq!(
                compile(&program),
                Err(CodegenError::UninitializedVariable {
                    function: "bad".to_owned(),
                    id: VariableId(7),
                    span,
                })
            );
        }
    }
}

#[test]
fn scalar_local_types_are_checked_before_initialization_or_rhs_emission() {
    let span = Span::new(12, 34);
    let cases = vec![
        (
            declaration(8, Type::Number, boolean(), span),
            Type::Number,
            Type::Bool,
        ),
        (
            declaration(8, Type::Bool, number(), span),
            Type::Bool,
            Type::Number,
        ),
        (
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(0, "p0", Type::Bool)),
                span,
            },
            Type::Number,
            Type::Bool,
        ),
        (
            TypedStatement {
                kind: TypedStatementKind::Expression(identifier(7, "x", Type::Bool)),
                span,
            },
            Type::Number,
            Type::Bool,
        ),
        (
            assignment(identifier(0, "p0", Type::Number), boolean(), span),
            Type::Number,
            Type::Bool,
        ),
        (
            assignment(identifier(0, "p0", Type::Bool), boolean(), span),
            Type::Number,
            Type::Bool,
        ),
        (
            assignment(identifier(7, "x", Type::Bool), number_literal("NaN"), span),
            Type::Number,
            Type::Bool,
        ),
        (
            TypedStatement {
                kind: TypedStatementKind::Expression(TypedExpr {
                    kind: TypedExprKind::Bool(true),
                    ty: Type::Number,
                }),
                span,
            },
            Type::Bool,
            Type::Number,
        ),
    ];

    for (statement, expected, found) in cases {
        let mut program = local_program(vec![
            statement,
            declaration(7, Type::Number, number(), Span::new(40, 60)),
        ]);

        program.functions[0].params = user(0, "bad", &[Type::Number], Type::Void).params;

        assert_eq!(
            compile(&program),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected,
                found,
                span,
            })
        );
    }
}

#[test]
fn non_identifier_assignment_targets_including_array_indices_are_rejected_before_rhs() {
    let span = Span::new(12, 34);
    let targets = vec![
        number(),
        boolean(),
        binary(number(), BinaryOp::Add, number()),
        TypedExpr {
            kind: TypedExprKind::Call {
                func: FunctionRef::User(FunctionId(0)),
                name: "bad".to_owned(),
                args: vec![],
            },
            ty: Type::Number,
        },
        TypedExpr {
            kind: TypedExprKind::Index {
                expr: Box::new(TypedExpr {
                    kind: TypedExprKind::Array(vec![number()]),
                    ty: Type::NumberArray,
                }),
                index: Box::new(number_literal("0")),
            },
            ty: Type::Number,
        },
    ];

    for target in targets {
        assert_eq!(
            compile(&local_program(vec![assignment(
                target,
                number_literal("NaN"),
                span
            )])),
            Err(CodegenError::InvalidAssignmentTarget {
                function: "bad".to_owned(),
                span,
            }),
        );
    }
}

#[test]
fn unsupported_local_types_are_rejected_before_body_emission_even_in_unreachable_nested_blocks() {
    let span = Span::new(40, 60);

    for ty in [Type::String, Type::NumberArray, Type::Void] {
        for nested in [false, true] {
            let local = declaration(7, ty, number(), span);
            let statement = if nested {
                TypedStatement {
                    kind: TypedStatementKind::If {
                        cond: Box::new(boolean()),
                        then: vec![],
                        else_: Some(vec![TypedStatement {
                            kind: TypedStatementKind::While {
                                cond: Box::new(boolean()),
                                body: vec![local],
                            },
                            span: Span::new(30, 70),
                        }]),
                    },
                    span: Span::new(20, 80),
                }
            } else {
                local
            };
            let program = local_program(vec![
                TypedStatement {
                    kind: TypedStatementKind::Expression(number_literal("NaN")),
                    span: Span::new(0, 7),
                },
                TypedStatement {
                    kind: TypedStatementKind::Return(None),
                    span: Span::new(8, 15),
                },
                statement,
            ]);

            assert_eq!(
                compile(&program),
                Err(CodegenError::UnsupportedLocalType {
                    function: "bad".to_owned(),
                    ty,
                    span,
                })
            );
        }
    }
}

#[test]
fn unreachable_local_initializers_and_assignments_are_still_validated() {
    let span = Span::new(40, 60);

    for kind in [
        declaration(8, Type::Number, number_literal("NaN"), span).kind,
        assignment(
            identifier(7, "x", Type::Number),
            number_literal("NaN"),
            span,
        )
        .kind,
    ] {
        let program = local_program(vec![
            declaration(7, Type::Number, number(), Span::new(0, 7)),
            TypedStatement {
                kind: TypedStatementKind::Return(None),
                span: Span::new(8, 15),
            },
            TypedStatement { kind, span },
        ]);

        assert_eq!(
            compile(&program),
            Err(CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            })
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_parameterized_arithmetic_and_numeric_mutations_at_all_optimization_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
            .unwrap();
        compiler
            .register_host_function("ready", "provider", "ready", vec![], Type::Bool)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn calculate(n: number, scale: number) -> number {
                 let value: number = n * scale + 1;
                 n = value - 3;
                 value = -n / 2;
                 value + 99;
                 return value;
             }
             fn passthrough(n: number) -> number { return n; }",
            )
            .expect("compile parameterized numeric runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("calculate".to_owned(), ExternalKind::Func, 2)
        );
        assert_eq!(module.locals, vec![vec![ValType::F64], vec![]]);

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            let calls = 0;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: { numeric: () => { calls += 1; return 99; } },
                provider: { ready: () => { calls += 1; return 1; } },
            });

            assert.equal(instance.exports.calculate(4, 3), -5);
            assert.equal(instance.exports.calculate(2, 0.5), 0.5);
            assert.equal(instance.exports.calculate(-2, 4), 5);
            assert.equal(instance.exports.passthrough(0.25), 0.25);
            assert.ok(Object.is(instance.exports.passthrough(-0), -0));
            assert.equal(calls, 0);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_bool_assignments_returns_and_drops_at_all_optimization_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("unused", "env", "unused", vec![Type::Bool], Type::Bool)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn saved(flag: bool, n: number) -> bool {
                 let value: number = n + 1;
                 let copy: bool = flag;
                 let ready: bool = false;
                 ready = copy;
                 flag = true;
                 value = value * 2;
                 value;
                 flag;
                 false;
                 return ready;
             }
             fn reset(flag: bool) -> bool { flag = false; return flag; }
             fn yes() -> bool { return true; }
             fn discard(n: number, flag: bool) -> void {
                 let copy: bool = flag;
                 copy = true;
                 n = n + 2;
                 n;
                 copy;
             }",
            )
            .expect("compile bool assignment runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("saved".to_owned(), ExternalKind::Func, 1)
        );
        assert_eq!(
            module.locals,
            vec![
                vec![ValType::F64, ValType::I32, ValType::I32],
                vec![],
                vec![],
                vec![ValType::I32],
            ]
        );

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            let calls = 0;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: { unused: flag => { calls += 1; return flag; } },
            });

            assert.equal(instance.exports.saved(0, 7), 0);
            assert.equal(instance.exports.saved(1, 7), 1);
            assert.equal(instance.exports.reset(1), 0);
            assert.equal(instance.exports.yes(), 1);
            assert.equal(instance.exports.discard(3, 0), undefined);
            assert.equal(instance.exports.discard(9, 1), undefined);
            assert.equal(calls, 0);
        "#,
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
