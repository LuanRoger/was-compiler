use std::borrow::Cow;

use crate::typed_ast::TypedProgram;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationLevel {
    #[default]
    None,
    Basic,
    Standard,
    Aggressive,
}

pub(crate) fn optimize(program: &TypedProgram, level: OptimizationLevel) -> Cow<'_, TypedProgram> {
    match level {
        OptimizationLevel::None
        | OptimizationLevel::Basic
        | OptimizationLevel::Standard
        | OptimizationLevel::Aggressive => Cow::Borrowed(program),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_borrow_the_input_until_optimization_passes_exist() {
        let program = TypedProgram {
            host_functions: vec![],
            functions: vec![],
        };

        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            let optimized = optimize(&program, level);

            assert!(matches!(optimized, Cow::Borrowed(_)));
            assert!(std::ptr::eq(optimized.as_ref(), &program));
        }
    }
}
