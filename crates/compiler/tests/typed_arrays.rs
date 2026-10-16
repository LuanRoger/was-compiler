use was_compiler::{ArrayElementType, Compiler, Type};

#[test]
fn boolean_arrays_compose_with_context_calls_iteration_and_captures() {
    let source = r#"
        fn choose(flags: bool[], other: bool[] = flags) -> bool[] { return other; }
        fn main() -> bool[] {
            let flags: bool[] = if true { [] } else { [true, false] };
            flags = choose(other: [], flags: [true]);
            flags[0] = false;
            for flag in flags { let read = () => flag; if read() { break; } }
            return loop { break match true { true => flags, false => [] }; };
        }
    "#;
    let compiler = Compiler::new();
    let program = compiler.analyze(source).unwrap();
    assert_eq!(
        program.functions[1].return_type,
        Type::Array(ArrayElementType::Bool)
    );
    let bytes = compiler.compile(&program).unwrap();
    wasmparser::Validator::new().validate_all(&bytes).unwrap();
}

#[test]
fn boolean_context_is_destination_directed_not_sibling_or_pipeline_inference() {
    let c = Compiler::new();
    for source in [
        "fn f(a: bool[]) -> bool[] { return a; } fn main() -> bool[] { return f([]); }",
        "fn main() -> bool[] { return { if true { return []; } [] }; }",
        "fn main() -> bool[] { return loop { if true { break []; } break [true]; }; }",
        "fn main() { let a: bool[] = [loop {}, true]; for x in a { let f = () => x; f(); } }",
        "fn main() { for x in [loop {}, true] { let y: bool = x; } }",
        "fn main() { let a = [true, false]; let f = (x: bool[]) => x; f([]); }",
    ] {
        let p = c
            .analyze(source)
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        let bytes = c.compile(&p).unwrap_or_else(|e| panic!("{source}: {e}"));
        wasmparser::Validator::new().validate_all(&bytes).unwrap();
    }
    for source in [
        "fn main() { let a = [true, 1]; }",
        "fn main() { let a = if true { [] } else { [true] }; }",
        "fn main() { if true { return []; } return [true]; }",
        "fn main() { let f = () => { if true { return []; } [true] }; }",
        "fn take(a: bool[]) -> bool[] { return a; } fn main() { return [] |> take; }",
        "fn empty() { return []; } fn main() -> bool[] { return empty(); }",
        "fn main() -> bool[] { let empty = () => []; return empty(); }",
        "fn main() { let a = [true]; a[loop {}] += 1; }",
        "fn main() { for x in [true] { let f = () => { x = false; }; } }",
        "fn main(a: bool[] = []) {}",
        "fn main(a: bool[]) requires a[0] {}",
    ] {
        assert!(c.analyze(source).is_err(), "{source}");
    }
}

#[test]
fn independent_backend_checks_element_and_iteration_metadata() {
    use was_compiler::typed_ast::{TypedExprKind, TypedStatementKind};
    let c = Compiler::new();
    let original = c
        .analyze("fn main() { let a = [true]; for x in a { let f = () => x; f(); } }")
        .unwrap();
    let mut forged = original.clone();
    let TypedStatementKind::Let { value, .. } = &mut forged.functions[0].body[0].kind else {
        panic!()
    };
    let TypedExprKind::Array(_, element) = &mut value.kind else {
        panic!()
    };
    *element = ArrayElementType::Number;
    assert!(c.compile(&forged).is_err());
    let mut forged = original.clone();
    let TypedStatementKind::For { element_type, .. } = &mut forged.functions[0].body[1].kind else {
        panic!()
    };
    *element_type = Type::Number;
    assert!(c.compile(&forged).is_err());
    let mut forged = original;
    let TypedStatementKind::Let { value, .. } = &mut forged.functions[0].body[0].kind else {
        panic!()
    };
    let TypedExprKind::Array(elements, _) = &mut value.kind else {
        panic!()
    };
    elements[0].ty = Type::Number.into();
    assert!(c.compile(&forged).is_err());
}

#[test]
fn string_arrays_context_defaults_contracts_and_nested_captures() {
    let source = r#"
        fn copy(a: string[], b: string[] = a) -> string[] requires a == b { return b; }
        fn main() -> string[] {
            let a: string[] = [];
            a = copy(["one", "two"]);
            let alias = a;
            let outer = () => { let inner = () => { a[0] = "changed"; a }; inner() };
            outer();
            for text in alias { let read = () => text; if read() == "changed" { continue; } }
            return match true { true => a, false => loop { break []; } };
        }
    "#;
    let c = Compiler::new();
    let p = c.analyze(source).unwrap();
    assert_eq!(
        p.functions[1].return_type,
        Type::Array(ArrayElementType::String)
    );
    wasmparser::Validator::new()
        .validate_all(&c.compile(&p).unwrap())
        .unwrap();
}

#[test]
fn both_element_types_preserve_exact_joins_whitelist_and_numeric_only_compounds() {
    let c = Compiler::new();
    for (ty, value, scalar) in [
        ("bool", "true", Type::Bool),
        ("string", "\"text\"", Type::String),
    ] {
        let source = format!(
            "fn main(a: {ty}[], copy: {ty}[] = a, n: number = len(copy)) -> {ty}[] requires a == copy {{ let empty: {ty}[] = []; let result = () => {{ return a; }}; for x in a {{ let read = () => x; read(); }} return if n == 0 {{ empty }} else {{ result() }}; }}"
        );
        let p = c.analyze(&source).unwrap();
        wasmparser::Validator::new()
            .validate_all(&c.compile(&p).unwrap())
            .unwrap();
        for op in ["+=", "-=", "*=", "/=", "%="] {
            for index in ["0", "loop {}"] {
                let source = format!("fn main() {{ let a = [{value}]; a[{index}] {op} 1; }}");
                assert!(c.analyze(&source).is_err(), "{source}");
            }
        }
        for source in [
            format!("fn main(a: {ty}[]) {{ let b = [[{value}]]; }}"),
            format!("fn main(a: {ty}[]) {{ let b = a == []; }}"),
            format!(
                "fn main(a: {ty}[]) {{ for x in a {{ let f = () => {{ return; x = {value}; }}; }} }}"
            ),
            format!("fn main(a: {ty}[] = [{value}]) {{}}"),
            format!("fn main(a: {ty}[]) requires true || a[0] == {value} {{}}"),
        ] {
            assert!(c.analyze(&source).is_err(), "{source}");
        }
        // Skipped/unreachable indexed annotations are independently checked.
        let mut p = c
            .analyze(&format!("fn main(a: {ty}[]) {{ return; a[0]; }}"))
            .unwrap();
        let was_compiler::typed_ast::TypedStatementKind::Expression(expr) =
            &mut p.functions[0].body[1].kind
        else {
            panic!()
        };
        assert_eq!(expr.ty, scalar);
        expr.ty = Type::Number.into();
        assert!(c.compile(&p).is_err());
    }
}

#[test]
fn empty_and_all_never_literals_retain_storage_independently_of_completion() {
    use was_compiler::typed_ast::{ExprResult, TypedExprKind, TypedStatementKind};
    let c = Compiler::new();
    for (ty, element) in [
        ("number", ArrayElementType::Number),
        ("bool", ArrayElementType::Bool),
        ("string", ArrayElementType::String),
    ] {
        for literal in ["[]", "[loop {}]"] {
            let p = c
                .analyze(&format!("fn main() -> {ty}[] {{ return {literal}; }}"))
                .unwrap();
            let TypedStatementKind::Return(Some(value)) = &p.functions[0].body[0].kind else {
                panic!()
            };
            let TypedExprKind::Array(_, storage) = &value.kind else {
                panic!()
            };
            assert_eq!(*storage, element);
            assert_eq!(
                value.ty,
                if literal == "[]" {
                    Type::Array(element).into()
                } else {
                    ExprResult::Never
                }
            );
            wasmparser::Validator::new()
                .validate_all(&c.compile(&p).unwrap())
                .unwrap();
        }
    }
    let p = c.analyze("fn main() { [loop {}]; }").unwrap();
    let TypedStatementKind::Expression(value) = &p.functions[0].body[0].kind else {
        panic!()
    };
    assert!(matches!(
        value.kind,
        TypedExprKind::Array(_, ArrayElementType::Number)
    ));
    assert_eq!(value.ty, ExprResult::Never);
}

#[test]
fn legacy_numeric_type_diagnostic_text_is_preserved() {
    let error = Compiler::new()
        .analyze("fn main() { len(true); }")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expected NumberArray, found Bool")
    );
}

#[test]
fn numeric_public_migration_preserves_compilation() {
    let compiler = Compiler::new();
    let program = compiler
        .analyze("fn main() -> number[] { return [1, 2]; }")
        .unwrap();
    assert_eq!(
        program.functions[0].return_type,
        Type::Array(ArrayElementType::Number)
    );
    let bytes = compiler.compile(&program).unwrap();
    wasmparser::Validator::new().validate_all(&bytes).unwrap();
    assert!(std::mem::size_of::<Type>() <= 2);
}
