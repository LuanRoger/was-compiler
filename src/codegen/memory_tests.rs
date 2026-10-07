use std::{
    io::Write,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

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

#[test]
fn scalar_only_modules_keep_their_sections_exports_and_function_counts() {
    let bytes = compile_source("fn add(a: number, b: number) -> number { return a + b; }");
    let module = decode(&bytes);
    assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.exports, vec![("add".into(), ExternalKind::Func, 0)]);
    assert!(module.memories.is_empty());
    assert!(module.globals.is_empty());
    assert!(module.data.is_empty());
}

#[test]
fn memory_modules_reject_user_exports_named_memory_or_allocator() {
    for name in ["memory", "__alloc"] {
        for source in [
            format!("fn {name}(a: number[]) -> void {{}}"),
            format!("fn {name}() -> void {{}} fn payload() -> string {{ return \"value\"; }}"),
        ] {
            for level in LEVELS {
                assert_eq!(
                    compiler(level, &[]).compile_source(&source),
                    Err(CompileError::Codegen(CodegenError::DuplicateExportName {
                        name: name.into(),
                    })),
                    "source {source}, level {level:?}",
                );
            }
        }
    }
}

#[test]
fn scalar_only_modules_allow_user_exports_named_memory_and_allocator() {
    let source = "fn memory() -> number { return 1; } fn __alloc() -> bool { return true; }";
    for level in LEVELS {
        let bytes = compiler(level, &[])
            .compile_source(source)
            .expect("reserved memory exports remain legal without memory usage");
        let module = decode(&bytes);
        assert_eq!(
            module.exports,
            vec![
                ("memory".into(), ExternalKind::Func, 0),
                ("__alloc".into(), ExternalKind::Func, 1),
            ]
        );
        assert_eq!(module.sections, vec![1, 2, 3, 7, 10]);
        assert_eq!(module.functions.len(), 2);
        assert!(module.memories.is_empty());
        assert!(module.globals.is_empty());
        assert!(module.data.is_empty());
    }
}

#[test]
fn memory_abi_import_offsets_six_unique_helpers_and_bytes_are_stable_at_all_levels() {
    let hosts = [
        host(0, "echo_string", &[Type::String], Type::String),
        host(1, "echo_array", &[Type::NumberArray], Type::NumberArray),
        host(2, "scalar", &[Type::Number], Type::Number),
    ];
    let source = r#"
        fn text(s: string) -> string { return echo_string(s); }
        fn values(a: number[]) -> number[] { return echo_array(a); }
        fn size(a: number[]) -> number { return len(a); }
        fn at(a: number[], i: number) -> number { return a[i]; }
        fn same(a: string, b: string) -> bool { return a == b; }
        fn literal() -> number[] { return [scalar(1), 2]; }
    "#;
    let mut snapshot = None;
    for level in LEVELS {
        let compiler = compiler(level, &hosts);
        let bytes = compiler.compile_source(source).expect("compile mixed ABI");
        assert_eq!(bytes, compiler.compile_source(source).unwrap());
        let module = decode(&bytes);
        assert_eq!(
            module
                .imports
                .iter()
                .map(|(m, f, _)| (m.as_str(), f.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("env", "echo_string"),
                ("env", "echo_array"),
                ("env", "scalar")
            ]
        );
        assert_eq!(module.imports[0].2, module.imports[1].2);
        assert_eq!(module.functions.len(), 12);
        assert_eq!(module.bodies.len(), 12);
        for (i, name) in ["text", "values", "size", "at", "same", "literal"]
            .iter()
            .enumerate()
        {
            assert!(
                module
                    .exports
                    .contains(&(name.to_string(), ExternalKind::Func, i as u32 + 3))
            );
        }
        assert_eq!(module.exports.len(), 8);
        assert!(
            module
                .exports
                .contains(&("memory".into(), ExternalKind::Memory, 0))
        );
        assert!(
            module
                .exports
                .contains(&("__alloc".into(), ExternalKind::Func, 9))
        );
        let expected = [
            (vec![ValType::I32], vec![ValType::I32]),
            (vec![ValType::I32], vec![ValType::I32]),
            (vec![ValType::I32], vec![ValType::I32]),
            (vec![ValType::I32, ValType::F64], vec![ValType::I32]),
            (vec![ValType::I32], vec![ValType::F64]),
            (vec![ValType::I32, ValType::I32], vec![ValType::I32]),
        ];
        for (type_index, signature) in module.functions[6..].iter().zip(expected) {
            assert_eq!(module.signatures[*type_index as usize], signature);
        }
        assert!(module.calls[0].contains(&0));
        assert!(module.calls[0].contains(&10));
        assert!(module.calls[1].contains(&1));
        assert!(module.calls[1].contains(&11));
        assert!(module.calls[2].contains(&13));
        assert!(module.calls[3].contains(&12));
        assert!(module.calls[4].contains(&14));
        assert!(module.calls[5].contains(&2));
        assert!(module.calls[5].contains(&9));
        for left in 6..12 {
            for right in left + 1..12 {
                assert_ne!(module.bodies[left], module.bodies[right]);
            }
        }
        assert_eq!(module.memories, vec![(1, Some(65535), false, false)]);
        assert_eq!(module.globals, vec![(ValType::I32, true, 8)]);
        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "level {level:?}");
        } else {
            snapshot = Some(bytes);
        }
    }
}

#[test]
fn memory_is_needed_by_signatures_locals_and_unreachable_literals() {
    for source in [
        "fn unused(s: string) -> void {}",
        "fn unused(a: number[]) -> void {}",
        "fn local() -> void { let a: number[] = []; }",
        "fn later() -> void { return; \"static\"; }",
    ] {
        let module = decode(&compile_source(source));
        assert_eq!(module.memories.len(), 1, "{source}");
        assert_eq!(module.functions.len(), 7, "{source}");
        assert!(
            module
                .exports
                .iter()
                .any(|(name, kind, _)| name == "memory" && *kind == ExternalKind::Memory)
        );
    }
    for ty in [Type::String, Type::NumberArray] {
        let module = decode(
            &compile(&TypedProgram {
                functions: vec![],
                host_functions: vec![host(0, "unused", &[ty], ty)],
            })
            .unwrap(),
        );
        assert_eq!(module.functions.len(), 6);
        assert_eq!(module.exports.len(), 2);
        assert!(
            module
                .exports
                .contains(&("__alloc".into(), ExternalKind::Func, 1))
        );
    }
}

#[test]
fn static_strings_are_utf8_byte_descriptors_interned_in_deterministic_preorder() {
    let source = "fn first(flag: bool) -> string {\n\
        if flag { return \"é🙂\"; } else { return \"\"; }\n\
        return \"A\0B\"; }\n\
        fn repeated() -> string { return \"é🙂\"; }\n\
        fn last() -> string { return \"A\0B\"; }";
    let bytes = compile_source(source);
    assert_eq!(bytes, compile_source(source));
    let module = decode(&bytes);
    let image = image(&module);
    let handles = &module.constants[0];
    assert_eq!(handles.len(), 3);
    assert!(handles[0] < handles[1] && handles[1] < handles[2]);
    for (handle, expected) in handles.iter().zip(["é🙂", "", "A\0B"]) {
        assert_eq!(static_string(&image, *handle as u32), expected.as_bytes());
    }
    assert_eq!(module.constants[1], vec![handles[0]]);
    assert_eq!(module.constants[2], vec![handles[2]]);
    let heap = module.globals[0].2 as u32;
    assert_eq!(heap % 8, 0);
    assert!(heap >= handles[2] as u32 + 8 + 3);
    assert_eq!(module.memories[0].0, (heap as u64).div_ceil(65536).max(1));
}

#[test]
fn initial_memory_covers_large_static_payloads_and_heap_begins_after_them() {
    let text = "é".repeat(40000);
    let bytes = compile(&return_program(string(&text), Span::new(12, 34))).unwrap();
    let module = decode(&bytes);
    let heap = module.globals[0].2 as u64;
    assert_eq!(
        module.memories,
        vec![(heap.div_ceil(65536), Some(65535), false, false)]
    );
    assert_eq!(module.memories[0].0, 2);
    assert_eq!(
        static_string(&image(&module), module.constants[0][0] as u32),
        text.as_bytes()
    );
}

#[test]
fn memory_expressions_validate_with_live_stack_prefixes_and_nested_calls_at_all_levels() {
    let source = r#"
        fn identity(a: number[]) -> number[] { return a; }
        fn combine(n: number, a: number[], s: string, b: bool) -> number {
            if b { return n + len(a); } else { return n; }
        }
        fn work(a: number[], i: number) -> number {
            return 10 + combine(2, identity([a[i], len(identity(a))]), "value", "a" == "a");
        }
    "#;
    for level in LEVELS {
        decode(
            &compiler(level, &[])
                .compile_source(source)
                .expect("validate stack prefixes"),
        );
    }
}

#[test]
fn forged_memory_roots_operands_and_nested_errors_preserve_statement_spans() {
    let span = Span::new(12, 34);
    let mut wrong_string = string("value");
    wrong_string.ty = Type::Bool;
    let mut wrong_array = array(vec![]);
    wrong_array.ty = Type::Number;
    let mut wrong_index = index(array(vec![]), number("0"));
    wrong_index.ty = Type::Bool;
    let mut wrong_len = len(vec![array(vec![])]);
    wrong_len.ty = Type::Bool;
    let mut wrong_equal = equal(string("a"), string("b"), BinaryOp::Equal);
    wrong_equal.ty = Type::Number;
    let cases = [
        (wrong_string, mismatch(Type::String, Type::Bool, span)),
        (wrong_array, mismatch(Type::NumberArray, Type::Number, span)),
        (wrong_index, mismatch(Type::Number, Type::Bool, span)),
        (wrong_len, mismatch(Type::Number, Type::Bool, span)),
        (wrong_equal, mismatch(Type::Bool, Type::Number, span)),
        (
            array(vec![boolean()]),
            mismatch(Type::Number, Type::Bool, span),
        ),
        (
            index(string("bad"), number("0")),
            mismatch(Type::NumberArray, Type::String, span),
        ),
        (
            index(array(vec![]), boolean()),
            mismatch(Type::Number, Type::Bool, span),
        ),
        (
            len(vec![string("bad")]),
            mismatch(Type::NumberArray, Type::String, span),
        ),
        (
            equal(string("a"), number("1"), BinaryOp::Equal),
            mismatch(Type::String, Type::Number, span),
        ),
        (
            equal(string("a"), boolean(), BinaryOp::NotEqual),
            mismatch(Type::String, Type::Bool, span),
        ),
        (
            array(vec![number("NaN"), boolean()]),
            CodegenError::InvalidNumberLiteral {
                function: "bad".into(),
                literal: "NaN".into(),
                span,
            },
        ),
        (
            array(vec![boolean(), number("NaN")]),
            mismatch(Type::Number, Type::Bool, span),
        ),
        (
            index(array(vec![number("-1")]), number("NaN")),
            CodegenError::InvalidNumberLiteral {
                function: "bad".into(),
                literal: "-1".into(),
                span,
            },
        ),
    ];
    for (value, expected) in cases {
        for after_return in [false, true] {
            let mut program = return_program(value.clone(), span);
            if after_return {
                program.functions[0].body.insert(
                    0,
                    TypedStatement {
                        kind: TypedStatementKind::Return(Some(value_for_type(value.ty))),
                        span: Span::new(0, 7),
                    },
                );
            }
            assert_eq!(
                compile(&program).expect_err("reject malformed memory expression"),
                expected,
                "{value:?}, after return {after_return}"
            );
        }
    }
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

#[test]
fn len_checks_arity_and_rejects_strings_in_source_and_typed_ast() {
    let span = Span::new(20, 40);
    for args in [vec![], vec![array(vec![]), number("NaN")]] {
        let count = args.len();
        assert_eq!(
            compile(&return_program(len(args), span)),
            Err(CodegenError::ArgumentCountMismatch {
                function: "bad".into(),
                reference: FunctionRef::Builtin(BuiltinFunction::Len),
                expected: 1,
                found: count,
                span,
            })
        );
    }
    for source in [
        "fn bad() -> number { return len(\"text\"); }",
        "fn bad() -> number { return len(1); }",
        "fn bad() -> number[] { return [true]; }",
        "fn bad(a: number[]) -> number { return a[true]; }",
    ] {
        assert!(
            matches!(
                Compiler::new().compile_source(source),
                Err(CompileError::Semantic(_))
            ),
            "{source}"
        );
    }
}

#[test]
fn indexed_assignment_checks_identifier_base_target_index_and_rhs_types() {
    let source = "fn bad(a: number[]) -> void { a[0] = 1; }";
    let compiler = Compiler::new();
    let original = compiler.analyze(source).unwrap();
    let span = original.functions[0].body[0].span;
    for (which, expected) in [
        (
            0,
            CodegenError::InvalidAssignmentTarget {
                function: "bad".into(),
                span,
            },
        ),
        (1, mismatch(Type::Number, Type::Bool, span)),
        (2, mismatch(Type::Number, Type::Bool, span)),
        (3, mismatch(Type::NumberArray, Type::Bool, span)),
    ] {
        let mut program = original.clone();
        let TypedStatementKind::Assignment { target, value } =
            &mut program.functions[0].body[0].kind
        else {
            unreachable!()
        };
        let TypedExprKind::Index { expr, index } = &mut target.kind else {
            unreachable!()
        };
        match which {
            0 => {
                **expr = array(vec![]);
                *value = number("NaN");
            }
            1 => {
                target.ty = Type::Bool;
                *value = number("NaN");
            }
            2 => {
                **index = boolean();
                *value = number("NaN");
            }
            3 => {
                expr.ty = Type::Bool;
                *value = number("NaN");
            }
            _ => unreachable!(),
        }
        assert_eq!(compile(&program), Err(expected));
    }
    let mut program = original;
    let TypedStatementKind::Assignment { value, .. } = &mut program.functions[0].body[0].kind
    else {
        unreachable!()
    };
    *value = boolean();
    assert_eq!(
        compile(&program),
        Err(mismatch(Type::Number, Type::Bool, span))
    );
}

#[test]
fn memory_local_initializers_remain_checked_in_unreachable_nested_blocks() {
    let original = Compiler::new()
        .analyze(
            r#"
        fn bad() -> void {
            return;
            if true {
                while false { let s: string = "valid"; let a: number[] = []; }
            }
        }
    "#,
        )
        .unwrap();
    for (position, expected_type) in [(0, Type::String), (1, Type::NumberArray)] {
        let mut program = original.clone();
        let TypedStatementKind::If { then, .. } = &mut program.functions[0].body[1].kind else {
            unreachable!()
        };
        let TypedStatementKind::While { body, .. } = &mut then[0].kind else {
            unreachable!()
        };
        let span = body[position].span;
        let TypedStatementKind::Let { value, .. } = &mut body[position].kind else {
            unreachable!()
        };
        *value = number("NaN");
        assert_eq!(
            compile(&program),
            Err(mismatch(expected_type, Type::Number, span))
        );
    }
}

#[test]
fn memory_handle_types_are_not_interchangeable_in_assignments_and_host_calls() {
    let hosts = [host(0, "text_host", &[Type::String], Type::String)];
    let compiler = compiler(OptimizationLevel::None, &hosts);
    for source in [
        "fn bad(s: string, a: number[]) -> void { s = s; }",
        "fn bad(s: string, a: number[]) -> void { text_host(s); }",
    ] {
        let mut program = compiler.analyze(source).unwrap();
        let span = program.functions[0].body[0].span;
        let a = &program.functions[0].params[1];
        let replacement = TypedExpr {
            kind: TypedExprKind::Identifier {
                id: a.id,
                name: a.name.clone(),
            },
            ty: Type::NumberArray,
        };
        match &mut program.functions[0].body[0].kind {
            TypedStatementKind::Assignment { value, .. } => *value = replacement,
            TypedStatementKind::Expression(TypedExpr {
                kind: TypedExprKind::Call { args, .. },
                ..
            }) => {
                args[0] = replacement;
            }
            _ => unreachable!(),
        }
        assert_eq!(
            compile(&program),
            Err(mismatch(Type::String, Type::NumberArray, span))
        );
    }
}

const NODE_PRELUDE: &str = r#"
const assert = require('node:assert/strict');
const bytes = require('node:fs').readFileSync(0);
const module_ = new WebAssembly.Module(bytes);
let e;
const view = () => new DataView(e.memory.buffer);
function descriptor(data, length) {
    const h = e.__alloc(8) >>> 0;
    view().setUint32(h, data, true);
    view().setUint32(h + 4, length, true);
    return h;
}
function string(value) {
    const bytes = new TextEncoder().encode(value);
    const data = e.__alloc(bytes.length) >>> 0;
    const h = descriptor(data, bytes.length);
    new Uint8Array(e.memory.buffer, data, bytes.length).set(bytes);
    return h;
}
function array(values) {
    const data = e.__alloc(values.length * 8) >>> 0;
    const h = descriptor(data, values.length);
    for (let i = 0; i < values.length; i++) view().setFloat64(data + i * 8, values[i], true);
    return h;
}
function readString(h) {
    const data = view().getUint32(h, true), length = view().getUint32(h + 4, true);
    return new TextDecoder('utf-8', {fatal: true}).decode(new Uint8Array(e.memory.buffer, data, length));
}
function readArray(h) {
    const data = view().getUint32(h, true), length = view().getUint32(h + 4, true);
    return Array.from({length}, (_, i) => view().getFloat64(data + i * 8, true));
}
function trap(f) { assert.throws(f, WebAssembly.RuntimeError); }
"#;

fn run_node(bytes: &[u8], script: &str) {
    let script = format!("{NODE_PRELUDE}\n{script}");
    let mut child = Command::new("node")
        .args(["-e", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Node.js");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(bytes)
        .expect("write Wasm");
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().expect("poll Node.js").is_none() {
        if Instant::now() >= deadline {
            child.kill().expect("kill timed out Node.js");
            let output = child.wait_with_output().unwrap();
            panic!(
                "Node.js exceeded 15 seconds: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("Node.js output");
    assert!(
        output.status.success(),
        "Node.js failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_at_all_levels(source: &str, hosts: &[HostFunction], script: &str) {
    let mut snapshot = None;
    for level in LEVELS {
        let bytes = compiler(level, hosts)
            .compile_source(source)
            .expect("compile memory runtime");
        decode(&bytes);
        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }
        run_node(&bytes, script);
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_arrays_sum_len_mutation_alias_reassignment_and_fresh_literals() {
    let source = r#"
        fn sum(a: number[]) -> number {
            let i: number = 0; let total: number = 0;
            while i < len(a) { total = total + a[i]; i = i + 1; }
            return total;
        }
        fn sample() -> number { return sum([10, 20, 30]); }
        fn empty() -> number[] { return []; }
        fn alias() -> number {
            let a: number[] = [1, 2]; let b: number[] = a;
            b[0] = 7; a = [9]; a[0] = 10;
            return b[0] + b[1] + a[0];
        }
        fn make() -> number[] { return [1, 2]; }
        fn repeat() -> void {
            let i: number = 0;
            while i < 4 { let a: number[] = [i]; seen(a); a[0] = 99; i = i + 1; }
        }
        fn recurse(n: number) -> number[] {
            if n <= 0 { return [1]; }
            let a: number[] = recurse(n - 1); a[0] = a[0] + 1; return a;
        }
        fn fresh_recursion(n: number) -> void {
            let a: number[] = [n]; seen(a);
            if n > 0 { fresh_recursion(n - 1); }
            a[0] = a[0] + 10;
        }
        fn nested() -> number { return sum(recurse(3)); }
    "#;
    run_at_all_levels(
        source,
        &[host(0, "seen", &[Type::NumberArray], Type::Void)],
        r#"
        const handles = [], values = [];
        e = new WebAssembly.Instance(module_, {env: {seen(h) {
            handles.push(h); values.push(readArray(h));
        }}}).exports;
        assert.equal(e.sample(), 60);
        assert.equal(e.sum(array([])), 0);
        const empty = e.empty();
        assert.deepEqual(readArray(empty), []);
        assert.equal(e.sum(empty), 0);
        assert.equal(e.alias(), 19);
        const first = e.make(), second = e.make();
        assert.notEqual(first, second);
        view().setFloat64(view().getUint32(first, true), 99, true);
        assert.deepEqual(readArray(second), [1, 2]);
        e.repeat();
        assert.deepEqual(values, [[0], [1], [2], [3]]);
        e.fresh_recursion(3);
        assert.deepEqual(values.slice(4), [[3], [2], [1], [0]]);
        assert.equal(new Set(handles).size, handles.length);
        for (const h of handles.slice(0, 4)) assert.deepEqual(readArray(h), [99]);
        assert.deepEqual(handles.slice(4).map(readArray), [[13], [12], [11], [10]]);
        assert.deepEqual(readArray(e.recurse(4)), [5]);
        assert.equal(e.nested(), 4);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_string_utf8_empty_nul_interning_and_distinct_descriptor_content_equality() {
    let source = "fn text() -> string { return \"é🙂\"; }\n\
        fn repeat() -> string { return \"é🙂\"; }\n\
        fn empty() -> string { return \"\"; }\n\
        fn nul() -> string { return \"A\0B\"; }\n\
        fn same(a: string, b: string) -> bool { return a == b; }\n\
        fn different(a: string, b: string) -> bool { return a != b; }\n\
        fn identity(a: string) -> string { return a; }";
    run_at_all_levels(
        source,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        assert.equal(readString(e.text()), 'é🙂');
        assert.equal(e.text(), e.repeat());
        assert.equal(view().getUint32(e.text() + 4, true), 6);
        assert.equal(readString(e.empty()), '');
        assert.equal(readString(e.nul()), 'A\0B');
        for (const value of ['', 'é🙂', 'A\0B']) {
            const a = string(value), b = string(value);
            assert.notEqual(a, b);
            assert.equal(e.same(a, b), 1);
            assert.equal(e.different(a, b), 0);
            assert.equal(e.identity(a), a);
        }
        assert.equal(e.same(string('é'), string('e')), 0);
        assert.equal(e.same(string('A\0B'), string('A\0C')), 0);
        assert.equal(e.same(string('A'), string('AA')), 0);
        assert.equal(e.different(string('é🙂'), e.empty()), 1);
        assert.equal(e.same(e.nul(), string('A\0B')), 1);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_all_invalid_indices_trap_and_negative_zero_is_valid_for_reads_and_writes() {
    run_at_all_levels(
        r#"
        fn get(a: number[], i: number) -> number { return a[i]; }
        fn set(a: number[], i: number, value: number) -> void { a[i] = value; }
    "#,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const a = array([10, 20]);
        for (const i of [NaN, Infinity, -Infinity, -1, -0.5, 0.5, 1.5, 2, 3, 4294967296, Number.MAX_VALUE]) {
            trap(() => e.get(a, i)); trap(() => e.set(a, i, 99));
            assert.deepEqual(readArray(a), [10, 20]);
        }
        assert.equal(e.get(a, -0), 10);
        e.set(a, -0, 7);
        assert.equal(e.get(a, 0), 7);
        e.set(a, 1, -Infinity);
        assert.equal(e.get(a, 1), -Infinity);
        const empty = array([]);
        trap(() => e.get(empty, 0)); trap(() => e.set(empty, -0, 99));
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_host_memory_roundtrips_reentrant_allocator_growth_and_reference_mutation() {
    let hosts = [
        host(0, "text_host", &[Type::String], Type::String),
        host(1, "array_host", &[Type::NumberArray], Type::NumberArray),
    ];
    run_at_all_levels(
        r#"
        fn text(a: string) -> string { return text_host(a); }
        fn values(a: number[]) -> number[] { return array_host(a); }
        fn sample() -> string { return text_host("guest"); }
    "#,
        &hosts,
        r#"
        let textCalls = 0, arrayCalls = 0;
        e = new WebAssembly.Instance(module_, {env: {
            text_host(h) {
                const value = readString(h), oldBuffer = e.memory.buffer;
                e.__alloc(131072);
                assert.notEqual(e.memory.buffer, oldBuffer);
                textCalls++;
                return string(value + 'é\0🙂');
            },
            array_host(h) {
                const values = readArray(h);
                e.__alloc(131072);
                const data = view().getUint32(h, true);
                if (values.length) view().setFloat64(data, 42, true);
                arrayCalls++;
                return array(values.map(x => x + 1));
            }
        }}).exports;
        assert.equal(readString(e.sample()), 'guesté\0🙂');
        assert.equal(readString(e.text(string('host'))), 'hosté\0🙂');
        assert.equal(readString(e.text(string(''))), 'é\0🙂');
        const a = array([1, 2, 3]);
        assert.deepEqual(readArray(e.values(a)), [2, 3, 4]);
        assert.deepEqual(readArray(a), [42, 2, 3]);
        assert.deepEqual(readArray(e.values(array([]))), []);
        assert.equal(textCalls, 3); assert.equal(arrayCalls, 2);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_allocator_alignment_reservation_growth_overflow_and_large_arrays() {
    run_at_all_levels(
        r#"
        fn len_array(a: number[]) -> number { return len(a); }
        fn get(a: number[], i: number) -> number { return a[i]; }
        fn marker() -> string { return "protected"; }
    "#,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const marker = e.marker();
        let previousEnd = marker + 8 + 9;
        for (const bytes of [0, 0, 1, 7, 8, 9, 63, 65536, 131072]) {
            const p = e.__alloc(bytes) >>> 0;
            assert.equal(p % 8, 0); assert.ok(p >= previousEnd);
            previousEnd = p + Math.max(8, Math.ceil(bytes / 8) * 8);
            assert.ok(previousEnd <= e.memory.buffer.byteLength);
        }
        assert.equal(readString(marker), 'protected');
        const values = Array.from({length: 20000}, (_, i) => i + 0.25);
        const a = array(values);
        assert.equal(e.len_array(a), 20000);
        assert.equal(e.get(a, 0), 0.25);
        assert.equal(e.get(a, 19999), 19999.25);
        const before = e.memory.buffer.byteLength;
        for (const bytes of [-1, -8, 0xfffffff0, 0xffff0000]) trap(() => e.__alloc(bytes));
        assert.equal(e.memory.buffer.byteLength, before);
        assert.equal(readString(marker), 'protected');
        assert.deepEqual(readArray(a), values);
        assert.ok((e.__alloc(8) >>> 0) >= previousEnd);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_guest_large_array_growth_and_bounded_allocator_exhaustion() {
    let elements = (0..9000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("fn large() -> number[] {{ return [{elements}]; }}");
    run_at_all_levels(
        &source,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const initial = e.memory.buffer.byteLength;
        const h = e.large();
        assert.ok(e.memory.buffer.byteLength > initial);
        const values = readArray(h);
        assert.equal(values.length, 9000);
        assert.equal(values[0], 0); assert.equal(values[8999], 8999);
    "#,
    );
    let bytes = compile_source("fn empty() -> number[] { return []; }");
    let initial = decode(&bytes).memories[0].0;
    let mut capped = wasm_encoder::Module::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        let payload = payload.unwrap();
        if let Some((id, range)) = payload.as_section() {
            if id == 5 {
                let mut memory = wasm_encoder::MemorySection::new();
                memory.memory(wasm_encoder::MemoryType {
                    minimum: initial,
                    maximum: Some(initial),
                    memory64: false,
                    shared: false,
                    page_size_log2: None,
                });
                capped.section(&memory);
            } else {
                let start = usize::try_from(range.start).expect("section start fits usize");
                let end = usize::try_from(range.end).expect("section end fits usize");
                capped.section(&wasm_encoder::RawSection {
                    id,
                    data: &bytes[start..end],
                });
            }
        }
    }
    let bytes = capped.finish();
    decode(&bytes);
    run_node(
        &bytes,
        r#"
        e = new WebAssembly.Instance(module_).exports;
        const p = e.__alloc(8) >>> 0, before = e.memory.buffer.byteLength;
        trap(() => e.__alloc(65536));
        assert.equal(e.memory.buffer.byteLength, before);
        assert.equal(e.__alloc(8) >>> 0, p + 8);
        assert.deepEqual(readArray(e.empty()), []);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_invalid_exported_and_host_descriptors_trap_before_effects() {
    let hosts = [
        host(0, "provide_string", &[], Type::String),
        host(1, "provide_array", &[], Type::NumberArray),
        host(2, "consume_string", &[Type::String], Type::Void),
        host(3, "consume_array", &[Type::NumberArray], Type::Void),
        host(4, "effect", &[], Type::Number),
    ];
    run_at_all_levels(
        r#"
        fn ignore_string(a: string) -> number { return effect(); }
        fn ignore_array(a: number[]) -> number { return effect(); }
        fn text() -> string { return provide_string(); }
        fn values() -> number[] { return provide_array(); }
        fn send_string(a: string) -> void { consume_string(a); }
        fn send_array(a: number[]) -> void { consume_array(a); }
        fn same(a: string, b: string) -> bool { return a == b; }
        fn unused_string_result() -> void { provide_string(); }
        fn unused_array_result() -> void { provide_array(); }
        fn after_mutation(a: number[]) -> void { effect(); consume_array(a); }
        fn after_string_mutation(a: string) -> void { effect(); consume_string(a); }
    "#,
        &hosts,
        r#"
        let supplied = 0, effects = 0, consumed = 0, mutate = 0;
        e = new WebAssembly.Instance(module_, {env: {
            provide_string() { return supplied; }, provide_array() { return supplied; },
            consume_string() { consumed++; }, consume_array() { consumed++; },
            effect() { effects++; if (mutate) view().setUint32(mutate + 4, 0xffffffff, true); return 1; }
        }}).exports;
        const end = e.memory.buffer.byteLength;
        const invalidHandles = [0, 1, 7, 9, -1, end, end - 4, end + 8];
        const badData = [
            descriptor(0, 0), descriptor(7, 1),
            descriptor(end + 8, 0), descriptor(end, 1), descriptor(end - 1, 2),
            descriptor(0xfffffff8, 16), descriptor(8, 0xffffffff),
        ];
        const validString = string('valid');
        for (const h of [...invalidHandles, ...badData]) {
            supplied = h;
            trap(() => e.ignore_string(h)); trap(() => e.ignore_array(h));
            trap(() => e.text()); trap(() => e.values());
            trap(() => e.send_string(h)); trap(() => e.send_array(h));
            trap(() => e.same(h, h)); trap(() => e.same(validString, h));
            trap(() => e.unused_string_result()); trap(() => e.unused_array_result());
        }
        const unalignedArray = descriptor(17, 1);
        supplied = unalignedArray;
        trap(() => e.ignore_array(unalignedArray)); trap(() => e.values());
        trap(() => e.send_array(unalignedArray));
        trap(() => e.ignore_array(descriptor(17, 0)));
        assert.equal(e.same(unalignedArray, unalignedArray), 1);
        assert.equal(effects, 0); assert.equal(consumed, 0);
        assert.equal(e.ignore_string(descriptor(end, 0)), 1);
        assert.equal(e.ignore_array(descriptor(end, 0)), 1);
        const a = array([1]); mutate = a;
        trap(() => e.after_mutation(a));
        assert.equal(effects, 3); assert.equal(consumed, 0);
        mutate = string('safe');
        trap(() => e.after_string_mutation(mutate));
        assert.equal(effects, 4); assert.equal(consumed, 0);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_memory_expression_order_live_prefixes_and_assignment_bounds_before_rhs() {
    let hosts = [
        host(0, "mark", &[Type::Number], Type::Number),
        host(1, "base", &[], Type::NumberArray),
        host(2, "position", &[], Type::Number),
        host(3, "rhs", &[], Type::Number),
    ];
    run_at_all_levels(
        r#"
        fn identity(a: number[]) -> number[] { return a; }
        fn combine(n: number, a: number[], s: string, b: bool) -> number {
            if b { return n + len(a); } else { return n; }
        }
        fn build() -> number[] { return [mark(1), mark(2), mark(3)]; }
        fn nested_array() -> number[] { return [len([mark(4), mark(5)]), mark(6)]; }
        fn read() -> number { let a: number[] = base(); return a[position()]; }
        fn write(a: number[]) -> void { a[position()] = rhs(); }
        fn live(a: number[]) -> number {
            return mark(10) + combine(mark(20), identity([mark(1), len(identity(a))]), "x", "x" == "x");
        }
    "#,
        &hosts,
        r#"
        const events = [];
        let supplied, position = 0, replacement = 0;
        e = new WebAssembly.Instance(module_, {env: {
            mark(n) { events.push(n); return n; },
            base() { events.push('base'); return supplied; },
            position() { events.push('index'); return position; },
            rhs() {
                events.push('rhs');
                if (replacement) view().setUint32(supplied, replacement, true);
                e.__alloc(131072);
                return 77;
            }
        }}).exports;
        assert.deepEqual(readArray(e.build()), [1, 2, 3]);
        assert.deepEqual(events.splice(0), [1, 2, 3]);
        assert.deepEqual(readArray(e.nested_array()), [2, 6]);
        assert.deepEqual(events.splice(0), [4, 5, 6]);
        supplied = array([4, 5]);
        assert.equal(e.read(), 4);
        assert.deepEqual(events.splice(0), ['base', 'index']);
        position = 2;
        trap(() => e.write(supplied));
        assert.deepEqual(events.splice(0), ['index']);
        position = 1;
        const originalData = view().getUint32(supplied, true);
        replacement = e.__alloc(16) >>> 0;
        view().setFloat64(replacement + 8, 123, true);
        e.write(supplied);
        assert.deepEqual(events.splice(0), ['index', 'rhs']);
        assert.equal(view().getFloat64(originalData + 8, true), 77);
        assert.equal(view().getFloat64(replacement + 8, true), 123);
        assert.equal(e.live(array([9, 8, 7])), 32);
        assert.deepEqual(events.splice(0), [10, 20, 1]);
    "#,
    );
}
