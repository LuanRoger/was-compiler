use crate::{
    ast::{BinaryOp, MatchPattern, MatchPatternKind, Type},
    number_literal,
    tokens::Span,
    typed_ast::ExprResult,
};

pub(crate) fn number(value: &str) -> Option<f64> {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let parsed = number_literal::parse(unsigned).ok()?;
    Some(if value.starts_with('-') {
        -parsed
    } else {
        parsed
    })
}

// A zero-width origin span marks whole-match errors; callers supply the enclosing
// location. Pattern-specific errors always retain the original pattern span.
pub(crate) fn check<'a>(
    scrutinee: ExprResult,
    patterns: impl IntoIterator<Item = &'a MatchPattern>,
) -> Result<(), (&'static str, Span)> {
    let overall = Span::new(0, 0);
    let mut domain = match scrutinee {
        ExprResult::Value(ty @ (Type::Number | Type::Bool | Type::String)) => Some(ty),
        ExprResult::Never => None,
        _ => return Err(("scrutinee must be number, bool, or string", overall)),
    };
    let mut count = 0;
    let mut wildcard = false;
    let mut booleans = [false; 2];

    for pattern in patterns {
        count += 1;
        if wildcard {
            return Err(("wildcard must occur once and last", pattern.span));
        }
        let invalid_number = || {
            (
                "pattern number must be a finite decimal literal",
                pattern.span,
            )
        };
        let ty = match &pattern.kind {
            MatchPatternKind::Wildcard => {
                wildcard = true;
                continue;
            }
            MatchPatternKind::Number(value) => {
                number(value).ok_or_else(invalid_number)?;
                Type::Number
            }
            MatchPatternKind::Relational { op, value } => {
                if !matches!(
                    op,
                    BinaryOp::LessThan
                        | BinaryOp::LessThanOrEqual
                        | BinaryOp::GreaterThan
                        | BinaryOp::GreaterThanOrEqual
                ) {
                    return Err(("invalid relational pattern operator", pattern.span));
                }
                number(value).ok_or_else(invalid_number)?;
                Type::Number
            }
            MatchPatternKind::Interval { start, end, .. } => {
                number(start).ok_or_else(invalid_number)?;
                number(end).ok_or_else(invalid_number)?;
                Type::Number
            }
            MatchPatternKind::String(_) => Type::String,
            MatchPatternKind::Bool(value) => {
                booleans[usize::from(*value)] = true;
                Type::Bool
            }
        };
        if domain.is_some_and(|domain| domain != ty) {
            return Err((
                "pattern domain does not match scrutinee or earlier patterns",
                pattern.span,
            ));
        }
        domain = Some(ty);
    }

    if count == 0 {
        return Err(("match requires at least one arm", overall));
    }
    if !wildcard && !(domain == Some(Type::Bool) && booleans == [true, true]) {
        return Err(("match patterns must be exhaustive", overall));
    }
    Ok(())
}
