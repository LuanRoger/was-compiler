use crate::{
    ast::{BinaryOp, Type},
    compiler::{CompileError, Compiler, CompilerOptions},
    host::{HostFunction, HostFunctionId},
    optimization::OptimizationLevel,
    tokens::Span,
    typed_ast::{
        BuiltinFunction, FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedFunction,
        TypedProgram, TypedStatement, TypedStatementKind,
    },
};
use wasmparser::{DataKind, ExternalKind, Operator, Parser, Payload, TypeRef, ValType, Validator};

use super::{CodegenError, compile};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

#[derive(Debug, Default, PartialEq)]
struct MemoryModule {
    sections: Vec<u8>,
    signatures: Vec<(Vec<ValType>, Vec<ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    memories: Vec<(u64, Option<u64>, bool, bool)>,
    globals: Vec<(ValType, bool, i32)>,
    data: Vec<(u32, Vec<u8>)>,
    bodies: Vec<Vec<String>>,
    calls: Vec<Vec<u32>>,
    constants: Vec<Vec<i32>>,
}

fn decode(bytes: &[u8]) -> MemoryModule {
    Validator::new()
        .validate_all(bytes)
        .expect("valid memory Wasm");
    let mut module = MemoryModule::default();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("Wasm payload");
        if let Some((id, _)) = payload.as_section() {
            module.sections.push(id);
        }
        match payload {
            Payload::TypeSection(reader) => {
                for ty in reader.into_iter_err_on_gc_types() {
                    let ty = ty.expect("function type");
                    module
                        .signatures
                        .push((ty.params().to_vec(), ty.results().to_vec()));
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.expect("import");
                    let TypeRef::Func(index) = import.ty else {
                        panic!("function import");
                    };
                    module
                        .imports
                        .push((import.module.into(), import.name.into(), index));
                }
            }
            Payload::FunctionSection(reader) => {
                module.functions = reader
                    .into_iter()
                    .map(|ty| ty.expect("function index"))
                    .collect();
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory.expect("memory");
                    module.memories.push((
                        memory.initial,
                        memory.maximum,
                        memory.memory64,
                        memory.shared,
                    ));
                }
            }
            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global.expect("global");
                    let mut ops = global.init_expr.get_operators_reader();
                    let Operator::I32Const { value } = ops.read().expect("global initializer")
                    else {
                        panic!("i32 heap initializer");
                    };
                    assert!(matches!(ops.read().unwrap(), Operator::End));
                    module
                        .globals
                        .push((global.ty.content_type, global.ty.mutable, value));
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.expect("export");
                    module
                        .exports
                        .push((export.name.into(), export.kind, export.index));
                }
            }
            Payload::DataSection(reader) => {
                for segment in reader {
                    let segment = segment.expect("data segment");
                    let DataKind::Active {
                        memory_index,
                        offset_expr,
                    } = segment.kind
                    else {
                        panic!("active data segment");
                    };
                    assert_eq!(memory_index, 0);
                    let Operator::I32Const { value } =
                        offset_expr.get_operators_reader().read().unwrap()
                    else {
                        panic!("constant data offset");
                    };
                    module.data.push((value as u32, segment.data.to_vec()));
                }
            }
            Payload::CodeSectionEntry(body) => {
                let mut ops = body.get_operators_reader().expect("operators");
                let mut instructions = Vec::new();
                let mut calls = Vec::new();
                let mut constants = Vec::new();
                while !ops.eof() {
                    let op = ops.read().expect("operator");
                    if let Operator::Call { function_index } = &op {
                        calls.push(*function_index);
                    }
                    if let Operator::I32Const { value } = &op {
                        constants.push(*value);
                    }
                    instructions.push(format!("{op:?}"));
                }
                module.bodies.push(instructions);
                module.calls.push(calls);
                module.constants.push(constants);
            }
            _ => {}
        }
    }
    module
}

fn host(id: usize, name: &str, params: &[Type], result: Type) -> HostFunction {
    HostFunction {
        id: HostFunctionId(id),
        name: name.into(),
        module: "env".into(),
        field: name.into(),
        params: params.to_vec(),
        return_type: result,
    }
}

fn compiler(level: OptimizationLevel, hosts: &[HostFunction]) -> Compiler {
    let mut compiler = Compiler::with_options(CompilerOptions {
        optimization_level: level,
    });
    for host in hosts {
        assert_eq!(
            compiler
                .register_host_function(
                    &host.name,
                    &host.module,
                    &host.field,
                    host.params.clone(),
                    host.return_type,
                )
                .expect("register memory host"),
            host.id,
        );
    }
    compiler
}

fn compile_source(source: &str) -> Vec<u8> {
    Compiler::new()
        .compile_source(source)
        .expect("compile memory source")
}

fn number(text: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number(text.into()),
        ty: Type::Number,
    }
}

fn string(text: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::String(text.into()),
        ty: Type::String,
    }
}

fn boolean() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Bool(true),
        ty: Type::Bool,
    }
}

fn array(elements: Vec<TypedExpr>) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Array(elements),
        ty: Type::NumberArray,
    }
}

fn index(base: TypedExpr, index: TypedExpr) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Index {
            expr: Box::new(base),
            index: Box::new(index),
        },
        ty: Type::Number,
    }
}

fn len(args: Vec<TypedExpr>) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Call {
            func: FunctionRef::Builtin(BuiltinFunction::Len),
            name: "not_a_lookup".into(),
            args,
        },
        ty: Type::Number,
    }
}

fn equal(left: TypedExpr, right: TypedExpr, op: BinaryOp) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
        ty: Type::Bool,
    }
}

fn return_program(value: TypedExpr, span: Span) -> TypedProgram {
    TypedProgram {
        host_functions: vec![],
        functions: vec![TypedFunction {
            id: FunctionId(0),
            name: "bad".into(),
            params: vec![],
            return_type: value.ty,
            body: vec![TypedStatement {
                kind: TypedStatementKind::Return(Some(value)),
                span,
            }],
            span: Span::new(0, 100),
        }],
    }
}

fn mismatch(expected: Type, found: Type, span: Span) -> CodegenError {
    CodegenError::TypeMismatch {
        function: "bad".into(),
        expected,
        found,
        span,
    }
}

fn image(module: &MemoryModule) -> Vec<u8> {
    let mut image = vec![0; module.globals[0].2 as usize];
    for (offset, data) in &module.data {
        let start = *offset as usize;
        image[start..start + data.len()].copy_from_slice(data);
    }
    image
}

fn word(image: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(image[offset..offset + 4].try_into().unwrap())
}

fn static_string(image: &[u8], handle: u32) -> &[u8] {
    assert!(handle >= 8 && handle.is_multiple_of(8));
    let data = word(image, handle as usize) as usize;
    let length = word(image, handle as usize + 4) as usize;
    &image[data..data + length]
}

fn value_for_type(ty: Type) -> TypedExpr {
    match ty {
        Type::Number => number("1"),
        Type::Bool => boolean(),
        Type::String => string("valid"),
        Type::NumberArray => array(vec![]),
        Type::Void => panic!("value type"),
    }
}

mod layout;
mod runtime;
mod validation;
