use super::*;
use crate::{
    ast::Type,
    codegen::{self, memory, numeric},
    compiler::{Compiler, CompilerOptions},
    host::HostFunctionId,
    optimization::OptimizationLevel,
    tokens::Span,
    typed_ast::{
        FunctionId, FunctionRef, TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind,
        VariableId,
    },
};
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, FunctionSection,
    GlobalSection, GlobalType, ImportSection, MemorySection, MemoryType, Module, TypeSection,
    ValType,
};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];
const SOURCE: &str = "
    fn first(value: number) -> number {
        let local: number = 1;
        return value + local;
    }
    fn second() -> number { return first(2); }
    fn hidden() -> void {}
";

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("tick", "env", "tick", vec![Type::Number], Type::Number)
        .unwrap();
    compiler
}

fn expr(kind: TypedExprKind, ty: Type) -> TypedExpr {
    TypedExpr {
        kind,
        ty: ty.into(),
    }
}

fn invalid_number() -> TypedExpr {
    expr(TypedExprKind::Number("invalid".into()), Type::Number)
}

fn invalidate_return(program: &mut TypedProgram, index: usize) {
    let statement = program.functions[index].body.last_mut().unwrap();
    statement.kind = TypedStatementKind::Return(Some(invalid_number()));
}

fn checked_compile(
    program: &TypedProgram,
    exports: &ExportOptions,
) -> Result<Vec<u8>, CodegenError> {
    ValidatedProgram::new(program, exports).map(|validated| codegen::assemble(&validated))
}

fn legacy_compile(
    program: &TypedProgram,
    selection: &ExportOptions,
) -> Result<Vec<u8>, CodegenError> {
    let layout = match selection {
        ExportOptions::All => Layout::new(program),
        _ => Layout::with_exports(program, selection),
    }?;
    let mut types = TypeSection::new();
    for signature in &layout.signatures {
        types.ty().function(
            signature.params.iter().copied(),
            signature.result.iter().copied(),
        );
    }
    let mut imports = ImportSection::new();
    for (host, type_index) in program.host_functions.iter().zip(&layout.host_type_indices) {
        imports.import(
            &host.module,
            &host.field,
            EntityType::Function(type_index.raw()),
        );
    }
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    for (user, plan) in program.functions.iter().zip(&layout.users) {
        let body = function::emit(user, &layout)?;
        functions.function(plan.type_index.raw());
        if plan.exported {
            exports.export(&user.name, ExportKind::Func, plan.function_index.raw());
        }
        code.function(&body);
    }
    let mut memories = MemorySection::new();
    let mut globals = GlobalSection::new();
    let mut data = DataSection::new();
    if let Some(memory) = &layout.memory {
        for (helper, plan) in memory::Helper::ALL.into_iter().zip(&memory.helpers) {
            functions.function(plan.type_index.raw());
            code.function(&memory::emit(helper, memory.first_function.raw()));
        }
        memories.memory(MemoryType {
            minimum: u64::from(memory.plan.heap_start)
                .div_ceil(memory::PAGE_BYTES)
                .max(1),
            maximum: Some(memory::MAX_PAGES),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(memory.plan.heap_start as i32),
        );
        exports.export("memory", ExportKind::Memory, 0);
        exports.export(
            "__alloc",
            ExportKind::Func,
            memory
                .first_function
                .offset(memory::Helper::Alloc.index())
                .raw(),
        );
        data.active(
            0,
            &ConstExpr::i32_const(0),
            memory.plan.data.iter().copied(),
        );
    }
    if let Some(remainder) = &layout.remainder {
        functions.function(remainder.type_index.raw());
        code.function(&numeric::emit_remainder());
    }
    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&functions);
    if layout.memory.is_some() {
        module.section(&memories);
        module.section(&globals);
    }
    module.section(&exports);
    module.section(&code);
    if layout.memory.is_some() {
        module.section(&data);
    }
    Ok(module.finish())
}

fn assert_same_error(program: &TypedProgram, exports: &ExportOptions) -> CodegenError {
    let original = program.clone();
    let legacy = legacy_compile(program, exports).unwrap_err();
    let checked = checked_compile(program, exports).unwrap_err();
    let public = codegen::compile_with_exports(program, exports).unwrap_err();

    assert_eq!(checked, legacy);
    assert_eq!(public, legacy);
    assert_eq!(program, &original);

    legacy
}

#[test]
fn validated_view_borrows_the_original_tree_and_owned_host_snapshot() {
    let mut compiler = compiler();
    let program = compiler.analyze(SOURCE).unwrap();
    let original = program.clone();
    let validated = ValidatedProgram::new(&program, &ExportOptions::All).unwrap();
    compiler
        .register_host_function("later", "env", "later", vec![], Type::Void)
        .unwrap();

    assert!(std::ptr::eq(validated.program(), &program));
    assert!(std::ptr::eq(
        &validated.program().host_functions,
        &program.host_functions
    ));
    assert_eq!(validated.bodies().len(), 3);
    assert_eq!(validated.layout().users.len(), 3);
    assert_eq!(validated.layout().host_type_indices.len(), 1);
    assert_eq!(compiler.host_functions().functions().len(), 2);
    assert_eq!(program, original);
    assert_eq!(
        codegen::assemble(&validated),
        compiler.compile(&program).unwrap()
    );
}

#[test]
fn checked_and_legacy_paths_preserve_bytes_for_all_exports_and_levels() {
    let mut compiler = compiler();
    let program = compiler
        .analyze(
            "fn first(value: number) -> number {
            let items: number[] = [value, 2];
            while value > 0 { value = value - 1; }
            if value == 0 && \"hello\" == \"hello\" { items[0] = tick(value) % 2; }
            return items[0] + len(items);
            return 99;
        }
        fn second() -> number { return first(3); }
        fn hidden() -> void { let unused: number[] = []; }",
        )
        .unwrap();
    let original = program.clone();

    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["second".into()]),
    ] {
        let legacy = legacy_compile(&program, &exports).unwrap();
        let validated = ValidatedProgram::new(&program, &exports).unwrap();

        assert_eq!(codegen::assemble(&validated), legacy);
        assert_eq!(codegen::assemble(&validated), legacy);
        wasmparser::Validator::new().validate_all(&legacy).unwrap();

        for level in LEVELS {
            compiler.set_optimization_level(level);
            compiler.set_exports(exports.clone());
            assert_eq!(compiler.compile(&program).unwrap(), legacy);
        }
    }

    assert_eq!(program, original);
}

#[test]
fn checked_bodies_and_layout_are_an_immutable_export_snapshot() {
    let program = compiler().analyze(SOURCE).unwrap();
    let mut exports = ExportOptions::Only(vec!["second".into()]);
    let validated = ValidatedProgram::new(&program, &exports).unwrap();
    let expected = legacy_compile(&program, &exports).unwrap();
    exports = ExportOptions::None;

    assert_eq!(codegen::assemble(&validated), expected);
    assert_ne!(
        codegen::assemble(&validated),
        legacy_compile(&program, &exports).unwrap()
    );
    assert_eq!(validated.bodies().len(), 3);
}

#[test]
fn layout_errors_still_precede_body_validation() {
    let compiler = compiler();
    let original = compiler.analyze(SOURCE).unwrap();
    let mut program = original.clone();
    invalidate_return(&mut program, 0);
    program.host_functions[0].id = HostFunctionId(99);
    assert!(matches!(
        assert_same_error(&program, &ExportOptions::All),
        CodegenError::InvalidHostFunctionId { actual: 99, .. }
    ));

    let mut program = original.clone();
    invalidate_return(&mut program, 0);
    program.functions[1].id = FunctionId(99);
    assert!(matches!(
        assert_same_error(&program, &ExportOptions::All),
        CodegenError::InvalidFunctionId { actual: 99, .. }
    ));

    let mut program = original.clone();
    invalidate_return(&mut program, 0);
    program.host_functions[0].params = vec![Type::Void];
    assert!(matches!(
        assert_same_error(&program, &ExportOptions::All),
        CodegenError::VoidParameter { .. }
    ));

    let mut program = original.clone();
    invalidate_return(&mut program, 0);
    program.functions[1].name = "first".into();
    assert!(
        matches!(assert_same_error(&program, &ExportOptions::None), CodegenError::DuplicateExportName { name } if name == "first")
    );

    let mut program = original;
    invalidate_return(&mut program, 0);
    assert!(
        matches!(assert_same_error(&program, &ExportOptions::Only(vec!["missing".into()])), CodegenError::UnknownExport { name } if name == "missing")
    );
}

#[test]
fn first_function_body_errors_precede_later_function_local_errors() {
    let mut program = compiler().analyze(SOURCE).unwrap();
    invalidate_return(&mut program, 0);
    let span = program.functions[0].body.last().unwrap().span;
    program.functions[1].body.insert(
        0,
        TypedStatement {
            kind: TypedStatementKind::Let {
                id: VariableId(99),
                name: "bad".into(),
                ty: Type::Void,
                value: invalid_number(),
            },
            span: Span::new(500, 510),
        },
    );

    assert!(
        matches!(assert_same_error(&program, &ExportOptions::None), CodegenError::InvalidNumberLiteral { function, span: found, .. } if function == "first" && found == span)
    );
}

#[test]
fn missing_return_precedes_errors_in_later_functions() {
    let mut program = compiler().analyze(SOURCE).unwrap();
    program.functions[0].body.pop();
    invalidate_return(&mut program, 1);

    assert!(
        matches!(assert_same_error(&program, &ExportOptions::All), CodegenError::MissingReturnValue { function, return_type: Type::Number } if function == "first")
    );
}

#[test]
fn local_type_checks_still_precede_duplicate_bindings_and_body_literals() {
    let mut program = compiler().analyze(SOURCE).unwrap();
    invalidate_return(&mut program, 0);
    let span = program.functions[0].body[0].span;
    let TypedStatementKind::Let { id, ty, .. } = &mut program.functions[0].body[0].kind else {
        panic!("let");
    };
    *id = VariableId(0);
    *ty = Type::Void;

    assert!(
        matches!(assert_same_error(&program, &ExportOptions::All), CodegenError::UnsupportedLocalType { function, ty: Type::Void, span: found } if function == "first" && found == span)
    );
}

#[test]
fn checked_construction_preserves_scope_and_reference_errors() {
    let compiler = compiler();
    let mut program = compiler.analyze(
        "fn first(value: number) -> number { if true { let inner: number = 1; } return value; }",
    ).unwrap();
    let statement = program.functions[0].body.last_mut().unwrap();
    let span = statement.span;
    statement.kind = TypedStatementKind::Return(Some(expr(
        TypedExprKind::Identifier {
            id: VariableId(1),
            name: "inner".into(),
        },
        Type::Number,
    )));
    assert!(
        matches!(assert_same_error(&program, &ExportOptions::All), CodegenError::UninitializedVariable { id: VariableId(1), span: found, .. } if found == span)
    );

    let mut program = compiler.analyze(SOURCE).unwrap();
    let statement = program.functions[0].body.last_mut().unwrap();
    let span = statement.span;
    statement.kind = TypedStatementKind::Return(Some(expr(
        TypedExprKind::Call {
            func: FunctionRef::User(FunctionId(999)),
            name: "missing".into(),
            args: vec![invalid_number()],
        },
        Type::Number,
    )));
    assert!(
        matches!(assert_same_error(&program, &ExportOptions::All), CodegenError::UnknownFunctionReference { reference: FunctionRef::User(FunctionId(999)), span: found, .. } if found == span)
    );
}

#[test]
fn hidden_unreachable_and_short_circuited_errors_are_not_skipped() {
    let compiler = compiler();
    for source in [
        "fn first() -> void {} fn hidden() -> number { return 1; }",
        "fn first() -> number { return 1; return 2; }",
        "fn first() -> bool { return false && 1 % 2 == 0; }",
    ] {
        let mut program = compiler.analyze(source).unwrap();
        let function = program.functions.last_mut().unwrap();
        let statement = function.body.last_mut().unwrap();
        let span = statement.span;
        if let TypedStatementKind::Return(Some(value)) = &mut statement.kind {
            match &mut value.kind {
                TypedExprKind::Binary { right, .. } => {
                    let TypedExprKind::Binary { left, .. } = &mut right.kind else {
                        panic!("comparison");
                    };
                    let TypedExprKind::Binary { right, .. } = &mut left.kind else {
                        panic!("remainder");
                    };
                    **right = invalid_number();
                }
                _ => *value = invalid_number(),
            }
        } else {
            panic!("return");
        }

        assert!(
            matches!(assert_same_error(&program, &ExportOptions::None), CodegenError::InvalidNumberLiteral { span: found, .. } if found == span)
        );
    }
}

#[test]
fn subsequent_validation_observes_edits_after_the_checked_borrow_ends() {
    let mut program = compiler().analyze(SOURCE).unwrap();
    {
        let validated = ValidatedProgram::new(&program, &ExportOptions::All).unwrap();
        assert_eq!(
            codegen::assemble(&validated),
            legacy_compile(&program, &ExportOptions::All).unwrap()
        );
    }
    invalidate_return(&mut program, 0);

    assert!(matches!(
        assert_same_error(&program, &ExportOptions::All),
        CodegenError::InvalidNumberLiteral { .. }
    ));
}

#[test]
fn empty_program_and_import_only_memory_modules_are_checked_without_user_bodies() {
    for program in [
        TypedProgram {
            host_functions: vec![],
            functions: vec![],
        },
        {
            let mut compiler = Compiler::new();
            compiler
                .register_host_function("echo", "env", "echo", vec![Type::String], Type::String)
                .unwrap();
            TypedProgram {
                host_functions: compiler.host_functions().functions().to_vec(),
                functions: vec![],
            }
        },
    ] {
        for exports in [
            ExportOptions::All,
            ExportOptions::None,
            ExportOptions::Only(vec![]),
        ] {
            let validated = ValidatedProgram::new(&program, &exports).unwrap();
            let bytes = codegen::assemble(&validated);

            assert!(validated.bodies().is_empty());
            assert_eq!(bytes, legacy_compile(&program, &exports).unwrap());
            wasmparser::Validator::new().validate_all(&bytes).unwrap();
        }
    }
}

#[test]
fn checked_boundary_matches_the_existing_independent_abi_fixture() {
    let mut compiler = Compiler::with_options(CompilerOptions::default());
    for (name, ty) in [
        ("echo_array", Type::NumberArray),
        ("echo_string", Type::String),
        ("echo_bool", Type::Bool),
    ] {
        compiler
            .register_host_function(name, "env", name, vec![ty], ty)
            .unwrap();
    }
    let program = compiler
        .analyze(include_str!("../../tests/fixtures/abi/memory.was"))
        .unwrap();
    let expected = include_bytes!("../../tests/fixtures/abi/memory.wasm");

    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        ExportOptions::Only(vec!["values".into()]),
    ] {
        assert_eq!(
            checked_compile(&program, &exports),
            legacy_compile(&program, &exports)
        );
    }
    assert_eq!(
        checked_compile(&program, &ExportOptions::All).unwrap(),
        expected
    );
}
