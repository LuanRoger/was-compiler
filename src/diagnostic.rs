use crate::CompileError;

pub fn render(path: &str, source: &str, error: &CompileError) -> String {
    let stage = match error {
        CompileError::Parse(_) => "parse",
        CompileError::Semantic(_) => "semantic",
        CompileError::Codegen(_) => "code generation",
    };

    let Some(span) = error.span() else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CodegenError, Compiler, parser::ParseError, semantic::SemanticError, tokens::Span,
    };

    #[test]
    fn compiler_stages_have_locations_and_source_excerpts() {
        let overflow_source = format!(
            "fn main() -> number {{\n    return {};\n}}",
            "9".repeat(400)
        );

        for (source, stage) in [
            ("fn main() -> void {\n    let n: number = ;\n}", "parse"),
            ("fn main() -> void {\n    missing();\n}", "semantic"),
            (overflow_source.as_str(), "code generation"),
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
