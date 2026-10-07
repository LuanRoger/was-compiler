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
        BuiltinFunction, FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedFunction,
        TypedParam, TypedProgram, TypedStatement, TypedStatementKind, VariableId,
    },
};
use wasmparser::{BlockType, ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

use super::{CodegenError, checked_u32, compile, locals::LocalLayout};

#[derive(Debug, Clone, PartialEq)]
enum DecodedInstruction {
    F64Const(f64),
    I32Const(i32),
    LocalGet(u32),
    LocalSet(u32),
    Call(u32),
    I32Eqz,
    I32Eq,
    I32Ne,
    F64Eq,
    F64Ne,
    F64Lt,
    F64Le,
    F64Gt,
    F64Ge,
    Block(BlockType),
    Loop(BlockType),
    If(BlockType),
    Else,
    Br(u32),
    BrIf(u32),
    Unreachable,
    Drop,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Neg,
    Return,
    End,
    Runtime(String),
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
                        Operator::Call { function_index } => {
                            DecodedInstruction::Call(function_index)
                        }
                        Operator::I32Eqz => DecodedInstruction::I32Eqz,
                        Operator::I32Eq => DecodedInstruction::I32Eq,
                        Operator::I32Ne => DecodedInstruction::I32Ne,
                        Operator::F64Eq => DecodedInstruction::F64Eq,
                        Operator::F64Ne => DecodedInstruction::F64Ne,
                        Operator::F64Lt => DecodedInstruction::F64Lt,
                        Operator::F64Le => DecodedInstruction::F64Le,
                        Operator::F64Gt => DecodedInstruction::F64Gt,
                        Operator::F64Ge => DecodedInstruction::F64Ge,
                        Operator::Block { blockty } => DecodedInstruction::Block(blockty),
                        Operator::Loop { blockty } => DecodedInstruction::Loop(blockty),
                        Operator::If { blockty } => DecodedInstruction::If(blockty),
                        Operator::Else => DecodedInstruction::Else,
                        Operator::Br { relative_depth } => DecodedInstruction::Br(relative_depth),
                        Operator::BrIf { relative_depth } => {
                            DecodedInstruction::BrIf(relative_depth)
                        }
                        Operator::Unreachable => DecodedInstruction::Unreachable,
                        Operator::Drop => DecodedInstruction::Drop,
                        Operator::F64Add => DecodedInstruction::F64Add,
                        Operator::F64Sub => DecodedInstruction::F64Sub,
                        Operator::F64Mul => DecodedInstruction::F64Mul,
                        Operator::F64Div => DecodedInstruction::F64Div,
                        Operator::F64Neg => DecodedInstruction::F64Neg,
                        Operator::Return => DecodedInstruction::Return,
                        Operator::End => DecodedInstruction::End,
                        operator => DecodedInstruction::Runtime(format!("{operator:?}")),
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

fn call(reference: FunctionRef, name: &str, args: Vec<TypedExpr>, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Call {
            func: reference,
            name: name.to_owned(),
            args,
        },
        ty,
    }
}

fn call_program(value: TypedExpr, params: &[Type], return_type: Type, span: Span) -> TypedProgram {
    let mut program = local_program(vec![TypedStatement {
        kind: TypedStatementKind::Expression(value),
        span,
    }]);
    let mut callee = user(1, "callee", params, return_type);

    if return_type != Type::Void {
        callee.body.push(TypedStatement {
            kind: TypedStatementKind::Return(Some(if return_type == Type::Bool {
                boolean()
            } else {
                number()
            })),
            span: Span::new(0, 0),
        });
    }

    program.functions.push(callee);
    program
        .host_functions
        .push(host(0, "callee", params, return_type));

    program
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
fn memory_parameter_and_return_abis_use_i32_for_hosts_and_users() {
    for ty in [Type::String, Type::NumberArray] {
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
                                TypedExprKind::Array(vec![number()])
                            },
                            ty,
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
    for return_type in [Type::Number, Type::Bool, Type::String, Type::NumberArray] {
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
fn malformed_memory_expression_statements_keep_their_span_even_after_return() {
    for after_return in [false, true] {
        for (expected, value) in [
            (
                Type::String,
                TypedExpr {
                    kind: TypedExprKind::String("bad".to_owned()),
                    ty: Type::Number,
                },
            ),
            (
                Type::NumberArray,
                TypedExpr {
                    kind: TypedExprKind::Array(vec![number()]),
                    ty: Type::Number,
                },
            ),
        ] {
            let kind = TypedStatementKind::Expression(value);
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
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected,
                    found: Type::Number,
                    span: Span::new(12, 34),
                })
            );
        }
    }
}

#[test]
fn host_call_statements_drop_only_scalar_results_and_normalize_bool_returns() {
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

        let mut expected = vec![DecodedInstruction::Call(0)];

        if expression_type == Type::Bool {
            expected.extend([DecodedInstruction::I32Eqz, DecodedInstruction::I32Eqz]);
        }

        if expression_type != Type::Void {
            expected.push(DecodedInstruction::Drop);
        }

        expected.push(DecodedInstruction::End);

        assert_eq!(decode(&compile(&program).unwrap()).bodies, vec![expected]);
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
    let expressions = vec![("%", binary(boolean(), BinaryOp::Remainder, boolean()))];

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
fn source_returns_reject_remainder_with_its_label() {
    for (expression, return_type, operator) in
        [("1 % 2", "number", "%"), ("flag_number % 2", "number", "%")]
    {
        let source =
            format!("fn bad(flag_number: number) -> {return_type} {{ return {expression}; }}");
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
            TypedExpr {
                kind: TypedExprKind::String("bad".to_owned()),
                ty: Type::Number,
            },
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::String,
                found: Type::Number,
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
                    func: FunctionRef::Host(HostFunctionId(1)),
                    name: "numeric".to_owned(),
                    args: vec![],
                },
                ty: Type::Number,
            },
            CodegenError::UnknownFunctionReference {
                function: "bad".to_owned(),
                reference: FunctionRef::Host(HostFunctionId(1)),
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
fn malformed_memory_expression_kinds_are_rejected_with_the_statement_span() {
    let expressions = vec![
        (Type::String, TypedExprKind::String("text".to_owned())),
        (Type::NumberArray, TypedExprKind::Array(vec![number()])),
        (
            Type::NumberArray,
            TypedExprKind::Index {
                expr: Box::new(boolean()),
                index: Box::new(number_literal("0")),
            },
        ),
    ];
    let span = Span::new(12, 34);

    for (expected, kind) in expressions {
        let is_index = matches!(&kind, TypedExprKind::Index { .. });
        let value = TypedExpr {
            kind,
            ty: Type::Number,
        };

        for is_statement in [false, true] {
            let mut program = if is_statement {
                local_program(vec![TypedStatement {
                    kind: TypedStatementKind::Expression(value.clone()),
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
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected,
                    found: if is_index { Type::Bool } else { Type::Number },
                    span,
                }),
            );
        }
    }
}

#[test]
fn source_builtin_len_propagates_unsupported_array_element_operators() {
    let compiler = Compiler::new();
    let program = compiler
        .analyze("fn bad() -> number { return len([1 % 2]); }")
        .expect("analyze builtin return expression");

    assert_eq!(
        compiler.compile(&program),
        Err(CodegenError::UnsupportedOperator {
            function: "bad".to_owned(),
            operator: "%",
            span: program.functions[0].body[0].span,
        }),
    );
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
            Some(binary(number(), BinaryOp::Remainder, number())),
            CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator: "%",
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
fn local_layout_collects_and_emits_nested_shadow_bindings_in_preorder() {
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

    let module = decode(&compile(&program).expect("compile nested shadow bindings"));

    assert_eq!(
        module.locals[0],
        vec![
            ValType::F64,
            ValType::I32,
            ValType::F64,
            ValType::I32,
            ValType::F64,
            ValType::I32,
            ValType::F64
        ]
    );
    assert!(module.bodies[0].contains(&DecodedInstruction::If(BlockType::Empty)));
    assert!(module.bodies[0].contains(&DecodedInstruction::Loop(BlockType::Empty)));
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
fn non_identifier_assignment_targets_and_index_bases_are_rejected_before_rhs() {
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

    for ty in [Type::Void] {
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
fn forward_nested_scalar_and_void_calls_preserve_import_indices_at_all_optimization_levels() {
    let mut snapshot = None;

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
            .register_host_function("seed", "env", "seed", vec![Type::Number], Type::Number)
            .unwrap();
        compiler
            .register_host_function("ready", "env", "ready", vec![Type::Bool], Type::Bool)
            .unwrap();
        compiler
            .register_host_function("print", "env", "log", vec![Type::Number], Type::Void)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn run(n: number, flag: bool) -> number {
                    let value: number = identity(seed(n) + 2);
                    let copy: bool = pass(ready(flag));
                    copy;
                    relay(value);
                    identity(value);
                    return identity(seed(value));
                }
                fn identity(n: number) -> number { return n; }
                fn pass(flag: bool) -> bool { return flag; }
                fn relay(n: number) -> void { print(n); }",
            )
            .expect("compile forward nested calls");
        let module = decode(&bytes);

        assert_eq!(
            module.exports,
            vec![
                ("run".to_owned(), ExternalKind::Func, 3),
                ("identity".to_owned(), ExternalKind::Func, 4),
                ("pass".to_owned(), ExternalKind::Func, 5),
                ("relay".to_owned(), ExternalKind::Func, 6),
            ],
        );
        assert_eq!(
            module.imports,
            vec![
                ("env".to_owned(), "seed".to_owned(), 0),
                ("env".to_owned(), "ready".to_owned(), 1),
                ("env".to_owned(), "log".to_owned(), 2),
            ],
        );
        assert_eq!(
            module.bodies,
            vec![
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(0),
                    DecodedInstruction::F64Const(2.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(4),
                    DecodedInstruction::LocalSet(2),
                    DecodedInstruction::LocalGet(1),
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::Call(1),
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::I32Eqz,
                    DecodedInstruction::Call(5),
                    DecodedInstruction::LocalSet(3),
                    DecodedInstruction::LocalGet(3),
                    DecodedInstruction::Drop,
                    DecodedInstruction::LocalGet(2),
                    DecodedInstruction::Call(6),
                    DecodedInstruction::LocalGet(2),
                    DecodedInstruction::Call(4),
                    DecodedInstruction::Drop,
                    DecodedInstruction::LocalGet(2),
                    DecodedInstruction::Call(0),
                    DecodedInstruction::Call(4),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(2),
                    DecodedInstruction::End,
                ],
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes);
        }
    }
}

#[test]
fn call_ids_are_authoritative_even_when_names_match_other_users_or_hosts() {
    let span = Span::new(12, 34);
    let mut program = call_program(
        binary(
            call(
                FunctionRef::User(FunctionId(1)),
                "host_only",
                vec![],
                Type::Number,
            ),
            BinaryOp::Add,
            call(
                FunctionRef::Host(HostFunctionId(0)),
                "callee",
                vec![],
                Type::Number,
            ),
        ),
        &[],
        Type::Number,
        span,
    );

    program.host_functions[0].name = "host_only".to_owned();
    program.functions[0].body.push(TypedStatement {
        kind: TypedStatementKind::Expression(call(
            FunctionRef::User(FunctionId(1)),
            "not_registered_anywhere",
            vec![],
            Type::Number,
        )),
        span,
    });

    assert_eq!(
        decode(&compile(&program).unwrap()).bodies[0],
        vec![
            DecodedInstruction::Call(2),
            DecodedInstruction::Call(0),
            DecodedInstruction::F64Add,
            DecodedInstruction::Drop,
            DecodedInstruction::Call(2),
            DecodedInstruction::Drop,
            DecodedInstruction::End,
        ],
    );
}

#[test]
fn host_bool_arguments_are_normalized_immediately_between_nested_scalar_calls() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function(
            "host_bool",
            "env",
            "predicate",
            vec![Type::Bool, Type::Number, Type::Bool],
            Type::Bool,
        )
        .unwrap();
    compiler
        .register_host_function(
            "host_number",
            "env",
            "numeric",
            vec![Type::Number],
            Type::Number,
        )
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn bad(a: bool, n: number, b: bool) -> bool {
                return host_bool(user_bool(a), host_number(n), user_bool(b));
            }
            fn user_bool(flag: bool) -> bool { return flag; }",
        )
        .expect("compile mixed host arguments");

    assert_eq!(
        decode(&bytes).bodies[0],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::Call(3),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::LocalGet(1),
            DecodedInstruction::Call(1),
            DecodedInstruction::LocalGet(2),
            DecodedInstruction::Call(3),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::Call(0),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ],
    );
}

#[test]
fn self_and_mutual_calls_link_without_executing_unbounded_guest_recursion() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("unused", "env", "unused", vec![], Type::Void)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn recurse(n: number) -> number { return recurse(n); }
            fn left(flag: bool) -> bool { return right(flag); }
            fn right(flag: bool) -> bool { return left(flag); }
            fn spin() -> void { spin(); }",
        )
        .expect("compile recursive references");
    let module = decode(&bytes);

    assert_eq!(
        module.bodies,
        vec![
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(3),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(2),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
            vec![DecodedInstruction::Call(4), DecodedInstruction::End],
        ],
    );
}

#[test]
fn calls_reject_argument_counts_before_emitting_any_argument() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for args in [vec![], vec![number_literal("NaN"), number()]] {
            let found = args.len();
            let program = call_program(
                call(reference, "ignored", args, Type::Number),
                &[Type::Number],
                Type::Number,
                span,
            );

            assert_eq!(
                compile(&program),
                Err(CodegenError::ArgumentCountMismatch {
                    function: "bad".to_owned(),
                    reference,
                    expected: 1,
                    found,
                    span,
                }),
            );
        }
    }

    let program = return_program(
        Some(call(
            FunctionRef::User(FunctionId(0)),
            "bad",
            vec![number()],
            Type::Number,
        )),
        Type::Number,
        span,
    );

    assert_eq!(
        compile(&program),
        Err(CodegenError::ArgumentCountMismatch {
            function: "bad".to_owned(),
            reference: FunctionRef::User(FunctionId(0)),
            expected: 0,
            found: 1,
            span,
        }),
    );
}

#[test]
fn calls_check_real_return_types_including_void_before_argument_errors() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for expected in [Type::Number, Type::Bool, Type::Void] {
            for found in [Type::Number, Type::Bool, Type::Void] {
                if expected == found {
                    continue;
                }

                let program = call_program(
                    call(reference, "ignored", vec![number_literal("NaN")], found),
                    &[Type::Number],
                    expected,
                    span,
                );

                assert_eq!(
                    compile(&program),
                    Err(CodegenError::TypeMismatch {
                        function: "bad".to_owned(),
                        expected,
                        found,
                        span,
                    }),
                );
            }
        }
    }

    let mut program = return_program(Some(number()), Type::Number, Span::new(0, 7));

    program.functions[0].body.push(TypedStatement {
        kind: TypedStatementKind::Expression(call(
            FunctionRef::User(FunctionId(0)),
            "bad",
            vec![],
            Type::Void,
        )),
        span,
    });

    assert_eq!(
        compile(&program),
        Err(CodegenError::TypeMismatch {
            function: "bad".to_owned(),
            expected: Type::Number,
            found: Type::Void,
            span,
        }),
    );
}

#[test]
fn calls_require_exact_argument_types_at_first_and_higher_positions() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for expected in [Type::Number, Type::Bool] {
            for found in [
                Type::Number,
                Type::Bool,
                Type::Void,
                Type::String,
                Type::NumberArray,
            ] {
                if expected == found {
                    continue;
                }

                for position in [0, 2] {
                    let valid = if expected == Type::Bool {
                        boolean()
                    } else {
                        number()
                    };
                    let mut args = vec![valid; 3];

                    args[position] = TypedExpr {
                        kind: TypedExprKind::Number("NaN".to_owned()),
                        ty: found,
                    };

                    let program = call_program(
                        call(reference, "ignored", args, Type::Void),
                        &[expected; 3],
                        Type::Void,
                        span,
                    );

                    assert_eq!(
                        compile(&program),
                        Err(CodegenError::TypeMismatch {
                            function: "bad".to_owned(),
                            expected,
                            found,
                            span,
                        }),
                    );
                }
            }
        }
    }
}

#[test]
fn call_arguments_are_type_checked_and_emitted_left_to_right() {
    let span = Span::new(12, 34);
    let cases = vec![
        (
            vec![number_literal("NaN"), boolean()],
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            },
        ),
        (
            vec![boolean(), number_literal("NaN")],
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            vec![number(), number_literal("NaN")],
            CodegenError::InvalidNumberLiteral {
                function: "bad".to_owned(),
                literal: "NaN".to_owned(),
                span,
            },
        ),
        (
            vec![identifier(99, "unknown", Type::Number), boolean()],
            CodegenError::UnknownVariable {
                function: "bad".to_owned(),
                id: VariableId(99),
                span,
            },
        ),
    ];

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        for (args, expected) in &cases {
            let value = call(reference, "ignored", args.clone(), Type::Number);
            let statements = vec![
                TypedStatement {
                    kind: TypedStatementKind::Expression(value.clone()),
                    span,
                },
                TypedStatement {
                    kind: TypedStatementKind::Return(Some(value.clone())),
                    span,
                },
                declaration(7, Type::Number, value.clone(), span),
                assignment(identifier(0, "p0", Type::Number), value, span),
            ];

            for statement in statements {
                let mut program =
                    call_program(number(), &[Type::Number, Type::Number], Type::Number, span);

                program.functions[0].params = user(0, "bad", &[Type::Number], Type::Number).params;
                program.functions[0].return_type = Type::Number;
                program.functions[0].body = vec![statement];

                assert_eq!(
                    &compile(&program).expect_err("reject call argument"),
                    expected
                );
            }
        }
    }
}

#[test]
fn unknown_call_references_do_not_spill_between_host_and_user_tables_or_use_names() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(2)),
        FunctionRef::User(FunctionId(usize::MAX)),
        FunctionRef::Host(HostFunctionId(1)),
        FunctionRef::Host(HostFunctionId(2)),
        FunctionRef::Host(HostFunctionId(usize::MAX)),
    ] {
        let program = call_program(
            call(reference, "callee", vec![number_literal("NaN")], Type::Bool),
            &[],
            Type::Number,
            span,
        );

        assert_eq!(
            compile(&program),
            Err(CodegenError::UnknownFunctionReference {
                function: "bad".to_owned(),
                reference,
                span,
            }),
        );
    }
}

#[test]
fn nested_builtin_argument_types_are_checked_before_emission_in_all_statement_contexts() {
    let span = Span::new(40, 60);
    let builtin = call(
        FunctionRef::Builtin(BuiltinFunction::Len),
        "not_a_lookup",
        vec![number_literal("NaN")],
        Type::Number,
    );
    let value = call(
        FunctionRef::Host(HostFunctionId(0)),
        "callee",
        vec![builtin],
        Type::Number,
    );
    let statements = vec![
        TypedStatement {
            kind: TypedStatementKind::Expression(value.clone()),
            span,
        },
        TypedStatement {
            kind: TypedStatementKind::Return(Some(value.clone())),
            span,
        },
        declaration(7, Type::Number, value.clone(), span),
        assignment(identifier(0, "p0", Type::Number), value, span),
    ];

    for statement in statements {
        let mut function = user(0, "bad", &[Type::Number], Type::Number);

        function.body = vec![statement];

        assert_eq!(
            compile(&TypedProgram {
                functions: vec![function],
                host_functions: vec![host(0, "callee", &[Type::Number], Type::Number)],
            }),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::NumberArray,
                found: Type::Number,
                span,
            }),
        );
    }
}

#[test]
fn void_calls_are_rejected_in_scalar_returns_initializers_assignments_and_arguments() {
    let span = Span::new(12, 34);

    for reference in [
        FunctionRef::User(FunctionId(1)),
        FunctionRef::Host(HostFunctionId(0)),
    ] {
        let value = call(reference, "callee", vec![], Type::Void);
        let statements = vec![
            TypedStatement {
                kind: TypedStatementKind::Return(Some(value.clone())),
                span,
            },
            declaration(7, Type::Number, value.clone(), span),
            assignment(identifier(0, "p0", Type::Number), value.clone(), span),
            TypedStatement {
                kind: TypedStatementKind::Expression(call(
                    FunctionRef::Host(HostFunctionId(1)),
                    "numeric",
                    vec![value],
                    Type::Number,
                )),
                span,
            },
        ];

        for statement in statements {
            let mut program = call_program(number(), &[], Type::Void, span);

            program.functions[0].params = user(0, "bad", &[Type::Number], Type::Number).params;
            program.functions[0].return_type = Type::Number;
            program.functions[0].body = vec![statement];
            program
                .host_functions
                .push(host(1, "numeric", &[Type::Number], Type::Number));

            assert_eq!(
                compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Number,
                    found: Type::Void,
                    span,
                }),
            );
        }
    }
}

#[test]
fn call_statements_preserve_drop_cleanup_after_return_for_host_and_user_results() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("numeric", "env", "numeric", vec![], Type::Number)
        .unwrap();
    compiler
        .register_host_function("predicate", "env", "predicate", vec![], Type::Bool)
        .unwrap();
    compiler
        .register_host_function("noop", "env", "noop", vec![], Type::Void)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn discard() -> void {
                numeric(); predicate(); noop(); number_user(); bool_user(); void_user();
                return;
                numeric(); predicate(); noop(); number_user(); bool_user(); void_user();
            }
            fn number_user() -> number { return 1; }
            fn bool_user() -> bool { return true; }
            fn void_user() -> void {}",
        )
        .expect("compile call statement cleanup");
    let sequence = vec![
        DecodedInstruction::Call(0),
        DecodedInstruction::Drop,
        DecodedInstruction::Call(1),
        DecodedInstruction::I32Eqz,
        DecodedInstruction::I32Eqz,
        DecodedInstruction::Drop,
        DecodedInstruction::Call(2),
        DecodedInstruction::Call(4),
        DecodedInstruction::Drop,
        DecodedInstruction::Call(5),
        DecodedInstruction::Drop,
        DecodedInstruction::Call(6),
    ];
    let mut expected = sequence.clone();

    expected.push(DecodedInstruction::Return);
    expected.extend(sequence);
    expected.push(DecodedInstruction::End);

    assert_eq!(decode(&bytes).bodies[0], expected);
}

#[test]
fn invalid_unreachable_calls_abort_without_partial_output_and_keep_statement_spans() {
    let span = Span::new(40, 60);
    let cases = vec![
        (
            call(
                FunctionRef::Host(HostFunctionId(1)),
                "callee",
                vec![],
                Type::Number,
            ),
            CodegenError::UnknownFunctionReference {
                function: "bad".to_owned(),
                reference: FunctionRef::Host(HostFunctionId(1)),
                span,
            },
        ),
        (
            call(
                FunctionRef::Host(HostFunctionId(0)),
                "callee",
                vec![],
                Type::Number,
            ),
            CodegenError::ArgumentCountMismatch {
                function: "bad".to_owned(),
                reference: FunctionRef::Host(HostFunctionId(0)),
                expected: 1,
                found: 0,
                span,
            },
        ),
        (
            call(
                FunctionRef::Host(HostFunctionId(0)),
                "callee",
                vec![number()],
                Type::Bool,
            ),
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            call(
                FunctionRef::User(FunctionId(1)),
                "callee",
                vec![boolean()],
                Type::Number,
            ),
            CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            },
        ),
        (
            call(
                FunctionRef::Builtin(BuiltinFunction::Len),
                "len",
                vec![],
                Type::Number,
            ),
            CodegenError::ArgumentCountMismatch {
                function: "bad".to_owned(),
                reference: FunctionRef::Builtin(BuiltinFunction::Len),
                expected: 1,
                found: 0,
                span,
            },
        ),
    ];

    for (value, expected) in cases {
        let mut program = call_program(value, &[Type::Number], Type::Number, span);

        program.functions[0].body.insert(
            0,
            TypedStatement {
                kind: TypedStatementKind::Return(None),
                span: Span::new(0, 7),
            },
        );

        assert_eq!(compile(&program), Err(expected));
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_nested_user_host_calls_left_to_right_with_void_side_effects() {
    let mut snapshot = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        for (name, field, params, result) in [
            ("unused", "unused", vec![], Type::Void),
            ("mark", "mark", vec![Type::Number], Type::Number),
            ("join", "join", vec![Type::Number; 3], Type::Number),
            ("print", "log", vec![Type::Number], Type::Void),
        ] {
            compiler
                .register_host_function(name, "env", field, params, result)
                .unwrap();
        }

        let bytes = compiler
            .compile_source(
                "fn entry() -> number {
                    let result: number = combine(mark(1), identity(mark(2)), mark(identity(mark(3))));
                    relay(result);
                    combine(mark(4), mark(5), mark(6));
                    return result;
                }
                fn identity(n: number) -> number { return n; }
                fn combine(a: number, b: number, c: number) -> number { return join(a, b, c); }
                fn relay(n: number) -> void { print(n); }",
            )
            .expect("compile nested user and host runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("entry".to_owned(), ExternalKind::Func, 4)
        );
        assert_eq!(
            module.bodies[0],
            vec![
                DecodedInstruction::F64Const(1.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::F64Const(2.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(5),
                DecodedInstruction::F64Const(3.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(5),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(6),
                DecodedInstruction::LocalSet(0),
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(7),
                DecodedInstruction::F64Const(4.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::F64Const(5.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::F64Const(6.0),
                DecodedInstruction::Call(1),
                DecodedInstruction::Call(6),
                DecodedInstruction::Drop,
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const events = [];
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    unused: () => assert.fail('unused import called'),
                    mark: n => { events.push(['mark', n]); return n + 10; },
                    join: (a, b, c) => { events.push(['join', a, b, c]); return a * 100 + b * 10 + c; },
                    log: n => { events.push(['log', n]); return 999; },
                },
            });

            assert.equal(instance.exports.entry(), 1243);
            assert.deepEqual(events, [
                ['mark', 1], ['mark', 2], ['mark', 3], ['mark', 13],
                ['join', 11, 12, 23], ['log', 1243],
                ['mark', 4], ['mark', 5], ['mark', 6], ['join', 14, 15, 16],
            ]);
            events.length = 0;
            assert.equal(instance.exports.relay(7), undefined);
            assert.deepEqual(events, [['log', 7]]);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_host_bool_normalization_ieee754_numbers_and_import_link_errors() {
    let mut snapshot = None;

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
            .register_host_function(
                "host_bool",
                "env",
                "predicate",
                vec![Type::Bool, Type::Number, Type::Bool],
                Type::Bool,
            )
            .unwrap();
        compiler
            .register_host_function(
                "host_number",
                "env",
                "numeric",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn bool_bridge(flag: bool, n: number, other: bool) -> bool {
                    return guest_bool(host_bool(flag, n, other));
                }
                fn guest_bool(flag: bool) -> bool { return flag; }
                fn raw_bool(flag: bool) -> bool { return guest_bool(flag); }
                fn number_bridge(n: number) -> number { return guest_number(host_number(n)); }
                fn guest_number(n: number) -> number { return n; }",
            )
            .expect("compile host ABI runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports[0],
            ("bool_bridge".to_owned(), ExternalKind::Func, 2)
        );
        assert_eq!(
            module.bodies[0],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::LocalGet(1),
                DecodedInstruction::LocalGet(2),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::Call(0),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::Call(3),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        );
        assert_eq!(
            module.bodies[2],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::Call(3),
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const module = new WebAssembly.Module(bytes);
            let result = 0;
            const booleans = [];
            const numbers = [];
            const imports = {
                env: {
                    predicate: (flag, n, other) => { booleans.push([flag, n, other]); return result; },
                    numeric: n => { numbers.push(n); return n; },
                },
            };
            const instance = new WebAssembly.Instance(module, imports);

            for (const raw of [2, -2, 0]) {
                result = raw;
                assert.equal(instance.exports.bool_bridge(2, 7.5, -2), raw === 0 ? 0 : 1);
                assert.equal(instance.exports.bool_bridge(0, 9, 0), raw === 0 ? 0 : 1);
            }
            assert.deepEqual(booleans, [
                [1, 7.5, 1], [0, 9, 0], [1, 7.5, 1], [0, 9, 0], [1, 7.5, 1], [0, 9, 0],
            ]);
            for (const raw of [2, -2, 0]) {
                assert.equal(instance.exports.raw_bool(raw), raw);
            }
            for (const n of [NaN, Infinity, -Infinity, -0, 0.25]) {
                assert.ok(Object.is(instance.exports.number_bridge(n), n));
                assert.ok(Object.is(numbers.at(-1), n));
            }
            for (const field of ['predicate', 'numeric']) {
                for (const value of [undefined, null, 7, {}]) {
                    const env = { ...imports.env, [field]: value };
                    assert.throws(() => new WebAssembly.Instance(module, { env }), WebAssembly.LinkError);
                }
                const env = { ...imports.env };
                delete env[field];
                assert.throws(() => new WebAssembly.Instance(module, { env }), WebAssembly.LinkError);
            }
            assert.throws(() => new WebAssembly.Instance(module, {}), TypeError);
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_self_and_mutual_recursion_with_a_bounded_host_tick() {
    let mut snapshot = None;

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
            .register_host_function("unused", "env", "unused", vec![], Type::Void)
            .unwrap();
        compiler
            .register_host_function("tick", "env", "tick", vec![Type::Number], Type::Void)
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn recurse(n: number) -> number { tick(n); return recurse(n + 1); }
                fn left(n: number) -> void { tick(n); right(n + 1); }
                fn right(n: number) -> void { tick(n); left(n + 1); }",
            )
            .expect("compile bounded recursive runtime");
        let module = decode(&bytes);

        assert_eq!(
            module.exports,
            vec![
                ("recurse".to_owned(), ExternalKind::Func, 2),
                ("left".to_owned(), ExternalKind::Func, 3),
                ("right".to_owned(), ExternalKind::Func, 4),
            ],
        );
        assert_eq!(
            module.bodies,
            vec![
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(1),
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(2),
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(1),
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(4),
                    DecodedInstruction::End,
                ],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::Call(1),
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::Call(3),
                    DecodedInstruction::End,
                ],
            ],
        );

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const sentinel = new Error('bounded tick sentinel');
            const seen = [];
            let remaining = 0;
            const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    unused: () => assert.fail('unused import called'),
                    tick: n => {
                        seen.push(n);
                        remaining -= 1;
                        if (remaining === 0) throw sentinel;
                    },
                },
            });

            for (const name of ['recurse', 'left', 'right']) {
                for (const bound of [1, 2, 7, 12]) {
                    remaining = bound;
                    seen.length = 0;
                    assert.throws(() => instance.exports[name](10), error => error === sentinel);
                    assert.equal(remaining, 0);
                    assert.deepEqual(seen, Array.from({ length: bound }, (_, i) => 10 + i));
                }
            }
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

fn bool_binary(left: TypedExpr, op: BinaryOp, right: TypedExpr) -> TypedExpr {
    let mut expr = binary(left, op, right);

    expr.ty = Type::Bool;
    expr
}

fn bool_not(value: TypedExpr) -> TypedExpr {
    let mut expr = unary(UnaryOp::Not, value);

    expr.ty = Type::Bool;
    expr
}

fn returned(value: Option<TypedExpr>, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::Return(value),
        span,
    }
}

fn conditional(
    cond: TypedExpr,
    then: Vec<TypedStatement>,
    else_: Option<Vec<TypedStatement>>,
    span: Span,
) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::If {
            cond: Box::new(cond),
            then,
            else_,
        },
        span,
    }
}

fn while_loop(cond: TypedExpr, body: Vec<TypedStatement>, span: Span) -> TypedStatement {
    TypedStatement {
        kind: TypedStatementKind::While {
            cond: Box::new(cond),
            body,
        },
        span,
    }
}

#[test]
fn numeric_comparisons_emit_f64_operands_and_bool_results_at_all_optimization_levels() {
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        for (operator, instruction) in [
            ("==", DecodedInstruction::F64Eq),
            ("!=", DecodedInstruction::F64Ne),
            ("<", DecodedInstruction::F64Lt),
            ("<=", DecodedInstruction::F64Le),
            (">", DecodedInstruction::F64Gt),
            (">=", DecodedInstruction::F64Ge),
        ] {
            let source = format!(
                "fn compare(a: number, b: number) -> bool {{ return a + 1 {operator} b * 2; }}"
            );
            let module = decode(
                &compiler
                    .compile_source(&source)
                    .expect("compile comparison"),
            );

            assert_eq!(
                module.signatures,
                vec![(vec![ValType::F64, ValType::F64], vec![ValType::I32])]
            );
            assert_eq!(
                module.bodies[0],
                vec![
                    DecodedInstruction::LocalGet(0),
                    DecodedInstruction::F64Const(1.0),
                    DecodedInstruction::F64Add,
                    DecodedInstruction::LocalGet(1),
                    DecodedInstruction::F64Const(2.0),
                    DecodedInstruction::F64Mul,
                    instruction,
                    DecodedInstruction::Return,
                    DecodedInstruction::End,
                ],
                "operator {operator}, level {level:?}"
            );
        }
    }
}

#[test]
fn bool_equality_normalizes_both_operands_and_not_emits_eqz() {
    let bytes = Compiler::new()
        .compile_source(
            "fn equal(a: bool, b: bool) -> bool { return a == b; }
         fn unequal(a: bool, b: bool) -> bool { return a != b; }
         fn invert(a: bool) -> bool { return !a; }",
        )
        .expect("compile bool operators");
    let module = decode(&bytes);

    for (index, instruction) in [DecodedInstruction::I32Eq, DecodedInstruction::I32Ne]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            module.bodies[index],
            vec![
                DecodedInstruction::LocalGet(0),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                DecodedInstruction::LocalGet(1),
                DecodedInstruction::I32Eqz,
                DecodedInstruction::I32Eqz,
                instruction,
                DecodedInstruction::Return,
                DecodedInstruction::End,
            ]
        );
    }

    assert_eq!(
        module.bodies[2],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::I32Eqz,
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn logical_operators_guard_rhs_calls_inside_i32_result_ifs() {
    let mut compiler = Compiler::new();

    compiler
        .register_host_function("rhs", "env", "rhs", vec![], Type::Bool)
        .unwrap();

    let bytes = compiler
        .compile_source(
            "fn both(a: bool) -> bool { return a && rhs(); }
         fn either(a: bool) -> bool { return a || rhs(); }
         fn nested(a: bool, b: bool) -> bool { return a && (b || rhs()); }",
        )
        .expect("compile short circuit");
    let module = decode(&bytes);

    for (index, rhs_in_then, constant) in [(0, true, 0), (1, false, 1)] {
        let body = &module.bodies[index];
        let if_index = body
            .iter()
            .position(|op| *op == DecodedInstruction::If(BlockType::Type(ValType::I32)))
            .unwrap();
        let else_index = body
            .iter()
            .position(|op| *op == DecodedInstruction::Else)
            .unwrap();
        let call_index = body
            .iter()
            .position(|op| *op == DecodedInstruction::Call(0))
            .unwrap();

        assert!(if_index < call_index);
        assert_eq!(call_index < else_index, rhs_in_then);
        assert!(body.contains(&DecodedInstruction::I32Const(constant)));
        assert_eq!(
            body.iter()
                .filter(|op| **op == DecodedInstruction::Call(0))
                .count(),
            1
        );
        assert_eq!(
            &body[call_index + 1..call_index + 3],
            &[DecodedInstruction::I32Eqz, DecodedInstruction::I32Eqz]
        );
        assert_eq!(
            &body[body.len() - 3..],
            &[
                DecodedInstruction::End,
                DecodedInstruction::Return,
                DecodedInstruction::End
            ]
        );
    }

    assert_eq!(
        module.bodies[2]
            .iter()
            .filter(|op| **op == DecodedInstruction::If(BlockType::Type(ValType::I32)))
            .count(),
        2
    );
    assert_eq!(
        module.bodies[2]
            .iter()
            .filter(|op| **op == DecodedInstruction::Call(0))
            .count(),
        1
    );
}

#[test]
fn logical_branches_preserve_live_equality_operands_and_call_arguments() {
    use DecodedInstruction::*;

    let bytes = Compiler::new()
        .compile_source(
            "fn compare(a: bool, b: bool, c: bool) -> bool { return a == (b && c); }
             fn invoke(a: bool, b: bool) -> number { return take(7, a || b); }
             fn take(value: number, flag: bool) -> number {
                 if flag { return value; } else { return -value; }
             }",
        )
        .expect("compile logical branches with live stack prefixes");
    let module = decode(&bytes);

    assert_eq!(
        &module.bodies[0][..5],
        &[
            LocalGet(0),
            I32Eqz,
            I32Eqz,
            LocalGet(1),
            If(BlockType::Type(ValType::I32))
        ]
    );
    assert_eq!(
        &module.bodies[1][..3],
        &[
            F64Const(7.0),
            LocalGet(0),
            If(BlockType::Type(ValType::I32))
        ]
    );
    assert_eq!(
        &module.bodies[1][module.bodies[1].len() - 3..],
        &[Call(2), Return, End]
    );
}

#[test]
fn forged_bool_operator_roots_and_operands_are_type_checked() {
    let span = Span::new(12, 34);
    let mut cases = vec![(bool_not(number()), Type::Bool, Type::Number)];
    let mut wrong_not_root = bool_not(boolean());

    wrong_not_root.ty = Type::Number;
    cases.push((wrong_not_root, Type::Bool, Type::Number));

    for op in [
        BinaryOp::Equal,
        BinaryOp::NotEqual,
        BinaryOp::LessThan,
        BinaryOp::LessThanOrEqual,
        BinaryOp::GreaterThan,
        BinaryOp::GreaterThanOrEqual,
        BinaryOp::And,
        BinaryOp::Or,
    ] {
        let logical = matches!(op, BinaryOp::And | BinaryOp::Or);
        let operand = if logical { boolean() } else { number() };
        let mut wrong_root = bool_binary(operand.clone(), op, operand);

        wrong_root.ty = Type::Number;
        cases.push((wrong_root, Type::Bool, Type::Number));

        if logical {
            cases.push((
                bool_binary(number(), op, boolean()),
                Type::Bool,
                Type::Number,
            ));
            cases.push((
                bool_binary(boolean(), op, number()),
                Type::Bool,
                Type::Number,
            ));
        } else {
            cases.push((
                bool_binary(number(), op, boolean()),
                Type::Number,
                Type::Bool,
            ));

            if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
                cases.push((
                    bool_binary(boolean(), op, number()),
                    Type::Bool,
                    Type::Number,
                ));
            } else {
                cases.push((
                    bool_binary(boolean(), op, number()),
                    Type::Number,
                    Type::Bool,
                ));
            }
        }
    }

    for (value, expected, found) in cases {
        assert_eq!(
            compile(&return_program(Some(value.clone()), value.ty, span)),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected,
                found,
                span,
            }),
            "expression {value:?}"
        );
    }
}

#[test]
fn logical_rhs_is_validated_even_when_runtime_short_circuit_would_skip_it() {
    let span = Span::new(12, 34);

    for (op, left) in [
        (
            BinaryOp::And,
            TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool,
            },
        ),
        (BinaryOp::Or, boolean()),
    ] {
        let right = bool_binary(number(), BinaryOp::Remainder, number());
        let value = bool_binary(left.clone(), op, right);

        assert_eq!(
            compile(&return_program(Some(value), Type::Bool, span)),
            Err(CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator: "%",
                span,
            })
        );
        assert_eq!(
            compile(&return_program(
                Some(bool_binary(left, op, number())),
                Type::Bool,
                span
            )),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Bool,
                found: Type::Number,
                span,
            })
        );
    }
}

#[test]
fn if_else_complete_returns_emit_unreachable_after_end_and_validate() {
    let bytes = Compiler::new()
        .compile_source(
            "fn choose(flag: bool) -> number { if flag { return 1; } else { return 2; } }
         fn partial(flag: bool) -> number { if flag { return 1; } return 2; }
         fn noop(flag: bool) -> void { if flag { return; } else { return; } }",
        )
        .expect("compile conditional returns");
    let module = decode(&bytes);

    assert_eq!(
        module.bodies[0],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::If(BlockType::Empty),
            DecodedInstruction::F64Const(1.0),
            DecodedInstruction::Return,
            DecodedInstruction::Else,
            DecodedInstruction::F64Const(2.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
            DecodedInstruction::Unreachable,
            DecodedInstruction::End,
        ]
    );
    assert_eq!(
        module.bodies[1],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::If(BlockType::Empty),
            DecodedInstruction::F64Const(1.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
            DecodedInstruction::F64Const(2.0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]
    );
    assert_eq!(
        module.bodies[2],
        vec![
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::If(BlockType::Empty),
            DecodedInstruction::Return,
            DecodedInstruction::Else,
            DecodedInstruction::Return,
            DecodedInstruction::End,
            DecodedInstruction::Unreachable,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn while_emits_empty_block_loop_exit_and_back_edge_with_body_locals() {
    let bytes = Compiler::new()
        .compile_source(
            "fn countdown(n: number) -> number {
             while n > 0 { let step: number = 1; n = n - step; }
             return n;
         }",
        )
        .expect("compile loop with declaration");
    let module = decode(&bytes);

    assert_eq!(module.locals[0], vec![ValType::F64]);
    assert_eq!(
        module.bodies[0],
        vec![
            DecodedInstruction::Block(BlockType::Empty),
            DecodedInstruction::Loop(BlockType::Empty),
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::F64Const(0.0),
            DecodedInstruction::F64Gt,
            DecodedInstruction::I32Eqz,
            DecodedInstruction::BrIf(1),
            DecodedInstruction::F64Const(1.0),
            DecodedInstruction::LocalSet(1),
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::LocalGet(1),
            DecodedInstruction::F64Sub,
            DecodedInstruction::LocalSet(0),
            DecodedInstruction::Br(0),
            DecodedInstruction::End,
            DecodedInstruction::End,
            DecodedInstruction::LocalGet(0),
            DecodedInstruction::Return,
            DecodedInstruction::End,
        ]
    );
}

#[test]
fn nested_loops_keep_relative_exit_and_back_edge_depths() {
    let bytes = Compiler::new()
        .compile_source(
            "fn nested(a: bool, b: bool) -> void { while a { while b { b = false; } a = false; } }",
        )
        .expect("compile nested loops");
    let module = decode(&bytes);
    let body = &module.bodies[0];

    assert_eq!(
        body.iter()
            .filter(|op| **op == DecodedInstruction::Block(BlockType::Empty))
            .count(),
        2
    );
    assert_eq!(
        body.iter()
            .filter(|op| **op == DecodedInstruction::Loop(BlockType::Empty))
            .count(),
        2
    );
    assert_eq!(
        body.iter()
            .filter_map(|op| if let DecodedInstruction::BrIf(depth) = op {
                Some(*depth)
            } else {
                None
            })
            .collect::<Vec<_>>(),
        vec![1, 1]
    );
    assert_eq!(
        body.iter()
            .filter_map(|op| if let DecodedInstruction::Br(depth) = op {
                Some(*depth)
            } else {
                None
            })
            .collect::<Vec<_>>(),
        vec![0, 0]
    );
}

#[test]
fn forged_if_and_while_conditions_require_bool_and_preserve_statement_spans() {
    let span = Span::new(12, 34);

    for cond in [
        number(),
        call(
            FunctionRef::User(FunctionId(99)),
            "missing",
            vec![],
            Type::Void,
        ),
    ] {
        for statement in [
            conditional(cond.clone(), vec![], None, span),
            while_loop(cond.clone(), vec![], span),
        ] {
            assert_eq!(
                compile(&local_program(vec![statement])),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Bool,
                    found: cond.ty,
                    span,
                })
            );
        }
    }

    let forged = TypedExpr {
        kind: TypedExprKind::Number("1".to_owned()),
        ty: Type::Bool,
    };

    for statement in [
        conditional(forged.clone(), vec![], None, span),
        while_loop(forged, vec![], span),
    ] {
        assert_eq!(
            compile(&local_program(vec![statement])),
            Err(CodegenError::TypeMismatch {
                function: "bad".to_owned(),
                expected: Type::Number,
                found: Type::Bool,
                span,
            })
        );
    }
}

#[test]
fn nested_complete_and_partial_returns_validate_with_fallbacks() {
    for source in [
        "fn complete(a: bool, b: bool) -> number { if a { if b { return 1; } else { return 2; } } else { return 3; } }",
        "fn complete(a: bool, b: bool) -> bool { if a { return b; } else { if b { return true; } else { return false; } } }",
        "fn partial(a: bool, b: bool) -> number { if a { if b { return 1; } return 2; } else { return 3; } }",
        "fn partial(a: bool, b: bool) -> number { if a { if b { return 1; } } else { return 2; } return 3; }",
        "fn loop_fallback(a: bool) -> number { while a { if a { return 1; } else { return 2; } } return 3; }",
        "fn already_returned(a: bool) -> number { return 1; while a { return 2; } if a { return 3; } }",
    ] {
        let bytes = Compiler::new()
            .compile_source(source)
            .expect("compile complete return paths");

        decode(&bytes);
    }
}

#[test]
fn missing_return_paths_include_partial_nested_conditionals_and_all_loops() {
    let span = Span::new(12, 34);

    for return_type in [Type::Number, Type::Bool] {
        let value = if return_type == Type::Number {
            number()
        } else {
            boolean()
        };
        let ret = returned(Some(value), span);
        let full_if = conditional(boolean(), vec![ret.clone()], Some(vec![ret.clone()]), span);
        let cases = vec![
            vec![conditional(boolean(), vec![ret.clone()], None, span)],
            vec![conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![]),
                span,
            )],
            vec![conditional(
                boolean(),
                vec![],
                Some(vec![ret.clone()]),
                span,
            )],
            vec![conditional(
                boolean(),
                vec![conditional(boolean(), vec![ret.clone()], None, span)],
                Some(vec![ret.clone()]),
                span,
            )],
            vec![conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![conditional(boolean(), vec![ret.clone()], None, span)]),
                span,
            )],
            vec![while_loop(boolean(), vec![ret.clone()], span)],
            vec![while_loop(
                TypedExpr {
                    kind: TypedExprKind::Bool(false),
                    ty: Type::Bool,
                },
                vec![ret.clone()],
                span,
            )],
            vec![while_loop(boolean(), vec![full_if], span)],
            vec![conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![while_loop(boolean(), vec![ret], span)]),
                span,
            )],
        ];

        for body in cases {
            let mut program = local_program(body);

            program.functions[0].return_type = return_type;

            assert_eq!(
                compile(&program),
                Err(CodegenError::MissingReturnValue {
                    function: "bad".to_owned(),
                    return_type,
                }),
                "body {:?}",
                program.functions[0].body
            );
        }
    }
}

#[test]
fn forged_branch_and_loop_local_reads_do_not_escape_their_scope() {
    let outer = Span::new(0, 100);
    let span = Span::new(40, 60);
    let local = declaration(7, Type::Bool, boolean(), Span::new(12, 34));
    let read = TypedStatement {
        kind: TypedStatementKind::Expression(identifier(7, "x", Type::Bool)),
        span,
    };
    let cases = vec![
        vec![conditional(
            boolean(),
            vec![local.clone()],
            Some(vec![read.clone()]),
            outer,
        )],
        vec![
            conditional(boolean(), vec![local.clone()], None, outer),
            read.clone(),
        ],
        vec![
            conditional(boolean(), vec![], Some(vec![local.clone()]), outer),
            read.clone(),
        ],
        vec![conditional(
            boolean(),
            vec![read.clone()],
            Some(vec![local.clone()]),
            outer,
        )],
        vec![
            while_loop(boolean(), vec![local.clone()], outer),
            read.clone(),
        ],
        vec![while_loop(
            identifier(7, "x", Type::Bool),
            vec![local.clone()],
            span,
        )],
        vec![
            conditional(boolean(), vec![local.clone()], None, outer),
            while_loop(identifier(7, "x", Type::Bool), vec![], span),
        ],
        vec![
            while_loop(boolean(), vec![local.clone()], outer),
            while_loop(identifier(7, "x", Type::Bool), vec![], span),
        ],
        vec![conditional(
            boolean(),
            vec![
                conditional(boolean(), vec![local.clone()], None, outer),
                read.clone(),
            ],
            None,
            outer,
        )],
        vec![
            conditional(boolean(), vec![local], None, outer),
            assignment(identifier(7, "x", Type::Bool), boolean(), span),
        ],
    ];

    for body in cases {
        assert_eq!(
            compile(&local_program(body)),
            Err(CodegenError::UninitializedVariable {
                function: "bad".to_owned(),
                id: VariableId(7),
                span,
            })
        );
    }
}

#[test]
fn scoped_branch_and_loop_declarations_preserve_initialized_outer_bindings() {
    let span = Span::new(12, 34);
    let outer = identifier(7, "same_name", Type::Number);
    let branch = vec![
        declaration(8, Type::Bool, boolean(), span),
        assignment(outer.clone(), number_literal("2"), span),
    ];
    let loop_body = vec![
        declaration(9, Type::Number, number_literal("3"), span),
        assignment(
            outer.clone(),
            identifier(9, "same_name", Type::Number),
            span,
        ),
    ];
    let program = local_program(vec![
        declaration(7, Type::Number, number(), span),
        conditional(
            boolean(),
            branch,
            Some(vec![assignment(outer.clone(), number_literal("4"), span)]),
            span,
        ),
        while_loop(
            TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool,
            },
            loop_body,
            span,
        ),
        TypedStatement {
            kind: TypedStatementKind::Expression(outer),
            span,
        },
    ]);
    let module = decode(&compile(&program).expect("outer initialization survives scopes"));

    assert_eq!(
        module.locals[0],
        vec![ValType::F64, ValType::I32, ValType::F64]
    );
}

#[test]
fn invalid_statements_in_unreachable_branches_loops_and_continuations_are_diagnosed() {
    let span = Span::new(40, 60);
    let outer = Span::new(0, 100);
    let invalid = TypedStatement {
        kind: TypedStatementKind::Expression(binary(number(), BinaryOp::Remainder, number())),
        span,
    };
    let ret = returned(None, Span::new(0, 7));
    let complete = conditional(boolean(), vec![ret.clone()], Some(vec![ret.clone()]), outer);
    let cases = vec![
        vec![
            ret.clone(),
            conditional(boolean(), vec![invalid.clone()], None, outer),
        ],
        vec![conditional(
            boolean(),
            vec![ret.clone()],
            Some(vec![invalid.clone()]),
            outer,
        )],
        vec![
            ret.clone(),
            while_loop(boolean(), vec![invalid.clone()], outer),
        ],
        vec![while_loop(
            TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty: Type::Bool,
            },
            vec![invalid.clone()],
            outer,
        )],
        vec![conditional(
            boolean(),
            vec![ret.clone(), invalid.clone()],
            Some(vec![ret.clone()]),
            outer,
        )],
        vec![while_loop(boolean(), vec![ret, invalid.clone()], outer)],
        vec![complete, invalid],
    ];

    for body in cases {
        assert_eq!(
            compile(&local_program(body)),
            Err(CodegenError::UnsupportedOperator {
                function: "bad".to_owned(),
                operator: "%",
                span,
            })
        );
    }
}

#[test]
fn nested_return_type_errors_keep_inner_statement_spans() {
    let span = Span::new(40, 60);
    let outer = Span::new(0, 100);

    for value in [None, Some(boolean())] {
        let ret = returned(value.clone(), span);

        for statement in [
            conditional(
                boolean(),
                vec![ret.clone()],
                Some(vec![returned(Some(number()), outer)]),
                outer,
            ),
            while_loop(boolean(), vec![ret], outer),
        ] {
            let mut program = return_program(Some(number()), Type::Number, outer);

            program.functions[0].body.push(statement);

            assert_eq!(
                compile(&program),
                Err(CodegenError::TypeMismatch {
                    function: "bad".to_owned(),
                    expected: Type::Number,
                    found: value.as_ref().map_or(Type::Void, |expr| expr.ty),
                    span,
                })
            );
        }
    }
}

fn run_guest_at_all_optimization_levels(source: &str, script: &str) {
    let mut snapshot = None;

    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Basic,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        let compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });
        let bytes = compiler
            .compile_source(source)
            .expect("compile guest runtime");

        decode(&bytes);

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(&bytes, script);
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_comparisons_ieee754_and_noncanonical_bool_operators_at_all_levels() {
    run_guest_at_all_optimization_levels(
        "fn eq(a: number, b: number) -> bool { return a == b; }
         fn ne(a: number, b: number) -> bool { return a != b; }
         fn lt(a: number, b: number) -> bool { return a < b; }
         fn le(a: number, b: number) -> bool { return a <= b; }
         fn gt(a: number, b: number) -> bool { return a > b; }
         fn ge(a: number, b: number) -> bool { return a >= b; }
         fn bool_eq(a: bool, b: bool) -> bool { return a == b; }
         fn bool_ne(a: bool, b: bool) -> bool { return a != b; }
         fn invert(a: bool) -> bool { return !a; }
         fn both(a: bool, b: bool) -> bool { return a && b; }
         fn either(a: bool, b: bool) -> bool { return a || b; }
         fn nested(a: bool, b: bool) -> bool { return (a && !b) || (a != b); }
         fn precedence(a: bool, b: bool, c: bool) -> bool { return a || b && !c; }
         fn compare_and(a: bool, b: bool, c: bool) -> bool { return a == (b && c); }
         fn invoke_or(a: bool, b: bool) -> number { return take(7, a || b); }
         fn take(value: number, flag: bool) -> number {
             if flag { return value; } else { return -value; }
         }",
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes));
        const comparisons = {
            eq: (a, b) => a === b,
            ne: (a, b) => a !== b,
            lt: (a, b) => a < b,
            le: (a, b) => a <= b,
            gt: (a, b) => a > b,
            ge: (a, b) => a >= b,
        };
        const numbers = [NaN, Infinity, -Infinity, -0, 0, -1, 0.25, 1, Number.MIN_VALUE, Number.MAX_VALUE];

        for (const a of numbers) {
            for (const b of numbers) {
                for (const [name, compare] of Object.entries(comparisons)) {
                    assert.equal(guest[name](a, b), Number(compare(a, b)), `${name}(${a}, ${b})`);
                }
            }
        }

        const booleans = [0, 1, 2, -2, 2147483647, -2147483648];

        for (const a of booleans) {
            assert.equal(guest.invert(a), Number(!a));

            for (const b of booleans) {
                assert.equal(guest.bool_eq(a, b), Number(Boolean(a) === Boolean(b)));
                assert.equal(guest.bool_ne(a, b), Number(Boolean(a) !== Boolean(b)));
                assert.equal(guest.both(a, b), Number(Boolean(a) && Boolean(b)));
                assert.equal(guest.either(a, b), Number(Boolean(a) || Boolean(b)));
                assert.equal(guest.invoke_or(a, b), a || b ? 7 : -7);
                assert.equal(guest.nested(a, b), Number((Boolean(a) && !b) || (Boolean(a) !== Boolean(b))));

                for (const c of booleans) {
                    assert.equal(guest.precedence(a, b, c), Number(Boolean(a) || (Boolean(b) && !c)));
                    assert.equal(guest.compare_and(a, b, c), Number(Boolean(a) === (Boolean(b) && Boolean(c))));
                }
            }
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_short_circuit_host_effects_in_order_at_all_levels() {
    let mut snapshot = None;

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
            .register_host_function("probe", "env", "probe", vec![Type::Number], Type::Bool)
            .unwrap();
        compiler
            .register_host_function(
                "numeric",
                "env",
                "numeric",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();

        let bytes = compiler
            .compile_source(
                "fn both() -> bool { return probe(1) && probe(2); }
             fn either() -> bool { return probe(1) || probe(2); }
             fn nested_or() -> bool { return (probe(1) && probe(2)) || (probe(3) && probe(4)); }
             fn nested_and() -> bool { return probe(1) && (probe(2) || probe(3)); }
             fn negated() -> bool { return !probe(1) || probe(2); }
             fn equal() -> bool { return probe(1) == probe(2); }
             fn unequal() -> bool { return probe(1) != probe(2); }
             fn numeric_compare() -> bool { return numeric(1) < numeric(2); }
             fn guarded_compare() -> bool { return probe(1) && numeric(2) >= numeric(3); }",
            )
            .expect("compile host effects runtime");

        decode(&bytes);

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "optimization level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const seen = [];
            const sentinel = new Error('evaluated host call');
            let values = [];
            const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    probe: id => {
                        seen.push(`p${id}`);
                        const value = values[id - 1];
                        if (value === sentinel) throw sentinel;
                        return value;
                    },
                    numeric: id => { seen.push(`n${id}`); return id; },
                },
            });
            const expressions = {
                both: p => p(1) && p(2),
                either: p => p(1) || p(2),
                nested_or: p => (p(1) && p(2)) || (p(3) && p(4)),
                nested_and: p => p(1) && (p(2) || p(3)),
                negated: p => !p(1) || p(2),
                equal: p => p(1) === p(2),
                unequal: p => p(1) !== p(2),
            };

            for (const a of [0, 2, -2]) {
                for (const b of [0, 2, -2]) {
                    for (const c of [0, 2, -2]) {
                        for (const d of [0, 2, -2]) {
                            values = [a, b, c, d];

                            for (const [name, expression] of Object.entries(expressions)) {
                                const expectedSeen = [];
                                const result = expression(id => { expectedSeen.push(`p${id}`); return Boolean(values[id - 1]); });
                                seen.length = 0;
                                assert.equal(guest[name](), Number(result), name);
                                assert.deepEqual(seen, expectedSeen, name);
                            }
                        }
                    }
                }
            }

            for (const [name, skip] of [['both', 0], ['either', -2], ['negated', 0]]) {
                values = [skip, sentinel];
                seen.length = 0;
                assert.equal(guest[name](), name === 'both' ? 0 : 1);
                assert.deepEqual(seen, ['p1']);
                values[0] = skip === 0 ? -2 : 0;
                seen.length = 0;
                assert.throws(() => guest[name](), error => error === sentinel);
                assert.deepEqual(seen, ['p1', 'p2']);
            }

            seen.length = 0;
            assert.equal(guest.numeric_compare(), 1);
            assert.deepEqual(seen, ['n1', 'n2']);

            for (const raw of [0, 2, -2]) {
                values = [raw];
                seen.length = 0;
                assert.equal(guest.guarded_compare(), 0);
                assert.deepEqual(seen, raw === 0 ? ['p1'] : ['p1', 'n2', 'n3']);
            }
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_host_if_and_repeated_loop_conditions_at_all_levels() {
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
            .register_host_function("probe", "env", "probe", vec![], Type::Bool)
            .unwrap();
        compiler
            .register_host_function(
                "numeric",
                "env",
                "numeric",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();

        let bytes = compiler.compile_source(
            "fn choose() -> number { if probe() { return numeric(2); } else { return numeric(3); } }
             fn repeat() -> number { let n: number = 0; while probe() { n = n + 1; } return n; }",
        ).expect("compile host control flow runtime");

        decode(&bytes);
        run_node(
            &bytes,
            r#"
            const assert = require('node:assert/strict');
            const bytes = require('node:fs').readFileSync(0);
            const seen = [];
            let queue = [];
            const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes), {
                env: {
                    probe: () => {
                        assert.ok(queue.length > 0, 'condition evaluated too many times');
                        seen.push('probe');
                        return queue.shift();
                    },
                    numeric: n => { seen.push(n); return n; },
                },
            });

            for (const raw of [0, 2, -2]) {
                queue = [raw];
                seen.length = 0;
                assert.equal(guest.choose(), raw === 0 ? 3 : 2);
                assert.deepEqual(seen, ['probe', raw === 0 ? 3 : 2]);
                assert.deepEqual(queue, []);
            }

            for (const conditions of [[0], [2, 0], [2, -2, 0]]) {
                queue = [...conditions];
                seen.length = 0;
                assert.equal(guest.repeat(), conditions.length - 1);
                assert.deepEqual(seen, conditions.map(() => 'probe'));
                assert.deepEqual(queue, []);
            }
        "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_if_loops_early_returns_and_nested_shadowing_at_all_levels() {
    run_guest_at_all_optimization_levels(
        "fn choose(flag: bool) -> number { if flag { return 1; } else { return 2; } }
         fn partial(a: bool, b: bool) -> number { if a { if b { return 1; } } else { return 2; } return 3; }
         fn sum(n: number) -> number {
             let total: number = 0;
             while n > 0 { let current: number = n; total = total + current; n = n - 1; }
             return total;
         }
         fn nested(n: number, m: number) -> number {
             let total: number = 0;
             let i: number = 0;
             while i < n {
                 let j: number = 0;
                 while j < m { let term: number = i * 10 + j; total = total + term; j = j + 1; }
                 i = i + 1;
             }
             return total;
         }
         fn find(n: number, m: number, target: number) -> number {
             let i: number = 0;
             while i < n {
                 let j: number = 0;
                 while j < m {
                     let term: number = i * 10 + j;
                     if term == target { return term; }
                     j = j + 1;
                 }
                 i = i + 1;
             }
             return -1;
         }
         fn early(n: number) -> number { while n > 0 { if n == 3 { return n; } n = n - 1; } return 0; }
         fn loop_flag(flag: bool) -> number { let n: number = 0; while flag { n = n + 1; flag = false; } return n; }
         fn shadow(flag: bool, n: number) -> number {
             let result: number = n;
             if flag {
                 let n: bool = true;
                 if n { let result: number = 99; result; }
                 result = result + 1;
             } else { let n: number = 10; result = result + n; }
             let remaining: number = n;
             while remaining > 0 {
                 let n: bool = false;
                 let result: number = 5;
                 if !n { result = result + 1; }
                 remaining = remaining - 1;
             }
             return result + n;
         }",
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes));

        for (const a of [0, 1, 2, -2]) {
            assert.equal(guest.choose(a), a === 0 ? 2 : 1);
            assert.equal(guest.loop_flag(a), a === 0 ? 0 : 1);

            for (const b of [0, 1, 2, -2]) {
                assert.equal(guest.partial(a, b), a === 0 ? 2 : b === 0 ? 3 : 1);
            }

            for (const n of [0, 1, 2, 5]) {
                assert.equal(guest.shadow(a, n), 2 * n + (a === 0 ? 10 : 1));
            }
        }

        for (const n of [0, 1, 2, 5, 10]) {
            assert.equal(guest.sum(n), n * (n + 1) / 2);
            assert.equal(guest.early(n), n >= 3 ? 3 : 0);

            for (const m of [0, 1, 3, 5]) {
                const terms = [];

                for (let i = 0; i < n; i += 1) {
                    for (let j = 0; j < m; j += 1) terms.push(i * 10 + j);
                }

                assert.equal(guest.nested(n, m), terms.reduce((total, term) => total + term, 0));

                for (const target of [-1, 0, 2, 10, 21, 44, 99]) {
                    assert.equal(guest.find(n, m, target), terms.includes(target) ? target : -1);
                }
            }
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_executes_guest_only_factorial_and_mutual_recursion_base_cases_at_all_levels() {
    run_guest_at_all_optimization_levels(
        "fn factorial(n: number) -> number {
             if n <= 1 { return 1; } else { return n * factorial(n - 1); }
         }
         fn even(n: number) -> bool { if n <= 0 { return true; } else { return odd(n - 1); } }
         fn odd(n: number) -> bool { if n <= 0 { return false; } else { return even(n - 1); } }",
        r#"
        const assert = require('node:assert/strict');
        const bytes = require('node:fs').readFileSync(0);
        const { exports: guest } = new WebAssembly.Instance(new WebAssembly.Module(bytes));
        let factorial = 1;

        for (let n = 0; n <= 12; n += 1) {
            if (n > 0) factorial *= n;
            assert.equal(guest.factorial(n), factorial);
        }

        for (let n = 0; n <= 30; n += 1) {
            assert.equal(guest.even(n), Number(n % 2 === 0));
            assert.equal(guest.odd(n), Number(n % 2 !== 0));
        }

        assert.equal(guest.factorial(-1), 1);
        assert.equal(guest.even(-1), 1);
        assert.equal(guest.odd(-1), 0);
        "#,
    );
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
