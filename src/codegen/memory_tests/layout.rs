use super::*;

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
fn memory_abi_import_offsets_seven_unique_helpers_and_bytes_are_stable_at_all_levels() {
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
        assert_eq!(module.functions.len(), 13);
        assert_eq!(module.bodies.len(), 13);
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
            (vec![ValType::I32, ValType::I32], vec![ValType::I32]),
        ];
        assert_eq!(module.functions[6..].len(), expected.len());
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
        assert_eq!(module.calls[12], vec![11, 11]);
        for left in 6..13 {
            for right in left + 1..13 {
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
        assert_eq!(module.functions.len(), 8, "{source}");
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
        assert_eq!(module.functions.len(), 7);
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
