use std::{path::Path, process::Command, time::Duration};

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, Type, diagnostic,
    parser::ParseError,
    semantic::SemanticError,
    tokens::Span,
    typed_ast::{BuiltinFunction, FunctionRef, LoopId, VariableId},
};

#[path = "support/process.rs"]
mod test_process;

fn spans() -> [Span; 4] {
    [
        Span::new(2, 5),
        Span::new(0, 0),
        Span::new(1, 1),
        Span::new(usize::MAX, 0),
    ]
}

fn semantic_errors(span: Span) -> Vec<SemanticError> {
    vec![
        SemanticError::DuplicateFunction {
            name: "name".into(),
            span,
        },
        SemanticError::DuplicateVariable {
            name: "name".into(),
            span,
        },
        SemanticError::UndefinedVariable {
            name: "name".into(),
            span,
        },
        SemanticError::UndefinedFunction {
            name: "name".into(),
            span,
        },
        SemanticError::NotCallable {
            name: "name".into(),
            span,
        },
        SemanticError::InvalidVoidType {
            name: "name".into(),
            span,
        },
        SemanticError::TypeMismatch {
            expected: Type::Number,
            found: Type::Bool,
            span,
        },
        SemanticError::ArgumentCountMismatch {
            name: "name".into(),
            expected: 1,
            found: 0,
            span,
        },
        SemanticError::InvalidBinaryOperands {
            operator: "+",
            left: Type::Number,
            right: Type::Bool,
            span,
        },
        SemanticError::InvalidAssignmentTarget { span },
        SemanticError::BreakOutsideLoop { span },
        SemanticError::ContinueOutsideLoop { span },
        SemanticError::UnexpectedReturnValue { span },
        SemanticError::MissingReturn {
            name: "name".into(),
            span,
        },
    ]
}

fn located_codegen_errors(span: Span) -> Vec<CodegenError> {
    vec![
        CodegenError::UnsupportedStatement {
            function: "main".into(),
            kind: "statement",
            span,
        },
        CodegenError::UnsupportedExpression {
            function: "main".into(),
            kind: "expression",
            span,
        },
        CodegenError::UnsupportedOperator {
            function: "main".into(),
            operator: "+",
            span,
        },
        CodegenError::InvalidNumberLiteral {
            function: "main".into(),
            literal: "invalid".into(),
            span,
        },
        CodegenError::NumberLiteralOutOfRange {
            function: "main".into(),
            literal: "overflow".into(),
            span,
        },
        CodegenError::TypeMismatch {
            function: "main".into(),
            expected: Type::Number,
            found: Type::Bool,
            span,
        },
        CodegenError::UnknownFunctionReference {
            function: "main".into(),
            reference: FunctionRef::Builtin(BuiltinFunction::Len),
            span,
        },
        CodegenError::ArgumentCountMismatch {
            function: "main".into(),
            reference: FunctionRef::Builtin(BuiltinFunction::Len),
            expected: 1,
            found: 0,
            span,
        },
        CodegenError::UnsupportedBuiltin {
            function: "main".into(),
            builtin: BuiltinFunction::Len,
            span,
        },
        CodegenError::DuplicateVariableId {
            function: "main".into(),
            id: VariableId(1),
            span,
        },
        CodegenError::DuplicateLoopId {
            function: "main".into(),
            id: LoopId(1),
            span,
        },
        CodegenError::InvalidLoopTarget {
            function: "main".into(),
            target: LoopId(1),
            span,
        },
        CodegenError::UnknownVariable {
            function: "main".into(),
            id: VariableId(1),
            span,
        },
        CodegenError::UninitializedVariable {
            function: "main".into(),
            id: VariableId(1),
            span,
        },
        CodegenError::UnsupportedLocalType {
            function: "main".into(),
            ty: Type::Void,
            span,
        },
        CodegenError::InvalidAssignmentTarget {
            function: "main".into(),
            span,
        },
        CodegenError::UnexpectedReturnValue {
            function: "main".into(),
            span,
        },
    ]
}

fn unlocated_codegen_errors() -> Vec<CodegenError> {
    vec![
        CodegenError::InvalidFunctionId {
            function: "main".into(),
            expected: 0,
            actual: 1,
        },
        CodegenError::InvalidHostFunctionId {
            function: "host".into(),
            expected: 0,
            actual: 1,
        },
        CodegenError::DuplicateExportName {
            name: "main".into(),
        },
        CodegenError::UnknownExport {
            name: "missing".into(),
        },
        CodegenError::DuplicateExportSelection {
            name: "main".into(),
        },
        CodegenError::UnsupportedAbiType {
            function: "main".into(),
            position: "local",
            ty: Type::Void,
        },
        CodegenError::VoidParameter {
            function: "main".into(),
            index: 0,
        },
        CodegenError::MissingReturnValue {
            function: "main".into(),
            return_type: Type::Number,
        },
        CodegenError::LimitExceeded {
            what: "function count",
        },
    ]
}

#[test]
fn every_parse_variant_and_compile_wrapper_preserve_raw_span_offsets() {
    for span in spans() {
        for error in [
            ParseError::UnexpectedToken {
                expected: "expression",
                found: ";",
                span,
            },
            ParseError::InvalidAssignmentTarget { span },
        ] {
            let message = error.to_string();
            assert_eq!(error.span(), Some(span));
            let wrapped = CompileError::Parse(error);
            assert_eq!(wrapped.span(), Some(span));
            assert_eq!(wrapped.to_string(), message);
        }
    }
}

#[test]
fn every_semantic_variant_and_compile_wrapper_preserve_raw_span_offsets() {
    for span in spans() {
        for error in semantic_errors(span) {
            let message = error.to_string();
            assert_eq!(error.span(), Some(span));
            let wrapped = CompileError::Semantic(error);
            assert_eq!(wrapped.span(), Some(span));
            assert_eq!(wrapped.to_string(), message);
        }
    }
}

#[test]
fn every_located_codegen_variant_and_compile_wrapper_preserve_raw_span_offsets() {
    for span in spans() {
        for error in located_codegen_errors(span) {
            let message = error.to_string();
            assert_eq!(error.span(), Some(span));
            let wrapped = CompileError::Codegen(error);
            assert_eq!(wrapped.span(), Some(span));
            assert_eq!(wrapped.to_string(), message);
        }
    }
}

#[test]
fn every_unlocated_codegen_variant_remains_unlocated_through_rendering() {
    for error in unlocated_codegen_errors() {
        let message = error.to_string();
        assert_eq!(error.span(), None);
        let wrapped = CompileError::Codegen(error);
        assert_eq!(wrapped.span(), None);
        assert_eq!(wrapped.to_string(), message);
        assert_eq!(
            diagnostic::render("input.was", "unrelated source", &wrapped),
            format!("input.was: code generation error: {message}")
        );
    }
}

const FIXTURES: [(&str, &str, &str); 4] = [
    (
        "parse.was",
        include_str!("fixtures/diagnostics/parse.was"),
        include_str!("fixtures/diagnostics/parse.stderr"),
    ),
    (
        "semantic.was",
        include_str!("fixtures/diagnostics/semantic.was"),
        include_str!("fixtures/diagnostics/semantic.stderr"),
    ),
    (
        "codegen.was",
        include_str!("fixtures/diagnostics/codegen.was"),
        include_str!("fixtures/diagnostics/codegen.stderr"),
    ),
    (
        "unlocated.was",
        include_str!("fixtures/diagnostics/unlocated.was"),
        include_str!("fixtures/diagnostics/unlocated.stderr"),
    ),
];

#[test]
fn library_diagnostics_match_pre_refactor_cli_text_exactly() {
    for (name, source, expected) in FIXTURES {
        let mut compiler = Compiler::new();
        if name == "unlocated.was" {
            compiler.set_exports(ExportOptions::Only(vec!["missing".into()]));
        }
        let error = compiler.compile_source(source).unwrap_err();

        assert_eq!(error.span().is_none(), name == "unlocated.was");
        assert_eq!(
            diagnostic::render(name, source, &error),
            expected.strip_suffix('\n').unwrap(),
            "fixture {name}"
        );
    }
}

#[test]
fn cli_diagnostics_match_pre_refactor_text_and_exit_status_exactly() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics");
    for (name, _, expected) in FIXTURES {
        let mut command = Command::new(env!("CARGO_BIN_EXE_was-compiler"));
        command.current_dir(&directory).arg(name);
        if name == "unlocated.was" {
            command.args(["--export", "missing"]);
        }
        let output = test_process::output(&mut command, None, Duration::from_secs(15));

        assert_eq!(output.status.code(), Some(1), "fixture {name}: {output:?}");
        assert!(output.stdout.is_empty(), "fixture {name}: {output:?}");
        assert_eq!(output.stderr, expected.as_bytes(), "fixture {name}");
    }
}
