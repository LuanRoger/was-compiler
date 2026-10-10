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
    CodegenError, Compiler, CompilerOptions, ExportOptions, OptimizationLevel, Type,
    ast::{BinaryOp, UnaryOp},
    tokens::Span,
    typed_ast::{TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, ImportSection, Instruction,
    Module, TypeSection, ValType,
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, Validator};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];
const NODE_TIMEOUT: Duration = Duration::from_secs(30);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

const SCALAR_SOURCE: &str = "
    fn rem(a: number, b: number) -> number { return a % b; }
    fn nested(a: number, b: number) -> number { return (a % b) % (b % 3); }
    fn arithmetic(a: number, b: number) -> number { return -a % b * 2 + 9 / 3 - 1; }
    fn local(a: number, b: number) -> number {
        let result: number = a % b;
        result = (result + a) % b;
        return result;
    }
    fn flow(a: number, b: number) -> number {
        let result: number = 0;
        let i: number = 0;
        while i < 4 {
            if i % 2 == 0 { result = result + a % b; }
            else { result = result - a % b; }
            i = i + 1;
        }
        return result;
    }
    fn indirect(a: number, b: number) -> number { return rem(a, b); }
    fn discarded(a: number, b: number) -> void { a % b; }
    fn unreachable(a: number, b: number) -> number { return a; return a % b; }
    fn skipped(a: number, b: number) -> bool { return false && a % b == 0; }
";

fn compiler(level: OptimizationLevel) -> Compiler {
    Compiler::with_options(CompilerOptions {
        optimization_level: level,
    })
}

fn compile(compiler: &Compiler, source: &str) -> Vec<u8> {
    let bytes = compiler.compile_source(source).expect("compile remainder");
    Validator::new()
        .validate_all(&bytes)
        .expect("valid remainder WASM");
    bytes
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DecodedModule {
    signatures: Vec<(Vec<wasmparser::ValType>, Vec<wasmparser::ValType>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    calls: Vec<Vec<u32>>,
    bodies: Vec<Vec<u8>>,
    sections: Vec<(u8, Vec<u8>)>,
}

fn decode(bytes: &[u8]) -> DecodedModule {
    Validator::new().validate_all(bytes).expect("valid WASM");
    let mut module = DecodedModule::default();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.unwrap();
        if let Some((id, range)) = payload.as_section() {
            assert_ne!(id, 0, "no backend metadata");
            if id != 7 {
                let start = usize::try_from(range.start).unwrap();
                let end = usize::try_from(range.end).unwrap();
                module.sections.push((id, bytes[start..end].to_vec()));
            }
        }
        match payload {
            Payload::TypeSection(section) => {
                for signature in section.into_iter_err_on_gc_types() {
                    let signature = signature.unwrap();
                    module
                        .signatures
                        .push((signature.params().to_vec(), signature.results().to_vec()));
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import.unwrap();
                    let TypeRef::Func(index) = import.ty else {
                        panic!("only explicitly registered function imports are allowed");
                    };
                    module.imports.push((
                        import.module.to_string(),
                        import.name.to_string(),
                        index,
                    ));
                }
            }
            Payload::FunctionSection(section) => {
                module
                    .functions
                    .extend(section.into_iter().map(Result::unwrap));
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.unwrap();
                    module
                        .exports
                        .push((export.name.to_string(), export.kind, export.index));
                }
            }
            Payload::CodeSectionEntry(body) => {
                module.bodies.push(body.as_bytes().to_vec());
                let mut calls = Vec::new();
                for operator in body.get_operators_reader().unwrap() {
                    if let Operator::Call { function_index } = operator.unwrap() {
                        calls.push(function_index);
                    }
                }
                module.calls.push(calls);
            }
            _ => {}
        }
    }
    assert_eq!(module.functions.len(), module.bodies.len());
    module
}

fn numeric_signature() -> (Vec<wasmparser::ValType>, Vec<wasmparser::ValType>) {
    (
        vec![wasmparser::ValType::F64, wasmparser::ValType::F64],
        vec![wasmparser::ValType::F64],
    )
}

fn export(name: &str, index: u32) -> (String, ExternalKind, u32) {
    (name.into(), ExternalKind::Func, index)
}

#[test]
fn remainder_validates_deterministically_at_all_levels_without_mutating_typed_input() {
    let mut expected = None;
    for level in LEVELS {
        let compiler = compiler(level);
        let program = compiler.analyze(SCALAR_SOURCE).unwrap();
        let original = program.clone();
        let bytes = compile(&compiler, SCALAR_SOURCE);
        assert_eq!(compiler.compile(&program).unwrap(), bytes);
        assert_eq!(compiler.compile_source(SCALAR_SOURCE).unwrap(), bytes);
        assert_eq!(program, original);
        if let Some(expected) = &expected {
            assert_eq!(&bytes, expected, "level {level:?}");
        } else {
            expected = Some(bytes);
        }
    }
}

#[test]
fn scalar_remainder_appends_one_private_signature_deduplicated_helper_without_memory_or_imports() {
    for level in LEVELS {
        let module = decode(&compile(&compiler(level), SCALAR_SOURCE));
        assert!(module.imports.is_empty());
        assert!(
            module
                .sections
                .iter()
                .all(|(id, _)| ![5, 6, 11, 12].contains(id))
        );
        assert_eq!(module.functions.len(), 10);
        assert_eq!(module.exports.len(), 9);
        for (index, name) in [
            "rem",
            "nested",
            "arithmetic",
            "local",
            "flow",
            "indirect",
            "discarded",
            "unreachable",
            "skipped",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(module.exports[index], export(name, index as u32));
        }
        assert_eq!(module.functions[9], module.functions[0]);
        assert_eq!(
            module.signatures[module.functions[9] as usize],
            numeric_signature()
        );
        assert_eq!(
            module
                .signatures
                .iter()
                .filter(|signature| **signature == numeric_signature())
                .count(),
            1
        );
        assert_eq!(module.calls[0], [9]);
        assert_eq!(module.calls[1], [9, 9, 9]);
        assert_eq!(module.calls[2], [9]);
        assert_eq!(module.calls[3], [9, 9]);
        assert_eq!(module.calls[4], [9, 9, 9]);
        assert_eq!(module.calls[5], [0]);
        assert_eq!(module.calls[6], [9]);
        assert_eq!(module.calls[8], [9]);
        assert!(
            module.calls[9].is_empty(),
            "numeric helper must be guest-only"
        );
    }
}

#[test]
fn no_remainder_preserves_exact_scalar_module_bytes() {
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::F64, ValType::F64], [ValType::F64]);
    let imports = ImportSection::new();
    let mut functions = FunctionSection::new();
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("add", ExportKind::Func, 0);
    let mut body = Function::new([]);
    for instruction in [
        Instruction::LocalGet(0),
        Instruction::LocalGet(1),
        Instruction::F64Add,
        Instruction::Return,
        Instruction::End,
    ] {
        body.instruction(&instruction);
    }
    let mut code = CodeSection::new();
    code.function(&body);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&imports)
        .section(&functions)
        .section(&exports)
        .section(&code);
    let expected = module.finish();
    for level in LEVELS {
        assert_eq!(
            compile(
                &compiler(level),
                "fn add(a: number, b: number) -> number { return a + b; }"
            ),
            expected
        );
    }
}

#[test]
fn remainder_follows_users_and_seven_unchanged_memory_helpers_preserving_imports_and_calls() {
    let baseline = "
        fn rem(a: number, b: number) -> number { return a + b; }
        fn at(a: number[], i: number, b: number) -> number { return a[i] + b; }
        fn same(a: number[], b: number[]) -> bool { return a == b; }
        fn text(a: string, b: string) -> bool { return a == b; }
        fn items(a: number, b: number) -> number[] { return [a + b, rem(a, b)]; }
        fn host(a: number, b: number) -> number { return host_number(a + b); }
        fn size(a: number[]) -> number { return len(a); }
    ";
    let source = baseline
        .replace("a + b", "a % b")
        .replace("a[i] + b", "a[i] % b");
    for level in LEVELS {
        let mut compiler = compiler(level);
        compiler
            .register_host_function(
                "host_number",
                "env",
                "number",
                vec![Type::Number],
                Type::Number,
            )
            .unwrap();
        compiler
            .register_host_function("unused_host", "env", "unused", vec![], Type::Bool)
            .unwrap();
        let before = decode(&compile(&compiler, baseline));
        let after = decode(&compile(&compiler, &source));
        let scalar = decode(&compile(
            &self::compiler(level),
            "fn rem(a: number, b: number) -> number { return a % b; }",
        ));
        assert_eq!(before.functions.len(), 14);
        assert_eq!(after.functions.len(), 15);
        assert_eq!(after.imports, before.imports);
        assert_eq!(
            after
                .imports
                .iter()
                .map(|(module, name, _)| (module.as_str(), name.as_str()))
                .collect::<Vec<_>>(),
            [("env", "number"), ("env", "unused")]
        );
        assert_eq!(after.signatures, before.signatures);
        assert_eq!(&after.functions[..14], before.functions);
        assert_eq!(&after.bodies[7..14], &before.bodies[7..14]);
        assert_eq!(after.bodies[14], scalar.bodies[1]);
        assert_eq!(after.exports, before.exports);
        for (index, name) in ["rem", "at", "same", "text", "items", "host", "size"]
            .iter()
            .enumerate()
        {
            assert_eq!(after.exports[index], export(name, index as u32 + 2));
        }
        assert_eq!(after.exports.len(), 9);
        assert!(
            after
                .exports
                .contains(&("memory".into(), ExternalKind::Memory, 0))
        );
        assert!(after.exports.contains(&export("__alloc", 9)));
        assert_eq!(after.functions[14], after.functions[0]);
        assert_eq!(after.calls[0], [16]);
        assert!(after.calls[1].contains(&12));
        assert!(after.calls[1].contains(&16));
        assert!(after.calls[2].contains(&15));
        assert!(after.calls[3].contains(&14));
        assert!(after.calls[4].contains(&9));
        assert!(after.calls[4].contains(&2));
        assert!(after.calls[4].contains(&16));
        assert_eq!(after.calls[5], [16, 0]);
        assert!(after.calls[6].contains(&13));
        assert!(after.calls[14].is_empty());
        let unchanged = |module: &DecodedModule| {
            module
                .sections
                .iter()
                .filter(|(id, _)| ![1, 3, 10].contains(id))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(unchanged(&before), unchanged(&after));
    }
}

#[test]
fn export_selection_keeps_unexported_remainder_compiled_and_changes_only_exports() {
    for memory in [false, true] {
        let source = if memory {
            "fn visible(a: number[]) -> number { return len(a); } fn hidden(a: number, b: number) -> number { return a % b; }"
        } else {
            "fn visible(a: number) -> number { return a; } fn hidden(a: number, b: number) -> number { return a % b; }"
        };
        for level in LEVELS {
            let mut compiler = compiler(level);
            compiler
                .register_host_function("unused", "env", "unused", vec![], Type::Number)
                .unwrap();
            let program = compiler.analyze(source).unwrap();
            let original = program.clone();
            let before = decode(&compile(&compiler, source));
            let runtime = if memory {
                vec![
                    ("memory".into(), ExternalKind::Memory, 0),
                    export("__alloc", 3),
                ]
            } else {
                vec![]
            };
            let helper = if memory { 10 } else { 3 };
            assert_eq!(before.functions.len(), if memory { 10 } else { 3 });
            assert_eq!(before.calls[1], [helper]);
            for (selection, users) in [
                (
                    ExportOptions::All,
                    vec![export("visible", 1), export("hidden", 2)],
                ),
                (
                    ExportOptions::Only(vec!["visible".into()]),
                    vec![export("visible", 1)],
                ),
                (
                    ExportOptions::Only(vec!["hidden".into()]),
                    vec![export("hidden", 2)],
                ),
                (ExportOptions::Only(vec![]), vec![]),
                (ExportOptions::None, vec![]),
            ] {
                compiler.set_exports(selection.clone());
                let bytes = compile(&compiler, source);
                let after = decode(&bytes);
                let mut expected = users;
                expected.extend(runtime.clone());
                assert_eq!(after.exports, expected);
                assert_eq!(after.sections, before.sections);
                assert_eq!(after.functions, before.functions);
                assert_eq!(after.calls, before.calls);
                assert_eq!(after.imports, before.imports);
                assert_eq!(compiler.compile(&program).unwrap(), bytes);
                assert_eq!(compiler.compile_source(source).unwrap(), bytes);
                assert_eq!(compiler.exports(), &selection);
                assert_eq!(program, original);
            }
        }
    }
}

#[test]
fn helper_is_discovered_in_unreachable_and_short_circuited_code() {
    for source in [
        "fn test(a: number, b: number) -> number { return a; return a % b; }",
        "fn test(a: number, b: number) -> bool { return false && a % b == 0; }",
        "fn test(a: number, b: number) -> bool { return true || a % b == 0; }",
        "fn test(a: number, b: number) -> void { if false { a % b; } while false { a % b; } }",
    ] {
        for level in LEVELS {
            let module = decode(&compile(&compiler(level), source));
            assert_eq!(module.functions.len(), 2);
            assert_eq!(module.exports, [export("test", 0)]);
            assert_eq!(
                module.signatures[module.functions[1] as usize],
                numeric_signature()
            );
            assert!(module.imports.is_empty());
        }
    }
}

fn number(literal: &str) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Number(literal.into()),
        ty: Type::Number,
    }
}

fn value(ty: Type) -> TypedExpr {
    let kind = match ty {
        Type::Number => TypedExprKind::Number("1".into()),
        Type::Bool | Type::Void => TypedExprKind::Bool(false),
        Type::String => TypedExprKind::String("text".into()),
        Type::NumberArray => TypedExprKind::Array(vec![]),
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

fn remainder(left: TypedExpr, right: TypedExpr) -> TypedExpr {
    binary(left, BinaryOp::Remainder, right, Type::Number)
}

fn assert_forged_error(expr: TypedExpr, expected: CodegenError, span: Span) {
    for level in LEVELS {
        for context in 0..7 {
            let compiler = compiler(level);
            let mut program = compiler.analyze("fn test() -> void {}").unwrap();
            let expr = match context {
                2 | 3 => {
                    let condition = binary(
                        binary(number("1"), BinaryOp::Add, expr.clone(), Type::Number),
                        BinaryOp::Equal,
                        number("0"),
                        Type::Bool,
                    );
                    binary(
                        TypedExpr {
                            kind: TypedExprKind::Bool(context == 3),
                            ty: Type::Bool,
                        },
                        if context == 2 {
                            BinaryOp::And
                        } else {
                            BinaryOp::Or
                        },
                        condition,
                        Type::Bool,
                    )
                }
                6 => binary(number("1"), BinaryOp::Multiply, expr.clone(), Type::Number),
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
                        id: was_compiler::typed_ast::LoopId(0),
                        cond: Box::new(value(Type::Bool)),
                        body: vec![statement],
                    },
                    span: Span::new(10, 90),
                },
                _ => statement,
            });
            let snapshot = program.clone();
            assert_eq!(
                compiler.compile(&program).unwrap_err(),
                expected,
                "level {level:?}, context {context}"
            );
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn forged_remainder_result_and_each_operand_must_be_number_even_when_unreachable_or_skipped() {
    let span = Span::new(31, 67);
    for found in [Type::Bool, Type::String, Type::NumberArray, Type::Void] {
        for expr in [
            binary(number("1"), BinaryOp::Remainder, number("2"), found),
            remainder(value(found), number("2")),
            remainder(number("1"), value(found)),
        ] {
            assert_forged_error(
                expr,
                CodegenError::TypeMismatch {
                    function: "test".into(),
                    expected: Type::Number,
                    found,
                    span,
                },
                span,
            );
        }
    }
}

#[test]
fn forged_remainder_validates_nested_operand_kinds_and_reports_left_to_right_errors() {
    let span = Span::new(42, 78);
    for found in [Type::Bool, Type::String, Type::NumberArray] {
        let mut wrong_kind = value(found);
        wrong_kind.ty = Type::Number;
        for expr in [
            remainder(wrong_kind.clone(), number("2")),
            remainder(number("1"), wrong_kind.clone()),
            remainder(remainder(number("1"), wrong_kind.clone()), number("2")),
            remainder(number("1"), remainder(wrong_kind.clone(), number("2"))),
            remainder(wrong_kind, number("NaN")),
        ] {
            assert_forged_error(
                expr,
                CodegenError::TypeMismatch {
                    function: "test".into(),
                    expected: found,
                    found: Type::Number,
                    span,
                },
                span,
            );
        }
    }
    assert_forged_error(
        remainder(number("NaN"), number("Infinity")),
        CodegenError::InvalidNumberLiteral {
            function: "test".into(),
            literal: "NaN".into(),
            span,
        },
        span,
    );
}

#[test]
fn remainder_still_validates_lexical_numbers_and_overflow_in_both_operands() {
    let span = Span::new(53, 89);
    for literal in [
        "", "+1", "-1", "-0", "1e3", "1E3", "1e-3", " 1", "1 ", "\t1", "1\n", "NaN", "nan", "inf",
        "Infinity", "-inf", ".25", "1.", ".", "1.2.3", "1_000", "0x10", "１２", "1\0",
    ] {
        for expr in [
            remainder(number(literal), number("1")),
            remainder(number("1"), number(literal)),
        ] {
            assert_forged_error(
                expr,
                CodegenError::InvalidNumberLiteral {
                    function: "test".into(),
                    literal: literal.into(),
                    span,
                },
                span,
            );
        }
    }
    let literal = "9".repeat(400);
    for expr in [
        remainder(number(&literal), number("1")),
        remainder(number("1"), number(&literal)),
    ] {
        assert_forged_error(
            expr,
            CodegenError::NumberLiteralOutOfRange {
                function: "test".into(),
                literal: literal.clone(),
                span,
            },
            span,
        );
    }
}

#[test]
fn valid_underflowing_literals_and_unary_negative_zero_remain_valid_remainder_operands() {
    let literal = format!("0.{}1", "0".repeat(400));
    for level in LEVELS {
        let compiler = compiler(level);
        for expr in [
            remainder(number(&literal), number("1")),
            remainder(number("1"), number(&literal)),
            remainder(
                TypedExpr {
                    kind: TypedExprKind::Unary {
                        op: UnaryOp::Negate,
                        expr: Box::new(number("0")),
                    },
                    ty: Type::Number,
                },
                number("1"),
            ),
        ] {
            let mut program = compiler
                .analyze("fn test() -> number { return 0; }")
                .unwrap();
            program.functions[0].body[0].kind = TypedStatementKind::Return(Some(expr));
            Validator::new()
                .validate_all(&compiler.compile(&program).unwrap())
                .unwrap();
        }
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "was-compiler-remainder-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create remainder test directory");
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
function same(actual, expected, label = '') {
    if (Number.isNaN(expected)) assert.ok(Number.isNaN(actual), `${label}: expected NaN, got ${actual}`);
    else assert.ok(Object.is(actual, expected), `${label}: expected ${expected} (${Object.is(expected, -0) ? '-0' : '+0/nonzero'}), got ${actual} (${Object.is(actual, -0) ? '-0' : '+0/nonzero'})`);
}
const bitsView = new DataView(new ArrayBuffer(8));
function fromBits(bits) { bitsView.setBigUint64(0, bits, true); return bitsView.getFloat64(0, true); }
function bits(value) { bitsView.setFloat64(0, value, true); return bitsView.getBigUint64(0, true); }
function adjacent(value, up) {
    if (Number.isNaN(value)) return value;
    if (value === 0) return up ? Number.MIN_VALUE : -Number.MIN_VALUE;
    if (value === Infinity) return up ? value : Number.MAX_VALUE;
    if (value === -Infinity) return up ? -Number.MAX_VALUE : value;
    return fromBits(bits(value) + ((value > 0) === up ? 1n : -1n));
}
function view(e) { return new DataView(e.memory.buffer); }
function array(e, values) {
    const handle = e.__alloc(8 + values.length * 8) >>> 0;
    view(e).setUint32(handle, handle + 8, true);
    view(e).setUint32(handle + 4, values.length, true);
    values.forEach((value, index) => view(e).setFloat64(handle + 8 + index * 8, value, true));
    return handle;
}
function read(e, handle) {
    const data = view(e).getUint32(handle, true), length = view(e).getUint32(handle + 4, true);
    return Array.from({length}, (_, index) => view(e).getFloat64(data + index * 8, true));
}
"#;

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_remainder_exhaustive_special_values_signed_finite_fractions_and_extreme_ratios() {
    run_node_at_all_levels(
        "fn rem(a: number, b: number) -> number { return a % b; }",
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        assert.deepEqual(WebAssembly.Module.imports(module_), []);
        assert.deepEqual(Object.keys(e), ['rem']);
        const values = [NaN, fromBits(0x7ff0000000000001n), fromBits(0xfff8000000001234n), Infinity, -Infinity, 0, -0, 1, -1, 2, -2, 5.5, -5.5, 0.1, -0.1, Number.MIN_VALUE, -Number.MIN_VALUE, 3 * Number.MIN_VALUE, -3 * Number.MIN_VALUE, 2 ** -1022, -(2 ** -1022), Number.MAX_VALUE, -Number.MAX_VALUE];
        for (const a of values) for (const b of values) {
            let expected;
            if (Number.isNaN(a) || Number.isNaN(b) || !Number.isFinite(a) || b === 0) expected = NaN;
            else if (!Number.isFinite(b) || a === 0) expected = a;
            else expected = a % b;
            same(e.rem(a, b), expected, `special ${bits(a).toString(16)} % ${bits(b).toString(16)}`);
        }
        for (const [a, b, expected] of [
            [5, 2, 1], [5, -2, 1], [-5, 2, -1], [-5, -2, -1],
            [5.5, 2, 1.5], [5.5, -2, 1.5], [-5.5, 2, -1.5], [-5.5, -2, -1.5],
            [1.25, 0.5, 0.25], [-1.25, 0.5, -0.25], [1.25, -0.5, 0.25], [-1.25, -0.5, -0.25],
            [4, 2, 0], [4, -2, 0], [-4, 2, -0], [-4, -2, -0],
            [0, 2, 0], [-0, 2, -0], [0, -2, 0], [-0, -2, -0],
            [1, Infinity, 1], [-1, Infinity, -1], [1, -Infinity, 1], [-1, -Infinity, -1],
            [0, Infinity, 0], [-0, Infinity, -0], [0, -Infinity, 0], [-0, -Infinity, -0],
            [Number.MIN_VALUE, 2, Number.MIN_VALUE], [-Number.MIN_VALUE, 2, -Number.MIN_VALUE],
            [3 * Number.MIN_VALUE, 2 * Number.MIN_VALUE, Number.MIN_VALUE],
            [-3 * Number.MIN_VALUE, 2 * Number.MIN_VALUE, -Number.MIN_VALUE],
            [Number.MAX_VALUE, Number.MIN_VALUE, 0], [-Number.MAX_VALUE, Number.MIN_VALUE, -0],
            [Number.MAX_VALUE, -Number.MIN_VALUE, 0], [-Number.MAX_VALUE, -Number.MIN_VALUE, -0],
            [Number.MAX_VALUE, 3, 2], [-Number.MAX_VALUE, 3, -2],
            [Number.MAX_VALUE, Number.MAX_VALUE, 0], [-Number.MAX_VALUE, Number.MAX_VALUE, -0],
        ]) same(e.rem(a, b), expected, `finite ${a} % ${b}`);
        const adversarial = [[1, 0.1], [10, 0.1], [1e20, 3], [Number.MAX_VALUE, Number.MIN_VALUE], [Number.MAX_VALUE, 3], [Number.MAX_VALUE, 2 ** -1022], [1, 3 * Number.MIN_VALUE]];
        assert.ok(adversarial.some(([a, b]) => !Object.is(a - Math.trunc(a / b) * b, a % b)));
        for (const [a, b] of adversarial) for (const x of [a, -a]) for (const y of [b, -b]) same(e.rem(x, y), x % y, `truncation trap ${x} % ${y}`);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_remainder_seeded_raw_float64_differential_fuzz_and_boundary_stress() {
    run_node_at_all_levels(
        "fn rem(a: number, b: number) -> number { return a % b; }",
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        function check(a, b) { same(e.rem(a, b), a % b, `${bits(a).toString(16)} % ${bits(b).toString(16)}`); }
        const mask = (1n << 64n) - 1n;
        let state = 0x243f6a8885a308d3n;
        function randomBits() {
            state = (state + 0x9e3779b97f4a7c15n) & mask;
            let z = state;
            z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & mask;
            z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & mask;
            return (z ^ (z >> 31n)) & mask;
        }
        for (let i = 0; i < 5000; i++) check(fromBits(randomBits()), fromBits(randomBits()));
        const edges = [NaN, Infinity, -Infinity, 0, -0, Number.MIN_VALUE, -Number.MIN_VALUE, 2 * Number.MIN_VALUE, -2 * Number.MIN_VALUE, 3 * Number.MIN_VALUE, -3 * Number.MIN_VALUE, Number.MAX_VALUE, -Number.MAX_VALUE, adjacent(Number.MAX_VALUE, false), -adjacent(Number.MAX_VALUE, false), 0.1, -0.1, 0.3, -0.3, 1, -1, 3, -3, 1e20, -1e20];
        for (const exponent of [-1022, -1021, -53, -1, 0, 1, 52, 53, 1022, 1023]) {
            const power = 2 ** exponent;
            edges.push(adjacent(power, false), power, adjacent(power, true));
        }
        assert.ok(edges.length * edges.length <= 4096);
        for (const a of edges) for (const b of edges) check(a, b);
        for (let exponent = -1074; exponent <= 1023; exponent++) {
            const power = 2 ** exponent;
            const below = adjacent(power, false), above = adjacent(power, true);
            for (const a of [below, power, above]) {
                check(a, power); check(-a, power); check(a, -power); check(-a, -power);
                check(Number.MAX_VALUE, a); check(-Number.MAX_VALUE, a);
                check(a, Number.MIN_VALUE); check(-a, Number.MIN_VALUE);
            }
        }
        for (const divisor of [0.1, 0.2, 0.3, 3, Number.MIN_VALUE, 3 * Number.MIN_VALUE, 2 ** -1022, 2 ** 900, Number.MAX_VALUE / 8]) {
            for (const quotient of [1, 2, 3, 7, 31, 2 ** 26, 2 ** 52, 2 ** 53]) {
                const multiple = divisor * quotient;
                if (!Number.isFinite(multiple)) continue;
                for (const dividend of [adjacent(multiple, false), multiple, adjacent(multiple, true)]) {
                    check(dividend, divisor); check(-dividend, divisor);
                    check(dividend, -divisor); check(-dividend, -divisor);
                }
            }
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_remainder_precedence_mixed_arithmetic_unary_variables_and_control_flow() {
    let source = format!(
        "{SCALAR_SOURCE}
        fn precedence(a: number, b: number, c: number) -> number {{ return a + b % c * 2; }}
        fn left_associative(a: number, b: number, c: number) -> number {{ return a / b % c * 2; }}
        fn mixed(a: number, b: number, c: number) -> number {{ return a % b / c + a * b % c - 2; }}
        fn unary(a: number, b: number) -> number {{ return -a % -b; }}
        fn negate_result(a: number, b: number) -> number {{ return -(a % b); }}
    "
    );
    run_node_at_all_levels(
        &source,
        &[],
        r#"
        e = new WebAssembly.Instance(module_).exports;
        for (const [a, b, c] of [[17, 5, 3], [-17, 5, 3], [17, -5, 3], [5.5, 2, 0.5], [0, 2, 3], [-0, 2, 3], [Number.MIN_VALUE, 2, 3], [Number.MAX_VALUE, 3, 2], [Infinity, 2, 3], [NaN, 2, 3], [1, 0, 3]]) {
            same(e.precedence(a, b, c), a + b % c * 2);
            same(e.left_associative(a, b, c), a / b % c * 2);
            same(e.mixed(a, b, c), a % b / c + a * b % c - 2);
            same(e.arithmetic(a, b), -a % b * 2 + 9 / 3 - 1);
            same(e.unary(a, b), -a % -b);
            same(e.negate_result(a, b), -(a % b));
            same(e.nested(a, b), (a % b) % (b % 3));
            same(e.local(a, b), (a % b + a) % b);
            same(e.indirect(a, b), a % b);
            let result = 0;
            for (let i = 0; i < 4; i++) { if (i % 2 === 0) result = result + a % b; else result = result - a % b; }
            same(e.flow(a, b), result);
            same(e.unreachable(a, b), a);
            assert.equal(e.skipped(a, b), 0);
            assert.equal(e.discarded(a, b), undefined);
        }
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_remainder_evaluates_both_operands_once_left_to_right_and_skips_logical_rhs() {
    let source = "
        fn pass(x: number) -> number { return x; }
        fn ordered() -> number { return pass(mark(1)) % pass(mark(2)); }
        fn nested() -> number { return (pass(mark(1)) % pass(mark(2))) % (pass(mark(3)) % pass(mark(4))); }
        fn sink() -> number { return consume(pass(mark(1)) % pass(mark(2))); }
        fn and_skip() -> bool { return false && ordered() == 0; }
        fn or_skip() -> bool { return true || ordered() == 0; }
        fn and_run() -> bool { return true && ordered() == 0; }
        fn or_run() -> bool { return false || ordered() == 0; }
    ";
    run_node_at_all_levels(
        source,
        &[
            ("mark", vec![Type::Number], Type::Number),
            ("consume", vec![Type::Number], Type::Number),
        ],
        r#"
        const events = [];
        let values;
        e = new WebAssembly.Instance(module_, {env: {
            mark(n) { events.push(n); return values[n - 1]; },
            consume(n) { events.push('consume'); return n; },
        }}).exports;
        for (const pair of [[17, 5], [-0, 2], [0, -2], [NaN, 2], [2, NaN], [Infinity, 2], [-Infinity, 0], [1, 0], [1, -0], [1, Infinity], [-0, -Infinity], [Number.MAX_VALUE, Number.MIN_VALUE]]) {
            values = [...pair, 7, 3];
            events.length = 0; same(e.ordered(), pair[0] % pair[1]); assert.deepEqual(events, [1, 2]);
            events.length = 0; same(e.nested(), (pair[0] % pair[1]) % (7 % 3)); assert.deepEqual(events, [1, 2, 3, 4]);
            events.length = 0; same(e.sink(), pair[0] % pair[1]); assert.deepEqual(events, [1, 2, 'consume']);
            events.length = 0; assert.equal(e.and_skip(), 0); assert.deepEqual(events, []);
            events.length = 0; assert.equal(e.or_skip(), 1); assert.deepEqual(events, []);
            events.length = 0; assert.equal(e.and_run(), Number(pair[0] % pair[1] === 0)); assert.deepEqual(events, [1, 2]);
            events.length = 0; assert.equal(e.or_run(), Number(pair[0] % pair[1] === 0)); assert.deepEqual(events, [1, 2]);
        }
        values = [NaN, 2];
        events.length = 0;
        const failing = new WebAssembly.Instance(module_, {env: {
            mark(n) { events.push(n); if (n === 2) throw new Error('right operand executed'); return NaN; },
            consume(n) { return n; },
        }}).exports;
        assert.throws(() => failing.ordered(), /right operand executed/);
        assert.deepEqual(events, [1, 2]);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js on PATH"]
fn node_remainder_array_elements_index_assignments_calls_and_content_equality_without_alias_shortcuts()
 {
    let source = "
        fn rem(a: number, b: number) -> number { return a % b; }
        fn items(a: number, b: number) -> number[] { return [a % b, rem(-a, b), a % -b]; }
        fn edit(a: number[], b: number) -> number[] { a[0] = a[0] % b; return a; }
        fn at(a: number[], i: number, b: number) -> number { return a[i % len(a)] % b; }
        fn same(a: number, b: number) -> bool { return [a % b] == [rem(a, b)]; }
        fn different(a: number, b: number) -> bool { return [a % b] != [rem(a, b)]; }
        fn alias(a: number, b: number) -> bool { let values: number[] = [a % b]; return values == values; }
        fn alias_different(a: number, b: number) -> bool { let values: number[] = [a % b]; return values != values; }
        fn ordered() -> number[] { return [mark(1) % mark(2), mark(3) % mark(4)]; }
        fn host(a: number, b: number) -> number[] { return wrap([a % b, rem(a, b)]); }
    ";
    run_node_at_all_levels(
        source,
        &[
            ("mark", vec![Type::Number], Type::Number),
            ("wrap", vec![Type::NumberArray], Type::NumberArray),
        ],
        r#"
        const events = [];
        let values;
        e = new WebAssembly.Instance(module_, {env: {
            mark(n) { events.push(n); return values[n - 1]; },
            wrap(handle) { events.push('wrap'); return handle; },
        }}).exports;
        for (const [a, b] of [[5.5, 2], [-5.5, -2], [-0, 2], [4, -2], [-4, 2], [NaN, 2], [1, NaN], [Infinity, 2], [1, 0], [1, Infinity], [Number.MAX_VALUE, Number.MIN_VALUE], [Number.MIN_VALUE, 2]]) {
            const expected = a % b;
            read(e, e.items(a, b)).forEach((value, index) => same(value, [expected, -a % b, a % -b][index]));
            const handle = array(e, [a, 99]);
            assert.equal(e.edit(handle, b), handle);
            same(read(e, handle)[0], expected);
            same(read(e, handle)[1], 99);
            const input = array(e, [a, 99]);
            same(e.at(input, 2, b), expected);
            same(read(e, input)[0], a);
            const equals = Number(!Number.isNaN(expected));
            assert.equal(e.same(a, b), equals);
            assert.equal(e.different(a, b), 1 - equals);
            assert.equal(e.alias(a, b), equals);
            assert.equal(e.alias_different(a, b), 1 - equals);
            events.length = 0;
            read(e, e.host(a, b)).forEach(value => same(value, expected));
            assert.deepEqual(events, ['wrap']);
            values = [a, b, -a, -b]; events.length = 0;
            const ordered = read(e, e.ordered());
            same(ordered[0], expected); same(ordered[1], -a % -b);
            assert.deepEqual(events, [1, 2, 3, 4]);
        }
        "#,
    );
}
