use was_compiler::{
    CodegenError, CompileError, parser::ParseError, semantic::SemanticError, tokens::Span,
};

pub fn render(path: &str, source: &str, error: &CompileError) -> String {
    let (stage, span) = match error {
        CompileError::Parse(error) => ("parse", Some(parse_span(error))),
        CompileError::Semantic(error) => ("semantic", Some(semantic_span(error))),
        CompileError::Codegen(error) => ("code generation", codegen_span(error)),
    };

    let Some(span) = span else {
        return format!("{path}: {stage} error: {error}");
    };

    let start = boundary(source, span.start());
    let end = boundary(source, span.end()).max(start);
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |index| start + index);
    let line = source[..line_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let column = source[line_start..start].chars().count() + 1;
    let snippet = source[line_start..line_end]
        .trim_end_matches('\r')
        .replace('\t', "    ");
    let padding = source[line_start..start]
        .replace('\t', "    ")
        .chars()
        .count();
    let width = source[start..end.min(line_end)]
        .trim_end_matches('\r')
        .replace('\t', "    ")
        .chars()
        .count()
        .max(1);
    let gutter = line.to_string().len();

    format!(
        "{path}:{line}:{column}: {stage} error: {error}\n{blank:>gutter$} |\n{line:>gutter$} | {snippet}\n{blank:>gutter$} | {padding}{underline}",
        blank = "",
        padding = " ".repeat(padding),
        underline = "^".repeat(width),
    )
}

fn boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());

    while !source.is_char_boundary(offset) {
        offset -= 1;
    }

    offset
}

fn parse_span(error: &ParseError) -> Span {
    match error {
        ParseError::UnexpectedToken { span, .. } | ParseError::InvalidAssignmentTarget { span } => {
            *span
        }
    }
}

fn semantic_span(error: &SemanticError) -> Span {
    match error {
        SemanticError::DuplicateFunction { span, .. }
        | SemanticError::DuplicateVariable { span, .. }
        | SemanticError::UndefinedVariable { span, .. }
        | SemanticError::UndefinedFunction { span, .. }
        | SemanticError::NotCallable { span, .. }
        | SemanticError::InvalidVoidType { span, .. }
        | SemanticError::TypeMismatch { span, .. }
        | SemanticError::ArgumentCountMismatch { span, .. }
        | SemanticError::InvalidBinaryOperands { span, .. }
        | SemanticError::InvalidAssignmentTarget { span }
        | SemanticError::UnexpectedReturnValue { span }
        | SemanticError::MissingReturn { span, .. } => *span,
    }
}

fn codegen_span(error: &CodegenError) -> Option<Span> {
    match error {
        CodegenError::UnsupportedStatement { span, .. }
        | CodegenError::UnsupportedExpression { span, .. }
        | CodegenError::UnsupportedOperator { span, .. }
        | CodegenError::InvalidNumberLiteral { span, .. }
        | CodegenError::NumberLiteralOutOfRange { span, .. }
        | CodegenError::TypeMismatch { span, .. }
        | CodegenError::UnknownFunctionReference { span, .. }
        | CodegenError::ArgumentCountMismatch { span, .. }
        | CodegenError::UnsupportedBuiltin { span, .. }
        | CodegenError::DuplicateVariableId { span, .. }
        | CodegenError::UnknownVariable { span, .. }
        | CodegenError::UninitializedVariable { span, .. }
        | CodegenError::UnsupportedLocalType { span, .. }
        | CodegenError::InvalidAssignmentTarget { span, .. }
        | CodegenError::UnexpectedReturnValue { span, .. } => Some(*span),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use was_compiler::Compiler;

    #[test]
    fn compiler_stages_have_locations_and_source_excerpts() {
        for (source, stage) in [
            ("fn main() -> void {\n    let n: number = ;\n}", "parse"),
            ("fn main() -> void {\n    missing();\n}", "semantic"),
            (
                "fn main() -> number {\n    return 1 % 2;\n}",
                "code generation",
            ),
        ] {
            let error = Compiler::new().compile_source(source).unwrap_err();
            let diagnostic = render("program.was", source, &error);

            assert!(diagnostic.starts_with("program.was:2:"));
            assert!(diagnostic.contains(&format!("{stage} error:")));
            assert!(diagnostic.contains("2 |"));
            assert!(diagnostic.contains('^'));
        }
    }

    #[test]
    fn utf8_crlf_tabs_and_multiline_spans_render_without_invalid_slicing() {
        let source = "é\r\n\tβ missing();\r\nnext";
        let start = source.find("missing").unwrap();
        let error = CompileError::Semantic(SemanticError::UndefinedFunction {
            name: "missing".into(),
            span: Span::new(start, source.len()),
        });
        let text = render("unicode.was", source, &error);

        assert!(text.starts_with("unicode.was:2:4: semantic error:"));
        assert!(text.contains("2 |     β missing();"));
        assert!(!text.contains('\r'));
        assert!(text.ends_with(" |       ^^^^^^^^^^"));
    }

    #[test]
    fn malformed_and_eof_spans_are_clamped_safely() {
        for span in [Span::new(1, 1), Span::new(999, 0), Span::new(0, 999)] {
            let error = CompileError::Parse(ParseError::InvalidAssignmentTarget { span });
            let diagnostic = render("input", "é", &error);

            assert!(diagnostic.contains('^'));
        }

        let error = CompileError::Parse(ParseError::InvalidAssignmentTarget {
            span: Span::new(0, 0),
        });

        assert!(render("empty", "", &error).starts_with("empty:1:1:"));
    }

    #[test]
    fn unlocated_errors_do_not_invent_line_numbers() {
        let error = CompileError::Codegen(CodegenError::UnknownExport {
            name: "missing".into(),
        });

        assert_eq!(
            render("program.was", "", &error),
            "program.was: code generation error: unknown user function export `missing`"
        );
    }
}
