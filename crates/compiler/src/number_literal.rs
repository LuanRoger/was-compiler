#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralError {
    Invalid,
    OutOfRange,
}

pub(crate) fn parse(literal: &str) -> Result<f64, LiteralError> {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let valid = match literal.split_once('.') {
        Some((integer, fraction)) => digits(integer) && digits(fraction),
        None => digits(literal),
    };

    if !valid {
        return Err(LiteralError::Invalid);
    }

    let value = literal.parse::<f64>().map_err(|_| LiteralError::Invalid)?;

    if !value.is_finite() {
        return Err(LiteralError::OutOfRange);
    }

    Ok(value)
}
