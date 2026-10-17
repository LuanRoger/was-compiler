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

impl OptimizationLevel {
    pub(crate) fn numeric_policy(self) -> crate::codegen::NumericPolicy {
        match self {
            Self::None => crate::codegen::NumericPolicy::Baseline,
            Self::Basic | Self::Standard | Self::Aggressive => {
                crate::codegen::NumericPolicy::ProvenIntegers
            }
        }
    }
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
    fn representation_selection_does_not_rewrite_the_input_program() {
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
