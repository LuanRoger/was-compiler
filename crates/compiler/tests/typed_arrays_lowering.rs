use was_compiler::{Compiler, OptimizationLevel};
use wasmparser::{Operator, Parser, Payload, Validator};

#[test]
fn legacy_numeric_never_for_keeps_range_helpers_and_scratch() {
    let bytes = Compiler::new()
        .compile_source("fn main() { for x in [loop {}] {} }")
        .unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    let mut first = true;
    let mut functions = 0;
    let mut locals = 0;
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.unwrap() {
            Payload::FunctionSection(section) => functions = section.count(),
            Payload::CodeSectionEntry(body) if first => {
                first = false;
                locals = body
                    .get_locals_reader()
                    .unwrap()
                    .into_iter()
                    .map(|entry| entry.unwrap().0)
                    .sum::<u32>();
            }
            _ => {}
        }
    }
    assert_eq!(functions, 10); // user + seven memory + two legacy range helpers
    assert_eq!(locals, 7); // binding + five range scratch + array-literal handle
}

#[test]
fn typed_widths_and_helper_families_are_static_deterministic_and_lazy() {
    for (ty, literal, expected_functions) in [
        ("number", "1, 2", 8),
        ("bool", "true, false", 13),
        ("string", "\"one\", \"two\"", 12),
    ] {
        let source = format!(
            "fn main(a: {ty}[]) -> bool {{ let b = [{literal}]; b[0] = a[0]; for x in a {{ let y = x; }} return a == b; }}"
        );
        let mut c = Compiler::new();
        let baseline = c.compile_source(&source).unwrap();
        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            c.set_optimization_level(level);
            let bytes = c.compile_source(&source).unwrap();
            Validator::new().validate_all(&bytes).unwrap();
            if level == OptimizationLevel::None {
                assert_eq!(bytes, baseline);
            }
            assert_eq!(c.compile_source(&source).unwrap(), bytes);
            let program = c.analyze(&source).unwrap();
            assert_eq!(c.compile(&program).unwrap(), bytes);
        }
        Validator::new().validate_all(&baseline).unwrap();
        let mut count = 0;
        let mut first = true;
        let mut user = Vec::new();
        for payload in Parser::new(0).parse_all(&baseline) {
            match payload.unwrap() {
                Payload::FunctionSection(section) => count = section.count(),
                Payload::CodeSectionEntry(body) if first => {
                    first = false;
                    user = body
                        .get_operators_reader()
                        .unwrap()
                        .into_iter()
                        .map(Result::unwrap)
                        .collect();
                }
                _ => {}
            }
        }
        assert_eq!(count, expected_functions, "{ty}");
        match ty {
            "bool" => {
                assert!(user.iter().any(|op| matches!(op, Operator::I32Store8 { memarg } if memarg.offset == 8 && memarg.align == 0)));
                assert!(user.iter().any(|op| matches!(op, Operator::I32Store8 { memarg } if memarg.offset == 9 && memarg.align == 0)));
                assert!(user.iter().any(|op| matches!(op, Operator::I32Load8U { memarg } if memarg.offset == 0 && memarg.align == 0)));
            }
            "string" => {
                assert!(user.iter().any(|op| matches!(op, Operator::I32Store { memarg } if memarg.offset == 8 && memarg.align == 2)));
                assert!(user.iter().any(|op| matches!(op, Operator::I32Store { memarg } if memarg.offset == 12 && memarg.align == 2)));
            }
            _ => {
                assert!(user.iter().any(|op| matches!(op, Operator::F64Store { memarg } if memarg.offset == 8 && memarg.align == 3)));
                assert!(user.iter().any(|op| matches!(op, Operator::F64Store { memarg } if memarg.offset == 16 && memarg.align == 3)));
            }
        }
    }
    let bytes = Compiler::new()
        .compile_source("fn main(a: bool[], b: string[]) {}")
        .unwrap();
    let count = Parser::new(0)
        .parse_all(&bytes)
        .find_map(|p| match p.unwrap() {
            Payload::FunctionSection(s) => Some(s.count()),
            _ => None,
        })
        .unwrap();
    assert_eq!(count, 17);
}
