use std::{process::Command, time::Duration};

use crate::{
    ast::{BinaryOp, Type, UnaryOp},
    compiler::{CompileError, Compiler, CompilerOptions},
    host::{HostFunction, HostFunctionId},
    optimization::OptimizationLevel,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, FunctionId, FunctionRef, LoopId, TypedExpr, TypedExprKind, TypedFunction,
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
    F64Load { offset: u64, align: u8, memory: u32 },
    F64Store { offset: u64, align: u8, memory: u32 },
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
                        Operator::F64Load { memarg } => DecodedInstruction::F64Load {
                            offset: memarg.offset,
                            align: memarg.align,
                            memory: memarg.memory,
                        },
                        Operator::F64Store { memarg } => DecodedInstruction::F64Store {
                            offset: memarg.offset,
                            align: memarg.align,
                            memory: memarg.memory,
                        },
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
        contracts: Vec::new(),
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
        ty: Type::Number.into(),
    }
}

fn unary(op: UnaryOp, expr: TypedExpr) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Unary {
            op,
            expr: Box::new(expr),
        },
        ty: Type::Number.into(),
    }
}

fn binary(left: TypedExpr, op: BinaryOp, right: TypedExpr) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
        ty: Type::Number.into(),
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
        ty: ty.into(),
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
    let output = crate::test_process::output(
        Command::new("node").args(["-e", script]),
        Some(bytes),
        Duration::from_secs(15),
    );

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
        ty: Type::Bool.into(),
    }
}

fn call(reference: FunctionRef, name: &str, args: Vec<TypedExpr>, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Call {
            func: reference,
            name: name.to_owned(),
            args,
        },
        ty: ty.into(),
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

fn bool_binary(left: TypedExpr, op: BinaryOp, right: TypedExpr) -> TypedExpr {
    let mut expr = binary(left, op, right);

    expr.ty = Type::Bool.into();
    expr
}

fn bool_not(value: TypedExpr) -> TypedExpr {
    let mut expr = unary(UnaryOp::Not, value);

    expr.ty = Type::Bool.into();
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
            id: LoopId(span.start()),
            cond: Box::new(cond),
            body,
        },
        span,
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

mod block_values;
mod calls;
mod compound_assignment;
mod control_flow;
mod expressions;
mod layout;
mod locals;
mod loop_control;
mod malformed;
mod runtime;
mod value_loops;
