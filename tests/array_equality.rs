#[path = "support/process.rs"]
mod test_process;

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use was_compiler::{
    CodegenError, Compiler, CompilerOptions, OptimizationLevel, Type,
    ast::BinaryOp,
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, ImportSection, Instruction,
    Module, TypeSection, ValType,
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];
const NODE_TIMEOUT: Duration = Duration::from_secs(30);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

const SOURCE: &str = "
    fn same(a: number[], b: number[]) -> bool { return a == b; }
    fn different(a: number[], b: number[]) -> bool { return a != b; }
    fn literal_same() -> bool { return [1, 2, 3] == [1, 2, 3]; }
    fn literal_shorter() -> bool { return [1, 2, 3] == [1, 2]; }
    fn literal_different() -> bool { return [1, 2, 3] != [1, 2]; }
    fn literal_not_different() -> bool { return [1, 2, 3] != [1, 2, 3]; }
    fn empty() -> bool { return [] == []; }
    fn items() -> number[] { return [1, 2, 3]; }
    fn identity(a: number[]) -> number[] { return a; }
    fn edit(a: number[], value: number) -> number[] { a[1] = value; return a; }
    fn alias_edit(a: number[], b: number[]) -> bool {
        let alias: number[] = a;
        alias[1] = 9;
        return a == b;
    }
    fn guest_nan() -> bool { let a: number[] = [0 / 0]; return a == a; }
    fn guest_nan_different() -> bool { let a: number[] = [0 / 0]; return a != a; }
    fn guest_zero() -> bool { return [0, -0] == [-0, 0]; }
    fn guest_infinity() -> bool { return [1 / 0, -1 / 0] == [1 / 0, -1 / 0]; }
";

fn compiler(level: OptimizationLevel) -> Compiler {
    Compiler::with_options(CompilerOptions {
        optimization_level: level,
    })
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let bytes = compiler
        .compile_source(source)
        .expect("compile array equality");
    Validator::new()
        .validate_all(&bytes)
        .expect("valid array equality WASM");
    bytes
}

#[test]
fn array_equality_validates_and_is_deterministic_without_mutating_typed_input() {
    let mut expected = None;
    for level in LEVELS {
        let compiler = compiler(level);
        let program = compiler.analyze(SOURCE).unwrap();
        let original = program.clone();
        let bytes = compile(&compiler, SOURCE);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        assert_eq!(compiler.compile_source(SOURCE).unwrap(), bytes);
        assert_eq!(program, original);
        if let Some(expected) = &expected {
            assert_eq!(&bytes, expected, "level {level:?}");
        } else {
            expected = Some(bytes);
        }
    }
}

#[test]
fn array_equality_helper_is_appended_without_moving_imports_users_or_existing_helpers() {
    let source = "
        fn same(a: number[], b: number[]) -> bool { return a == b; }
        fn different(a: number[], b: number[]) -> bool { return a != b; }
        fn text(a: string, b: string) -> bool { return a == b; }
        fn echo(a: number[]) -> number[] { return host_array(a); }
        fn at(a: number[], i: number) -> number { return a[i]; }
        fn size(a: number[]) -> number { return len(a) + host_number(1); }
    ";
    for level in LEVELS {
        let mut compiler = compiler(level);
        compiler
            .register_host_function(
                "host_array",
                "env",
                "array",
                vec![Type::NumberArray],
                Type::NumberArray,
            )
            .unwrap();
        compiler
            .register_host_function(
                "host_number",
                "env",
                "number",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();
        let bytes = compile(&compiler, source);
        let mut signatures = Vec::new();
        let mut functions = Vec::new();
        let mut exports = Vec::new();
        let mut imports = Vec::new();
        let mut calls = Vec::new();
        let mut eqz = Vec::new();
        let mut float_comparisons = Vec::new();
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                Payload::TypeSection(section) => {
                    for signature in section.into_iter_err_on_gc_types() {
                        let signature = signature.unwrap();
                        signatures
                            .push((signature.params().to_vec(), signature.results().to_vec()));
                    }
                }
                Payload::ImportSection(section) => {
                    for import in section.into_imports() {
                        let import = import.unwrap();
                        imports.push((import.module.to_string(), import.name.to_string()));
                    }
                }
                Payload::FunctionSection(section) => {
                    functions.extend(section.into_iter().map(Result::unwrap))
                }
                Payload::ExportSection(section) => {
                    for export in section {
                        let export = export.unwrap();
                        exports.push((export.name.to_string(), export.kind, export.index));
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    let mut body_calls = Vec::new();
                    let mut body_eqz = 0;
                    let mut comparisons = 0;
                    for op in body.get_operators_reader().unwrap() {
                        match op.unwrap() {
                            Operator::Call { function_index } => body_calls.push(function_index),
                            Operator::I32Eqz => body_eqz += 1,
                            Operator::F64Eq | Operator::F64Ne => comparisons += 1,
                            _ => {}
                        }
                    }
                    calls.push(body_calls);
                    eqz.push(body_eqz);
                    float_comparisons.push(comparisons);
                }
                Payload::CustomSection(section) => {
                    assert_ne!(section.name(), "was-compiler.interface")
                }
                _ => {}
            }
        }
        assert_eq!(
            imports,
            [
                ("env".into(), "array".into()),
                ("env".into(), "number".into())
            ]
        );
        assert_eq!(functions.len(), 13);
        assert_eq!(calls.len(), 13);
        for (index, name) in ["same", "different", "text", "echo", "at", "size"]
            .iter()
            .enumerate()
        {
            assert!(exports.contains(&(name.to_string(), ExternalKind::Func, index as u32 + 2)));
        }
        assert_eq!(exports.len(), 8);
        assert!(exports.contains(&("memory".into(), ExternalKind::Memory, 0)));
        assert!(exports.contains(&("__alloc".into(), ExternalKind::Func, 8)));
        let helper_signatures = [
            (
                vec![wasmparser::ValType::I32],
                vec![wasmparser::ValType::I32],
            ),
            (
                vec![wasmparser::ValType::I32],
                vec![wasmparser::ValType::I32],
            ),
            (
                vec![wasmparser::ValType::I32],
                vec![wasmparser::ValType::I32],
            ),
            (
                vec![wasmparser::ValType::I32, wasmparser::ValType::F64],
                vec![wasmparser::ValType::I32],
            ),
            (
                vec![wasmparser::ValType::I32],
                vec![wasmparser::ValType::F64],
            ),
            (
                vec![wasmparser::ValType::I32, wasmparser::ValType::I32],
                vec![wasmparser::ValType::I32],
            ),
            (
                vec![wasmparser::ValType::I32, wasmparser::ValType::I32],
                vec![wasmparser::ValType::I32],
            ),
        ];
        for (type_index, signature) in functions[6..].iter().zip(helper_signatures) {
            assert_eq!(signatures[*type_index as usize], signature);
        }
        assert_eq!(functions[11], functions[12]);
        assert_eq!(calls[0], [10, 10, 14]);
        assert_eq!(calls[1], [10, 10, 14]);
        assert_eq!(eqz[0], 0);
        assert_eq!(eqz[1], 1);
        assert!(calls[2].contains(&13));
        assert!(calls[3].contains(&0));
        assert!(calls[4].contains(&11));
        assert!(calls[5].contains(&12));
        assert!(calls[5].contains(&1));
        assert_eq!(calls[12], [10, 10]);
        assert!(float_comparisons[12] > 0);
    }
}

#[test]
fn scalar_only_equality_keeps_the_exact_module_bytes_without_memory_helpers() {
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::F64, ValType::F64], [ValType::I32]);
    let imports = ImportSection::new();
    let mut functions = FunctionSection::new();
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("same", ExportKind::Func, 0);
    let mut body = Function::new([]);
    for instruction in [
        Instruction::LocalGet(0),
        Instruction::LocalGet(1),
        Instruction::F64Eq,
        Instruction::Return,
        Instruction::End,
    ] {
        body.instruction(&instruction);
    }
    let mut code = CodeSection::new();
    code.function(&body);
    let mut expected = Module::new();
    expected
        .section(&types)
        .section(&imports)
        .section(&functions)
        .section(&exports)
        .section(&code);
    let expected = expected.finish();
    for level in LEVELS {
        assert_eq!(
            compile(
                &compiler(level),
                "fn same(a: number, b: number) -> bool { return a == b; }"
            ),
            expected
        );
    }
}

fn array() -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Array(Vec::new()),
        ty: Type::NumberArray,
    }
}

fn value(ty: Type) -> TypedExpr {
    let kind = match ty {
        Type::Number => TypedExprKind::Number("1".into()),
        Type::String => TypedExprKind::String("text".into()),
        Type::NumberArray => return array(),
        Type::Bool | Type::Void => TypedExprKind::Bool(false),
    };
    TypedExpr { kind, ty }
}

fn binary(left: TypedExpr, op: BinaryOp, right: TypedExpr, ty: Type) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
        ty,
    }
}

fn assert_forged_error(expr: TypedExpr, expected: CodegenError, span: Span) {
    for level in LEVELS {
        let compiler = compiler(level);
        for context in 0..6 {
            let mut program = compiler.analyze("fn test() -> void {}").unwrap();
            let expr = match context {
                2 => binary(value(Type::Bool), BinaryOp::And, expr.clone(), Type::Bool),
                3 => binary(
                    TypedExpr {
                        kind: TypedExprKind::Bool(true),
                        ty: Type::Bool,
                    },
                    BinaryOp::Or,
                    expr.clone(),
                    Type::Bool,
                ),
                _ => expr.clone(),
            };
            let statement = TypedStatement {
                kind: TypedStatementKind::Expression(expr),
                span,
            };
            let body = &mut program.functions[0].body;
            if context == 1 {
                body.push(TypedStatement {
                    kind: TypedStatementKind::Return(None),
                    span: Span::new(1, 8),
                });
            }
            body.push(match context {
                4 => TypedStatement {
                    kind: TypedStatementKind::If {
                        cond: Box::new(value(Type::Bool)),
                        then: vec![statement],
                        else_: None,
                    },
                    span: Span::new(10, 90),
                },
                5 => TypedStatement {
                    kind: TypedStatementKind::While {
                        cond: Box::new(value(Type::Bool)),
                        body: vec![statement],
                    },
                    span: Span::new(10, 90),
                },
                _ => statement,
            });
            assert_eq!(
                compiler.compile(&program).unwrap_err(),
                expected,
                "level {level:?}, context {context}"
            );
        }
    }
}

#[test]
fn forged_array_equality_rejects_result_and_mixed_operand_types_with_statement_spans() {
    let span = Span::new(31, 67);
    for op in [BinaryOp::Equal, BinaryOp::NotEqual] {
        for found in [Type::Number, Type::String, Type::NumberArray, Type::Void] {
            assert_forged_error(
                binary(array(), op, array(), found),
                CodegenError::TypeMismatch {
                    function: "test".into(),
                    expected: Type::Bool,
                    found,
                    span,
                },
                span,
            );
        }
        for scalar in [Type::Number, Type::String, Type::Bool, Type::Void] {
            assert_forged_error(
                binary(array(), op, value(scalar), Type::Bool),
                CodegenError::TypeMismatch {
                    function: "test".into(),
                    expected: Type::NumberArray,
                    found: scalar,
                    span,
                },
                span,
            );
            assert_forged_error(
                binary(value(scalar), op, array(), Type::Bool),
                CodegenError::TypeMismatch {
                    function: "test".into(),
                    expected: if scalar == Type::Void {
                        Type::Number
                    } else {
                        scalar
                    },
                    found: if scalar == Type::Void {
                        Type::Void
                    } else {
                        Type::NumberArray
                    },
                    span,
                },
                span,
            );
        }
    }
}

#[test]
fn forged_array_equality_checks_nested_elements_even_when_unreachable_or_skipped() {
    let span = Span::new(42, 78);
    for op in [BinaryOp::Equal, BinaryOp::NotEqual] {
        let bad_array = TypedExpr {
            kind: TypedExprKind::Array(vec![value(Type::Bool)]),
            ty: Type::NumberArray,
        };
        for expr in [
            binary(bad_array.clone(), op, array(), Type::Bool),
            binary(array(), op, bad_array, Type::Bool),
        ] {
            assert_forged_error(
                expr,
                CodegenError::TypeMismatch {
                    function: "test".into(),
                    expected: Type::Number,
                    found: Type::Bool,
                    span,
                },
                span,
            );
        }
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "was-compiler-array-equality-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create array equality test directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_node_at_all_levels(source: &str, hosts: &[(&str, Vec<Type>, Type)], body: &str) {
    let scratch = Scratch::new();
    let mut command = Command::new("node");
    command.arg("-e").arg(format!("{NODE_PRELUDE}\nfor (const file of process.argv.slice(1)) {{\nconst module_ = new WebAssembly.Module(fs.readFileSync(file));\nlet e;\n{body}\n}}"));
    for (index, level) in LEVELS.into_iter().enumerate() {
        let mut compiler = compiler(level);
        for (name, params, result) in hosts {
            compiler
                .register_host_function(*name, "env", *name, params.clone(), *result)
                .unwrap();
        }
        let path = scratch.0.join(format!("level-{index}.wasm"));
        fs::write(&path, compile(&compiler, source)).unwrap();
        command.arg(path);
    }
    let std::process::Output {
        status,
        stdout,
        stderr,
    } = test_process::output(&mut command, None, NODE_TIMEOUT);
    assert!(
        status.success(),
        "Node.js failed:\n{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    assert!(stdout.is_empty());
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
}

const NODE_PRELUDE: &str = r#"
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
function view(e) { return new DataView(e.memory.buffer); }
function descriptor(e, data, length) {
    const handle = e.__alloc(8) >>> 0;
    view(e).setUint32(handle, data, true);
    view(e).setUint32(handle + 4, length, true);
    return handle;
}
function array(e, values) {
    const handle = e.__alloc(8 + values.length * 8) >>> 0;
    view(e).setUint32(handle, handle + 8, true);
    view(e).setUint32(handle + 4, values.length, true);
    values.forEach((value, index) => view(e).setFloat64(handle + 8 + index * 8, value, true));
    return handle;
}
function read(e, handle) {
    const data = view(e).getUint32(handle, true);
    const length = view(e).getUint32(handle + 4, true);
    return Array.from({length}, (_, index) => view(e).getFloat64(data + index * 8, true));
}
function trap(action) { assert.throws(action, WebAssembly.RuntimeError); }
"#;

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_array_content_equality_ieee754_aliases_and_no_mutation_at_all_levels() {
    run_node_at_all_levels(
        SOURCE,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        assert.equal(e.literal_same(), 1);
        assert.equal(e.literal_shorter(), 0);
        assert.equal(e.literal_different(), 1);
        assert.equal(e.literal_not_different(), 0);
        assert.equal(e.empty(), 1);
        assert.equal(e.guest_nan(), 0);
        assert.equal(e.guest_nan_different(), 1);
        assert.equal(e.guest_zero(), 1);
        assert.equal(e.guest_infinity(), 1);
        const cases = [
            [[], [], true], [[], [1], false], [[1], [], false],
            [[1, 2, 3], [1, 2, 3], true], [[1, 2], [1, 2, 3], false],
            [[1, 2, 3], [1, 2], false], [[9, 2, 3], [1, 2, 3], false],
            [[1, 9, 3], [1, 2, 3], false], [[1, 2, 9], [1, 2, 3], false],
            [[1, 2, 3], [3, 2, 1], false], [[0, -0], [-0, 0], true],
            [[NaN], [NaN], false], [[1, NaN], [1, NaN], false],
            [[Infinity, -Infinity], [Infinity, -Infinity], true],
            [[Infinity], [-Infinity], false], [[Infinity], [Number.MAX_VALUE], false],
            [[Number.MIN_VALUE, -Number.MAX_VALUE], [Number.MIN_VALUE, -Number.MAX_VALUE], true],
        ];
        for (const [left, right, expected] of cases) {
            const a = array(e, left), b = array(e, right);
            const before = Buffer.from(new Uint8Array(e.memory.buffer));
            for (const [x, y] of [[a, b], [b, a]]) {
                assert.equal(e.same(x, y), Number(expected));
                assert.equal(e.different(x, y), Number(!expected));
            }
            assert.deepEqual(Buffer.from(e.memory.buffer), before);
            assert.deepEqual(read(e, a), left);
            assert.deepEqual(read(e, b), right);
            assert.equal(e.same(a, a), Number(!left.some(Number.isNaN)));
            assert.equal(e.different(a, a), Number(left.some(Number.isNaN)));
        }
        const a = e.items(), b = e.items();
        assert.notEqual(a, b);
        assert.equal(e.identity(a), a);
        assert.equal(e.same(a, b), 1);
        assert.equal(e.alias_edit(a, b), 0);
        assert.deepEqual(read(e, a), [1, 9, 3]);
        assert.deepEqual(read(e, b), [1, 2, 3]);
        assert.equal(e.different(a, b), 1);
        assert.equal(e.edit(b, 9), b);
        assert.equal(e.same(a, b), 1);
        assert.equal(e.different(a, b), 0);
        const shared = descriptor(e, view(e).getUint32(a, true), 3);
        assert.equal(e.same(a, shared), 1);
        const end = e.memory.buffer.byteLength;
        assert.equal(e.same(descriptor(e, end, 0), array(e, [])), 1);
        const values = Array.from({length: 20000}, (_, index) => index + 0.25);
        const beforeGrowth = e.memory.buffer.byteLength;
        const largeLeft = array(e, values), largeRight = array(e, values);
        assert.ok(e.memory.buffer.byteLength > beforeGrowth);
        assert.equal(e.same(largeLeft, largeRight), 1);
        assert.equal(e.different(largeLeft, largeRight), 0);
        assert.equal(e.same(largeLeft, a), 0);
        const largeData = view(e).getUint32(largeRight, true);
        view(e).setFloat64(largeData + (values.length - 1) * 8, -1, true);
        assert.equal(e.same(largeLeft, largeRight), 0);
        assert.equal(e.different(largeLeft, largeRight), 1);
        assert.deepEqual(read(e, largeLeft), values);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_array_equality_evaluates_once_left_to_right_and_observes_current_storage() {
    let hosts = [
        ("mark", vec![Type::Number], Type::Number),
        ("provide", vec![Type::Number], Type::NumberArray),
        (
            "wrap",
            vec![Type::NumberArray, Type::Number],
            Type::NumberArray,
        ),
        ("right", vec![], Type::NumberArray),
    ];
    let source = "
        fn ordered() -> bool { return wrap(provide(mark(1)), mark(2)) == wrap(provide(mark(3)), mark(4)); }
        fn ordered_different() -> bool { return wrap(provide(mark(1)), mark(2)) != wrap(provide(mark(3)), mark(4)); }
        fn current() -> bool { return provide(mark(1)) == right(); }
        fn current_different() -> bool { return provide(mark(1)) != right(); }
        fn nested() -> bool { return (provide(mark(1)) == provide(mark(3))) == (provide(mark(1)) != provide(mark(3))); }
    ";
    run_node_at_all_levels(
        source,
        &hosts,
        r#"
        const events = [];
        let left, rhs, change = () => {};
        e = new WebAssembly.Instance(module_, {env: {
            mark(n) { events.push(`mark${n}`); return n; },
            provide(n) { events.push(`provide${n}`); return n === 1 ? left : rhs; },
            wrap(a, n) { events.push(`wrap${n}`); return a; },
            right() { events.push('right'); change(); return rhs; },
        }}).exports;
        const order = ['mark1', 'provide1', 'mark2', 'wrap2', 'mark3', 'provide3', 'mark4', 'wrap4'];
        for (const [a, b, expected] of [[[1, 2], [1, 2], 1], [[1], [1, 2], 0], [[], [], 1], [[NaN], [NaN], 0]]) {
            left = array(e, a); rhs = array(e, b);
            for (const [fn, result] of [[e.ordered, expected], [e.ordered_different, 1 - expected]]) {
                events.length = 0;
                assert.equal(fn(), result);
                assert.deepEqual(events, order);
            }
        }
        left = array(e, [1, 2]); rhs = array(e, [1, 9]);
        change = () => view(e).setFloat64(view(e).getUint32(left, true) + 8, 9, true);
        events.length = 0;
        assert.equal(e.current(), 1);
        assert.deepEqual(events, ['mark1', 'provide1', 'right']);
        left = array(e, [1, 2]);
        events.length = 0;
        assert.equal(e.current_different(), 0);
        assert.deepEqual(events, ['mark1', 'provide1', 'right']);
        left = array(e, [1]); rhs = array(e, [1]);
        change = () => view(e).setFloat64(view(e).getUint32(left, true), 2, true);
        assert.equal(e.current(), 0);
        left = array(e, [1]);
        assert.equal(e.current_different(), 1);
        left = array(e, [5, 6, 7]); rhs = array(e, [8]);
        change = () => {
            view(e).setUint32(left, view(e).getUint32(rhs, true), true);
            view(e).setUint32(left + 4, 1, true);
        };
        assert.equal(e.current(), 1);
        assert.deepEqual(read(e, left), [8]);
        left = array(e, [1]); rhs = left;
        change = () => view(e).setFloat64(view(e).getUint32(left, true), NaN, true);
        assert.equal(e.current(), 0);
        left = array(e, [1]); rhs = left;
        assert.equal(e.current_different(), 1);
        left = array(e, [1]); rhs = array(e, [1]);
        events.length = 0;
        assert.equal(e.nested(), 0);
        assert.deepEqual(events, ['mark1', 'provide1', 'mark3', 'provide3', 'mark1', 'provide1', 'mark3', 'provide3']);
    "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_array_equality_traps_on_both_malformed_descriptors_before_shortcuts() {
    let hosts = [
        ("left", vec![], Type::NumberArray),
        ("right", vec![], Type::NumberArray),
    ];
    let source = "
        fn same(a: number[], b: number[]) -> bool { return a == b; }
        fn different(a: number[], b: number[]) -> bool { return a != b; }
        fn callback_same() -> bool { return left() == right(); }
        fn callback_different() -> bool { return left() != right(); }
    ";
    run_node_at_all_levels(
        source,
        &hosts,
        r#"
        let suppliedLeft, suppliedRight, change = () => {};
        const events = [];
        e = new WebAssembly.Instance(module_, {env: {
            left() { events.push('left'); return suppliedLeft; },
            right() { events.push('right'); change(); return suppliedRight; },
        }}).exports;
        const end = e.memory.buffer.byteLength;
        const invalid = [
            0, 1, 7, 9, -1, end, end - 4, end + 8,
            descriptor(e, 0, 0), descriptor(e, 7, 0), descriptor(e, 9, 0),
            descriptor(e, end + 8, 0), descriptor(e, end, 1),
            descriptor(e, end - 8, 2), descriptor(e, 0xfffffff8, 2),
            descriptor(e, 8, 0xffffffff), descriptor(e, 8, 0x20000000),
        ];
        const valid = [array(e, []), array(e, [1]), array(e, [1, 2])];
        for (const bad of invalid) {
            for (const fn of [e.same, e.different]) {
                trap(() => fn(bad, bad));
                for (const good of valid) {
                    trap(() => fn(bad, good));
                    trap(() => fn(good, bad));
                }
            }
            suppliedLeft = bad; suppliedRight = valid[0];
            events.length = 0;
            trap(() => e.callback_same());
            assert.deepEqual(events, ['left']);
            suppliedLeft = valid[0]; suppliedRight = bad;
            events.length = 0;
            trap(() => e.callback_different());
            assert.deepEqual(events, ['left', 'right']);
        }
        const corruptions = [
            handle => view(e).setUint32(handle, 0, true),
            handle => view(e).setUint32(handle, 9, true),
            handle => view(e).setUint32(handle, end + 8, true),
            handle => view(e).setUint32(handle + 4, 0xffffffff, true),
            handle => { view(e).setUint32(handle, end - 8, true); view(e).setUint32(handle + 4, 2, true); },
        ];
        for (const fn of [e.callback_same, e.callback_different]) {
            for (const corrupt of corruptions) {
                for (const [a, b] of [[[], []], [[], [1]], [[1], []], [[1], [1, 2]], [[1, 2], [1]]]) {
                    suppliedLeft = array(e, a); suppliedRight = array(e, b);
                    change = () => corrupt(suppliedLeft);
                    events.length = 0;
                    trap(fn);
                    assert.deepEqual(events, ['left', 'right']);
                    suppliedLeft = array(e, a); suppliedRight = array(e, b);
                    change = () => corrupt(suppliedRight);
                    events.length = 0;
                    trap(fn);
                    assert.deepEqual(events, ['left', 'right']);
                }
                suppliedLeft = array(e, []); suppliedRight = suppliedLeft;
                change = () => corrupt(suppliedLeft);
                events.length = 0;
                trap(fn);
                assert.deepEqual(events, ['left', 'right']);
            }
        }
        change = () => {};
        suppliedLeft = array(e, [1]); suppliedRight = array(e, [1]);
        assert.equal(e.callback_same(), 1);
        assert.equal(e.callback_different(), 0);
    "#,
    );
}
