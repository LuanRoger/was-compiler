use std::{process::Command, time::Duration};

use crate::{
    CompileError, Compiler, CompilerOptions, ExportOptions, HostFunctionId, OptimizationLevel,
    Type, TypedProgram, typed_ast::FunctionId,
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef, Validator};

use super::{CodegenError, compile, compile_with_exports, layout::Layout};

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const SCALAR_SOURCE: &str = r#"
    fn main(value: number) -> number { return helper(value) + clock(); }
    fn unused(flag: bool) -> bool { return !flag; }
    fn helper(value: number) -> number { return value * 2; }
    fn tail() -> void {}
"#;

const MEMORY_SOURCE: &str = r#"
    fn main(value: number) -> number {
        let items: number[] = helper(value);
        return items[0] + len(items) + clock();
    }
    fn unused(text: string) -> bool { return text == "value"; }
    fn helper(value: number) -> number[] { return [value * 2, 3]; }
    fn tail() -> void {}
"#;

#[derive(Debug, Default, PartialEq, Eq)]
struct DecodedModule {
    sections: Vec<(u8, Vec<u8>)>,
    imports: Vec<(String, String, u32)>,
    functions: Vec<u32>,
    exports: Vec<(String, ExternalKind, u32)>,
    calls: Vec<Vec<u32>>,
}

fn decode(bytes: &[u8]) -> DecodedModule {
    Validator::new().validate_all(bytes).expect("valid Wasm");

    let mut module = DecodedModule::default();

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.expect("Wasm payload");

        if let Some((id, range)) = payload.as_section() {
            assert_ne!(id, 0, "codegen must not add backend metadata");

            if id != 7 {
                let start = usize::try_from(range.start).expect("section start");
                let end = usize::try_from(range.end).expect("section end");

                module.sections.push((id, bytes[start..end].to_vec()));
            }
        }

        match payload {
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
                    .map(|index| index.expect("function type"))
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
                let mut operators = body.get_operators_reader().expect("operators");
                let mut calls = Vec::new();

                while !operators.eof() {
                    if let Operator::Call { function_index } = operators.read().expect("operator") {
                        calls.push(function_index);
                    }
                }

                module.calls.push(calls);
            }
            _ => {}
        }
    }

    module
}

fn only(names: &[&str]) -> ExportOptions {
    ExportOptions::Only(names.iter().map(|name| (*name).to_owned()).collect())
}

fn compiler(level: OptimizationLevel) -> Compiler {
    let mut compiler = Compiler::with_options(CompilerOptions {
        optimization_level: level,
    });

    compiler
        .register_host_function("clock", "env", "now", vec![], Type::Number)
        .expect("register clock");
    compiler
        .register_host_function("sink", "env", "unused", vec![Type::Bool], Type::Void)
        .expect("register unused import");

    compiler
}

fn user_export(name: &str, index: u32) -> (String, ExternalKind, u32) {
    (name.to_owned(), ExternalKind::Func, index)
}

fn runtime_exports(first_function: u32) -> Vec<(String, ExternalKind, u32)> {
    vec![
        ("memory".to_owned(), ExternalKind::Memory, 0),
        user_export("__alloc", first_function),
    ]
}

#[test]
fn export_options_are_default_cloneable_and_equatable() {
    assert_eq!(ExportOptions::default(), ExportOptions::All);

    for exports in [
        ExportOptions::All,
        ExportOptions::None,
        only(&["tail", "main"]),
    ] {
        assert_eq!(exports.clone(), exports);
        assert!(!format!("{exports:?}").is_empty());
    }

    assert_ne!(ExportOptions::None, only(&[]));
    assert_ne!(only(&["main", "tail"]), only(&["tail", "main"]));
}

#[test]
fn compiler_defaults_to_all_without_changing_copyable_options_literals() {
    assert_eq!(Compiler::new().exports(), &ExportOptions::All);
    assert_eq!(Compiler::default().exports(), &ExportOptions::All);

    for level in LEVELS {
        let options = CompilerOptions {
            optimization_level: level,
        };
        let copied_options = options;
        let compiler = Compiler::with_options(options);

        assert_eq!(compiler.options(), &copied_options);
        assert_eq!(compiler.exports(), &ExportOptions::All);
    }
}

#[test]
fn legacy_compile_and_default_layout_keep_all_exports() {
    for source in [SCALAR_SOURCE, MEMORY_SOURCE] {
        for level in LEVELS {
            let compiler = compiler(level);
            let program = compiler.analyze(source).unwrap();
            let original = program.clone();
            let expected = compile(&program).unwrap();
            let default_layout = Layout::new(&program).unwrap();
            let explicit_layout = Layout::with_exports(&program, &ExportOptions::All).unwrap();

            assert_eq!(
                compile_with_exports(&program, &ExportOptions::All).unwrap(),
                expected
            );
            assert_eq!(compiler.compile(&program).unwrap(), expected);
            assert_eq!(compiler.compile_source(source).unwrap(), expected);
            assert!(default_layout.users.iter().all(|plan| plan.exported));
            assert_eq!(default_layout.signatures, explicit_layout.signatures);
            assert_eq!(
                default_layout.host_type_indices,
                explicit_layout.host_type_indices
            );
            assert_eq!(program, original);
        }
    }
}

fn assert_selection_only_changes_exports(source: &str, memory: bool) {
    for level in LEVELS {
        let mut compiler = compiler(level);
        let program = compiler.analyze(source).unwrap();
        let original = program.clone();
        let baseline = decode(&compile(&program).unwrap());
        let hosts = compiler.host_functions().clone();
        let options = *compiler.options();
        let runtime = if memory { runtime_exports(6) } else { vec![] };
        let all_users = vec![
            user_export("main", 2),
            user_export("unused", 3),
            user_export("helper", 4),
            user_export("tail", 5),
        ];
        let mut all_exports = all_users.clone();

        all_exports.extend(runtime.clone());

        assert_eq!(baseline.exports, all_exports);
        assert_eq!(baseline.imports.len(), 2);
        assert_eq!(baseline.functions.len(), if memory { 11 } else { 4 });
        assert_eq!(baseline.calls.len(), baseline.functions.len());
        assert!(baseline.calls[0].contains(&4));
        assert!(baseline.calls[0].contains(&0));

        for (selection, users) in [
            (ExportOptions::All, all_users),
            (
                only(&["tail", "main"]),
                vec![user_export("main", 2), user_export("tail", 5)],
            ),
            (
                only(&["main", "tail"]),
                vec![user_export("main", 2), user_export("tail", 5)],
            ),
            (only(&["helper"]), vec![user_export("helper", 4)]),
            (ExportOptions::None, vec![]),
            (only(&[]), vec![]),
        ] {
            let selection_snapshot = selection.clone();
            let direct = compile_with_exports(&program, &selection).unwrap();

            compiler.set_exports(selection);

            let bytes = compiler.compile(&program).unwrap();
            let mut expected_exports = users;

            expected_exports.extend(runtime.clone());

            let module = decode(&bytes);

            assert_eq!(module.exports, expected_exports, "level {level:?}");
            assert_eq!(module.sections, baseline.sections);
            assert_eq!(module.imports, baseline.imports);
            assert_eq!(module.functions, baseline.functions);
            assert_eq!(module.calls, baseline.calls);
            assert_eq!(bytes, direct);
            assert_eq!(compiler.compile(&program).unwrap(), bytes);
            assert_eq!(compiler.compile_source(source).unwrap(), bytes);
            assert_eq!(compiler.exports(), &selection_snapshot);
            assert_eq!(compiler.options(), &options);
            assert_eq!(compiler.host_functions(), &hosts);
            assert_eq!(compiler.analyze(source).unwrap(), original);
            assert_eq!(program, original);
        }

        assert_eq!(
            compile_with_exports(&program, &only(&["tail", "main"])).unwrap(),
            compile_with_exports(&program, &only(&["main", "tail"])).unwrap(),
        );
    }
}

#[test]
fn scalar_selection_preserves_all_imports_signatures_bodies_and_call_indices() {
    assert_selection_only_changes_exports(SCALAR_SOURCE, false);
}

#[test]
fn memory_selection_preserves_runtime_exports_helpers_data_and_call_indices() {
    assert_selection_only_changes_exports(MEMORY_SOURCE, true);
}

#[test]
fn empty_program_accepts_empty_selection_and_rejects_requested_names() {
    let program = TypedProgram {
        functions: vec![],
        host_functions: vec![],
    };
    let baseline = compile(&program).unwrap();

    for selection in [ExportOptions::All, ExportOptions::None, only(&[])] {
        assert_eq!(
            compile_with_exports(&program, &selection).unwrap(),
            baseline
        );
        assert!(decode(&baseline).exports.is_empty());
    }

    assert_eq!(
        compile_with_exports(&program, &only(&["main"])),
        Err(CodegenError::UnknownExport {
            name: "main".into()
        }),
    );
}

#[test]
fn import_only_memory_modules_keep_runtime_exports_with_no_user_exports() {
    for level in LEVELS {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });

        compiler
            .register_host_function("text", "env", "text", vec![Type::String], Type::String)
            .unwrap();

        let program = TypedProgram {
            functions: vec![],
            host_functions: compiler.host_functions().functions().to_vec(),
        };
        let original = program.clone();
        let expected = compiler.compile(&program).unwrap();
        let module = decode(&expected);

        assert_eq!(module.imports.len(), 1);
        assert_eq!(module.functions.len(), 7);
        assert_eq!(module.exports, runtime_exports(1));

        for selection in [ExportOptions::None, only(&[])] {
            compiler.set_exports(selection);

            assert_eq!(compiler.compile(&program).unwrap(), expected);
            assert_eq!(
                compile_with_exports(&program, compiler.exports()).unwrap(),
                expected,
            );
            assert_eq!(program, original);
        }
    }
}

#[test]
fn unexported_unused_reference_functions_still_require_memory() {
    let source = "fn main() -> number { return 1; } fn hidden() -> string { return \"value\"; }";

    for level in LEVELS {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });
        let program = compiler.analyze(source).unwrap();
        let baseline = decode(&compile(&program).unwrap());

        for selection in [only(&["main"]), ExportOptions::None, only(&[])] {
            compiler.set_exports(selection.clone());

            let module = decode(&compiler.compile(&program).unwrap());
            let mut expected = if selection == only(&["main"]) {
                vec![user_export("main", 0)]
            } else {
                vec![]
            };

            expected.extend(runtime_exports(2));

            assert_eq!(module.exports, expected);
            assert_eq!(module.sections, baseline.sections);
            assert_eq!(module.functions.len(), 9);
        }
    }
}

#[test]
fn memory_reserved_names_collide_only_when_the_user_function_is_exported() {
    for name in ["memory", "__alloc"] {
        for source in [
            format!("fn main() -> void {{}} fn {name}(a: number[]) -> void {{}}"),
            format!(
                "fn main() -> number {{ return {name}(); }} \
                 fn {name}() -> number {{ return 7; }} \
                 fn payload() -> string {{ return \"value\"; }}"
            ),
        ] {
            for level in LEVELS {
                let mut compiler = Compiler::with_options(CompilerOptions {
                    optimization_level: level,
                });
                let program = compiler.analyze(&source).unwrap();
                let original = program.clone();
                let runtime_index = program.functions.len() as u32;
                let expected_error = CodegenError::DuplicateExportName { name: name.into() };

                for selection in [ExportOptions::All, only(&[name]), only(&["main", name])] {
                    assert_eq!(
                        compile_with_exports(&program, &selection),
                        Err(CodegenError::DuplicateExportName { name: name.into() }),
                    );

                    compiler.set_exports(selection);

                    assert_eq!(
                        compiler.compile_source(&source),
                        Err(CompileError::Codegen(CodegenError::DuplicateExportName {
                            name: name.into(),
                        })),
                    );
                }

                assert_eq!(compile(&program), Err(expected_error));

                for selection in [only(&["main"]), ExportOptions::None, only(&[])] {
                    let selected_main = selection == only(&["main"]);

                    compiler.set_exports(selection);

                    let module = decode(&compiler.compile(&program).unwrap());
                    let mut expected = if selected_main {
                        vec![user_export("main", 0)]
                    } else {
                        vec![]
                    };

                    expected.extend(runtime_exports(runtime_index));

                    assert_eq!(module.exports, expected);
                    assert_eq!(module.functions.len(), program.functions.len() + 7);
                    assert_eq!(program, original);
                }
            }
        }
    }
}

#[test]
fn scalar_reserved_names_remain_valid_selected_user_exports() {
    let source = "fn memory() -> number { return 1; } fn __alloc() -> bool { return true; }";

    for level in LEVELS {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: level,
        });
        let program = compiler.analyze(source).unwrap();
        let baseline = decode(&compile(&program).unwrap());

        for (selection, expected) in [
            (
                ExportOptions::All,
                vec![user_export("memory", 0), user_export("__alloc", 1)],
            ),
            (
                only(&["__alloc", "memory"]),
                vec![user_export("memory", 0), user_export("__alloc", 1)],
            ),
            (only(&["__alloc"]), vec![user_export("__alloc", 1)]),
            (ExportOptions::None, vec![]),
        ] {
            compiler.set_exports(selection);

            let module = decode(&compiler.compile(&program).unwrap());

            assert_eq!(module.exports, expected);
            assert_eq!(module.sections, baseline.sections);
            assert!(!module.sections.iter().any(|(id, _)| *id == 5));
        }
    }
}

#[test]
fn unknown_selection_rejects_non_user_names_without_mutation_at_every_level() {
    for source in [SCALAR_SOURCE, MEMORY_SOURCE] {
        for level in LEVELS {
            let mut compiler = compiler(level);
            let program = compiler.analyze(source).unwrap();
            let original = program.clone();
            let hosts = compiler.host_functions().clone();
            let options = *compiler.options();

            for name in [
                "missing", "Main", "", " main", "clock", "now", "sink", "memory", "__alloc",
            ] {
                let selection = only(&["main", name]);
                let snapshot = selection.clone();

                assert_eq!(
                    compile_with_exports(&program, &selection),
                    Err(CodegenError::UnknownExport { name: name.into() }),
                );

                compiler.set_exports(selection);

                assert_eq!(
                    compiler.compile(&program),
                    Err(CodegenError::UnknownExport { name: name.into() }),
                );
                assert_eq!(
                    compiler.compile_source(source),
                    Err(CompileError::Codegen(CodegenError::UnknownExport {
                        name: name.into()
                    })),
                );
                assert_eq!(compiler.exports(), &snapshot);
                assert_eq!(compiler.options(), &options);
                assert_eq!(compiler.host_functions(), &hosts);
                assert_eq!(program, original);
            }

            compiler.set_exports(only(&["main"]));

            assert!(compiler.compile(&program).is_ok());
        }
    }
}

#[test]
fn duplicate_selection_has_a_distinct_explicit_error_at_every_level() {
    for level in LEVELS {
        let mut compiler = compiler(level);
        let program = compiler.analyze(MEMORY_SOURCE).unwrap();
        let original = program.clone();

        for (selection, name) in [
            (only(&["main", "main"]), "main"),
            (only(&["tail", "main", "tail"]), "tail"),
        ] {
            let snapshot = selection.clone();

            assert_eq!(
                compile_with_exports(&program, &selection),
                Err(CodegenError::DuplicateExportSelection { name: name.into() }),
            );

            compiler.set_exports(selection);

            assert_eq!(
                compiler.compile_source(MEMORY_SOURCE),
                Err(CompileError::Codegen(
                    CodegenError::DuplicateExportSelection { name: name.into() }
                )),
            );
            assert_eq!(compiler.exports(), &snapshot);
            assert_eq!(program, original);
        }
    }

    assert_eq!(
        CodegenError::UnknownExport {
            name: "missing".into()
        }
        .to_string(),
        "unknown user function export `missing`",
    );
    assert_eq!(
        CodegenError::DuplicateExportSelection {
            name: "main".into()
        }
        .to_string(),
        "duplicate requested export `main`",
    );
}

#[test]
fn duplicate_source_names_in_forged_programs_are_rejected_even_when_unexported() {
    let mut program = Compiler::new()
        .analyze("fn main() -> void {} fn helper() -> void {}")
        .unwrap();

    program.functions[1].name = "main".into();

    let original = program.clone();

    for selection in [
        ExportOptions::All,
        ExportOptions::None,
        only(&[]),
        only(&["main"]),
    ] {
        assert_eq!(
            compile_with_exports(&program, &selection),
            Err(CodegenError::DuplicateExportName {
                name: "main".into()
            }),
        );
        assert_eq!(program, original);
    }
}

#[test]
fn selection_never_skips_user_or_host_layout_validation() {
    let compiler = compiler(OptimizationLevel::None);
    let original = compiler.analyze(SCALAR_SOURCE).unwrap();
    let mut invalid_user = original.clone();
    let mut invalid_host = original.clone();
    let mut invalid_signature = original.clone();

    invalid_user.functions[2].id = FunctionId(99);
    invalid_host.host_functions[1].id = HostFunctionId(99);
    invalid_signature.functions[2].params[0].ty = Type::Void;

    for (program, error) in [
        (
            invalid_user,
            CodegenError::InvalidFunctionId {
                function: "helper".into(),
                expected: 2,
                actual: 99,
            },
        ),
        (
            invalid_host,
            CodegenError::InvalidHostFunctionId {
                function: "sink".into(),
                expected: 1,
                actual: 99,
            },
        ),
        (
            invalid_signature,
            CodegenError::VoidParameter {
                function: "helper".into(),
                index: 0,
            },
        ),
    ] {
        let snapshot = program.clone();

        for selection in [
            ExportOptions::All,
            ExportOptions::None,
            only(&[]),
            only(&["main"]),
        ] {
            assert_eq!(
                compile_with_exports(&program, &selection).unwrap_err(),
                error,
            );
            assert_eq!(program, snapshot);
        }
    }
}

#[test]
fn unexported_bodies_still_report_errors_at_every_optimization_level() {
    let literal = "9".repeat(400);
    let source = format!("fn main() -> void {{}} fn hidden() -> number {{ return {literal}; }}");

    for level in LEVELS {
        let mut compiler = compiler(level);
        let program = compiler.analyze(&source).unwrap();
        let original = program.clone();
        let expected = CodegenError::NumberLiteralOutOfRange {
            function: "hidden".into(),
            literal: literal.clone(),
            span: program.functions[1].body[0].span,
        };

        assert_eq!(compile(&program).unwrap_err(), expected);

        for selection in [
            ExportOptions::All,
            ExportOptions::None,
            only(&[]),
            only(&["main"]),
        ] {
            compiler.set_exports(selection);

            assert_eq!(compiler.compile(&program).unwrap_err(), expected);
            assert_eq!(
                compiler.compile_source(&source).unwrap_err(),
                CompileError::Codegen(compile(&program).unwrap_err())
            );
            assert_eq!(program, original);
        }
    }
}

#[test]
fn export_and_level_changes_preserve_options_registration_and_typed_import_snapshots() {
    let mut compiler = compiler(OptimizationLevel::Standard);
    let options_snapshot = *compiler.options();
    let hosts_snapshot = compiler.host_functions().clone();
    let program = compiler.analyze(SCALAR_SOURCE).unwrap();
    let original = program.clone();

    compiler.set_exports(only(&["main"]));

    let expected = compiler.compile(&program).unwrap();

    compiler
        .register_host_function("later", "extra", "later", vec![Type::String], Type::Void)
        .unwrap();

    let registered_hosts = compiler.host_functions().clone();

    for level in LEVELS {
        compiler.set_optimization_level(level);

        for selection in [ExportOptions::None, ExportOptions::All, only(&["main"])] {
            compiler.set_exports(selection.clone());

            assert_eq!(compiler.options().optimization_level, level);
            assert_eq!(compiler.host_functions(), &registered_hosts);
            assert_eq!(compiler.exports(), &selection);
            assert_eq!(
                compiler.analyze(SCALAR_SOURCE).unwrap().host_functions,
                registered_hosts.functions()
            );
            assert_eq!(program, original);
        }

        assert_eq!(compiler.compile(&program).unwrap(), expected);
        assert_ne!(compiler.compile_source(SCALAR_SOURCE).unwrap(), expected);
    }

    assert_eq!(
        options_snapshot.optimization_level,
        OptimizationLevel::Standard
    );
    assert_eq!(hosts_snapshot.functions().len(), 2);
    assert_eq!(program.host_functions, hosts_snapshot.functions());
    assert_eq!(compiler.host_functions().functions().len(), 3);
}

fn run_node(bytes: &[u8], script: &str) {
    let script = format!(
        "const assert = require('node:assert/strict');\n\
         const bytes = require('node:fs').readFileSync(0);\n\
         const module_ = new WebAssembly.Module(bytes);\n{script}"
    );
    let output = crate::test_process::output(
        Command::new("node").args(["-e", &script]),
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

fn run_at_all_levels(source: &str, selection: ExportOptions, script: &str) {
    let mut snapshot = None;

    for level in LEVELS {
        let mut compiler = compiler(level);

        compiler.set_exports(selection.clone());

        let program = compiler.analyze(source).unwrap();
        let original = program.clone();
        let bytes = compiler.compile(&program).unwrap();

        decode(&bytes);

        if let Some(expected) = &snapshot {
            assert_eq!(&bytes, expected, "level {level:?}");
        } else {
            snapshot = Some(bytes.clone());
        }

        run_node(&bytes, script);

        assert_eq!(program, original);
        assert_eq!(compiler.exports(), &selection);
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_selected_scalar_exports_call_forward_unexported_helpers_and_imports() {
    run_at_all_levels(
        SCALAR_SOURCE,
        only(&["tail", "main"]),
        r#"
        assert.deepEqual(WebAssembly.Module.exports(module_), [
            {name: 'main', kind: 'function'},
            {name: 'tail', kind: 'function'},
        ]);
        assert.deepEqual(WebAssembly.Module.imports(module_), [
            {module: 'env', name: 'now', kind: 'function'},
            {module: 'env', name: 'unused', kind: 'function'},
        ]);
        let calls = 0;
        const e = new WebAssembly.Instance(module_, {env: {
            now: () => { calls += 1; return 5; },
            unused: () => assert.fail('unused import called'),
        }}).exports;
        assert.deepEqual(Object.keys(e), ['main', 'tail']);
        assert.equal(e.main(4), 13);
        assert.equal(e.main(-3), -1);
        assert.equal(e.tail(), undefined);
        assert.equal(e.helper, undefined);
        assert.equal(e.unused, undefined);
        assert.equal(calls, 2);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_selected_memory_exports_call_forward_helpers_with_unchanged_runtime_indices() {
    run_at_all_levels(
        MEMORY_SOURCE,
        only(&["tail", "main"]),
        r#"
        assert.deepEqual(WebAssembly.Module.exports(module_), [
            {name: 'main', kind: 'function'},
            {name: 'tail', kind: 'function'},
            {name: 'memory', kind: 'memory'},
            {name: '__alloc', kind: 'function'},
        ]);
        const e = new WebAssembly.Instance(module_, {env: {
            now: () => 5,
            unused: () => assert.fail('unused import called'),
        }}).exports;
        assert.deepEqual(Object.keys(e), ['main', 'tail', 'memory', '__alloc']);
        assert.equal(e.main(4), 15);
        assert.equal(e.main(-3), 1);
        assert.equal(e.helper, undefined);
        assert.equal(e.unused, undefined);
        const pointer = e.__alloc(24) >>> 0;
        assert.equal(pointer % 8, 0);
        new DataView(e.memory.buffer).setFloat64(pointer, 123, true);
        assert.equal(new DataView(e.memory.buffer).getFloat64(pointer, true), 123);
        assert.equal(e.main(2), 11);
        "#,
    );
}

#[test]
#[ignore = "requires Node.js"]
fn node_none_and_empty_only_export_runtime_memory_and_working_allocator() {
    for selection in [ExportOptions::None, only(&[])] {
        run_at_all_levels(
            MEMORY_SOURCE,
            selection,
            r#"
            assert.deepEqual(WebAssembly.Module.exports(module_), [
                {name: 'memory', kind: 'memory'},
                {name: '__alloc', kind: 'function'},
            ]);
            assert.equal(WebAssembly.Module.imports(module_).length, 2);
            const e = new WebAssembly.Instance(module_, {env: {
                now: () => assert.fail('no user function should run'),
                unused: () => assert.fail('unused import called'),
            }}).exports;
            assert.deepEqual(Object.keys(e), ['memory', '__alloc']);
            assert.equal(e.main, undefined);
            assert.equal(e.helper, undefined);
            const first = e.__alloc(24) >>> 0;
            assert.equal(first % 8, 0);
            new DataView(e.memory.buffer).setFloat64(first, 42, true);
            const before = e.memory.buffer.byteLength;
            const second = e.__alloc(before + 16) >>> 0;
            assert(second >= first + 24);
            assert(e.memory.buffer.byteLength > before);
            assert.equal(new DataView(e.memory.buffer).getFloat64(first, true), 42);
            "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_none_and_empty_only_scalar_modules_keep_imports_but_no_exports() {
    for selection in [ExportOptions::None, only(&[])] {
        run_at_all_levels(
            SCALAR_SOURCE,
            selection,
            r#"
            assert.deepEqual(WebAssembly.Module.exports(module_), []);
            assert.equal(WebAssembly.Module.imports(module_).length, 2);
            const e = new WebAssembly.Instance(module_, {env: {
                now: () => assert.fail('no user function should run'),
                unused: () => assert.fail('unused import called'),
            }}).exports;
            assert.deepEqual(Object.keys(e), []);
            "#,
        );
    }
}

#[test]
#[ignore = "requires Node.js"]
fn node_unexported_reserved_source_names_remain_callable_in_memory_modules() {
    run_at_all_levels(
        r#"
        fn main() -> number { return memory() + __alloc() + clock(); }
        fn memory() -> number { return 10; }
        fn __alloc() -> number { return 20; }
        fn hidden() -> string { return "kept"; }
        "#,
        only(&["main"]),
        r#"
        assert.deepEqual(WebAssembly.Module.exports(module_), [
            {name: 'main', kind: 'function'},
            {name: 'memory', kind: 'memory'},
            {name: '__alloc', kind: 'function'},
        ]);
        const e = new WebAssembly.Instance(module_, {env: {
            now: () => 5,
            unused: () => assert.fail('unused import called'),
        }}).exports;
        assert.equal(e.main(), 35);
        assert(e.memory instanceof WebAssembly.Memory);
        const pointer = e.__alloc(8) >>> 0;
        assert(pointer >= 8);
        assert.equal(pointer % 8, 0);
        assert.equal(e.main(), 35);
        assert.equal(e.hidden, undefined);
        "#,
    );
}
