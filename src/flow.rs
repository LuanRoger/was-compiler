use crate::ast::Type;

/// Possible completions, with reachable return and nearest-loop value-break constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Flow {
    completions: u8,
    break_types: u8,
    first_break_type: Option<Type>,
    return_types: u8,
    first_return_type: Option<Type>,
}

impl Flow {
    pub(crate) const NONE: Self = Self::completion(0);
    pub(crate) const FALLTHROUGH: Self = Self::completion(1);
    pub(crate) const RETURN: Self = Self::completion(2);
    pub(crate) const BREAK: Self = Self::completion(4);
    pub(crate) const CONTINUE: Self = Self::completion(8);
    pub(crate) const DIVERGE: Self = Self::completion(16);

    const fn completion(completions: u8) -> Self {
        Self {
            completions,
            break_types: 0,
            first_break_type: None,
            return_types: 0,
            first_return_type: None,
        }
    }

    fn type_bit(ty: Type) -> u8 {
        match ty {
            Type::Number => 1,
            Type::Bool => 2,
            Type::String => 4,
            Type::NumberArray => 8,
            Type::Void => 16,
            Type::Range => 32,
        }
    }

    pub(crate) fn value_break(ty: Type) -> Self {
        Self {
            break_types: Self::type_bit(ty),
            first_break_type: Some(ty),
            ..Self::BREAK
        }
    }

    pub(crate) fn return_value(ty: Type) -> Self {
        Self {
            return_types: Self::type_bit(ty),
            first_return_type: Some(ty),
            ..Self::RETURN
        }
    }

    pub(crate) fn return_type(self) -> Result<Option<Type>, (Type, Type)> {
        Self::constraint_type(self.first_return_type, self.return_types)
    }

    pub(crate) fn break_type(self) -> Result<Option<Type>, (Type, Type)> {
        Self::constraint_type(self.first_break_type, self.break_types)
    }

    fn constraint_type(first: Option<Type>, types: u8) -> Result<Option<Type>, (Type, Type)> {
        let Some(expected) = first else {
            return Ok(None);
        };

        for found in [
            Type::Number,
            Type::Bool,
            Type::String,
            Type::NumberArray,
            Type::Void,
            Type::Range,
        ] {
            if found != expected && types & Self::type_bit(found) != 0 {
                return Err((expected, found));
            }
        }

        Ok(Some(expected))
    }

    pub(crate) fn returns_or_diverges(self) -> bool {
        self.completions != 0
            && self.completions & !(Self::RETURN.completions | Self::DIVERGE.completions) == 0
    }

    /// Only normally completing paths reach the next statement.
    pub(crate) fn then(self, next: Self) -> Self {
        if self.can_fall_through() {
            Self {
                completions: self.completions & !Self::FALLTHROUGH.completions,
                ..self
            }
            .join(next)
        } else {
            self
        }
    }

    pub(crate) fn without_fallthrough(self) -> Self {
        self.then(Self::NONE)
    }

    pub(crate) fn join(self, other: Self) -> Self {
        Self {
            completions: self.completions | other.completions,
            break_types: self.break_types | other.break_types,
            first_break_type: self.first_break_type.or(other.first_break_type),
            return_types: self.return_types | other.return_types,
            first_return_type: self.first_return_type.or(other.first_return_type),
        }
    }

    pub(crate) fn can_fall_through(self) -> bool {
        self.completions & Self::FALLTHROUGH.completions != 0
    }

    /// A while may execute zero times; loop transfers never escape its boundary.
    pub(crate) fn while_loop(self) -> Self {
        Self {
            completions: Self::FALLTHROUGH.completions
                | (self.completions & (Self::RETURN.completions | Self::DIVERGE.completions)),
            break_types: 0,
            first_break_type: None,
            ..self
        }
    }

    /// A backedge may repeat forever; only a reachable break completes normally.
    pub(crate) fn infinite_loop(self) -> Self {
        let mut completions =
            self.completions & (Self::RETURN.completions | Self::DIVERGE.completions);

        if self.completions & Self::BREAK.completions != 0 {
            completions |= Self::FALLTHROUGH.completions;
        }

        if self.completions & (Self::FALLTHROUGH.completions | Self::CONTINUE.completions) != 0 {
            completions |= Self::DIVERGE.completions;
        }

        Self {
            completions,
            break_types: 0,
            first_break_type: None,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Flow;

    #[test]
    fn empty_outcomes_and_fallthrough_removal_compose() {
        assert!(!Flow::NONE.can_fall_through());
        assert!(!Flow::NONE.returns_or_diverges());
        assert_eq!(Flow::NONE.then(Flow::RETURN), Flow::NONE);
        assert_eq!(Flow::FALLTHROUGH.without_fallthrough(), Flow::NONE);

        for abrupt in [Flow::RETURN, Flow::BREAK, Flow::CONTINUE] {
            assert_eq!(Flow::NONE.join(abrupt), abrupt);
            assert_eq!(abrupt.without_fallthrough(), abrupt);
            assert_eq!(abrupt.join(Flow::FALLTHROUGH).without_fallthrough(), abrupt);
        }
    }

    #[test]
    fn short_circuit_preserves_skipped_rhs_and_lhs_transfers() {
        for rhs in [
            Flow::NONE,
            Flow::FALLTHROUGH,
            Flow::RETURN,
            Flow::BREAK,
            Flow::CONTINUE,
        ] {
            let optional_rhs = Flow::FALLTHROUGH.join(rhs.without_fallthrough());

            assert!(optional_rhs.can_fall_through());
            assert_eq!(Flow::RETURN.then(optional_rhs), Flow::RETURN);
            assert_eq!(Flow::BREAK.then(optional_rhs), Flow::BREAK);
        }

        assert_eq!(
            Flow::FALLTHROUGH.then(Flow::FALLTHROUGH.join(Flow::RETURN.without_fallthrough())),
            Flow::FALLTHROUGH.join(Flow::RETURN)
        );
    }

    #[test]
    fn function_completion_requires_return_or_explicit_divergence() {
        assert!(Flow::RETURN.returns_or_diverges());
        assert!(Flow::DIVERGE.returns_or_diverges());
        assert!(Flow::RETURN.join(Flow::DIVERGE).returns_or_diverges());

        for outcome in [Flow::FALLTHROUGH, Flow::BREAK, Flow::CONTINUE] {
            assert!(!outcome.returns_or_diverges());
            assert!(!outcome.join(Flow::RETURN).returns_or_diverges());
            assert!(!outcome.join(Flow::DIVERGE).returns_or_diverges());
        }

        assert!(Flow::FALLTHROUGH.can_fall_through());
        assert!(Flow::FALLTHROUGH.join(Flow::RETURN).can_fall_through());

        for outcome in [Flow::RETURN, Flow::BREAK, Flow::CONTINUE] {
            assert!(!outcome.can_fall_through());
        }
    }

    #[test]
    fn abrupt_completion_ignores_unreachable_flow() {
        for abrupt in [Flow::RETURN, Flow::BREAK, Flow::CONTINUE] {
            for next in [Flow::FALLTHROUGH, Flow::RETURN, Flow::BREAK, Flow::CONTINUE] {
                assert_eq!(abrupt.then(next), abrupt);
                assert_eq!(Flow::FALLTHROUGH.then(next), next);
            }
        }

        let transfers = Flow::BREAK.join(Flow::CONTINUE);

        assert_eq!(transfers.then(Flow::RETURN), transfers);
        assert!(!transfers.then(Flow::RETURN).returns_or_diverges());
    }

    #[test]
    fn conditional_composition_preserves_prior_abrupt_paths() {
        let conditional_return = Flow::RETURN.join(Flow::FALLTHROUGH);

        assert_eq!(conditional_return.then(Flow::RETURN), Flow::RETURN);
        assert_eq!(
            conditional_return.then(Flow::BREAK),
            Flow::RETURN.join(Flow::BREAK)
        );

        let conditional_break = Flow::BREAK.join(Flow::FALLTHROUGH);

        assert_eq!(
            conditional_break.then(Flow::RETURN),
            Flow::BREAK.join(Flow::RETURN)
        );
        assert!(!conditional_break.then(Flow::RETURN).returns_or_diverges());
        assert_eq!(
            conditional_break.then(Flow::CONTINUE.join(Flow::FALLTHROUGH)),
            Flow::BREAK.join(Flow::CONTINUE).join(Flow::FALLTHROUGH)
        );
        assert_eq!(
            Flow::CONTINUE.join(Flow::FALLTHROUGH).then(Flow::RETURN),
            Flow::CONTINUE.join(Flow::RETURN)
        );
    }

    #[test]
    fn joins_are_unordered_idempotent_outcome_unions() {
        for left in [Flow::FALLTHROUGH, Flow::RETURN, Flow::BREAK, Flow::CONTINUE] {
            assert_eq!(left.join(left), left);

            for right in [Flow::FALLTHROUGH, Flow::RETURN, Flow::BREAK, Flow::CONTINUE] {
                assert_eq!(left.join(right), right.join(left));
            }
        }
    }

    #[test]
    fn value_break_constraints_follow_reachable_sequence_and_nested_boundaries() {
        use crate::ast::Type;

        let number = Flow::value_break(Type::Number);
        let boolean = Flow::value_break(Type::Bool);

        assert_eq!(number.break_type(), Ok(Some(Type::Number)));
        assert_eq!(number.then(boolean).break_type(), Ok(Some(Type::Number)));
        assert_eq!(Flow::RETURN.then(number).break_type(), Ok(None));
        assert_eq!(Flow::DIVERGE.then(number).break_type(), Ok(None));
        assert_eq!(
            number.join(boolean).break_type(),
            Err((Type::Number, Type::Bool))
        );
        assert_eq!(
            boolean.join(number).break_type(),
            Err((Type::Bool, Type::Number))
        );
        assert_eq!(number.infinite_loop().break_type(), Ok(None));
        assert_eq!(number.while_loop().break_type(), Ok(None));
        assert_eq!(
            number.infinite_loop().then(boolean).break_type(),
            Ok(Some(Type::Bool))
        );
        assert_eq!(
            Flow::DIVERGE.infinite_loop().then(boolean).break_type(),
            Ok(None)
        );
    }

    #[test]
    fn infinite_loops_consume_transfers_without_inventing_fallthrough() {
        assert_eq!(Flow::FALLTHROUGH.infinite_loop(), Flow::DIVERGE);
        assert_eq!(Flow::CONTINUE.infinite_loop(), Flow::DIVERGE);
        assert_eq!(Flow::BREAK.infinite_loop(), Flow::FALLTHROUGH);
        assert_eq!(Flow::RETURN.infinite_loop(), Flow::RETURN);
        assert_eq!(Flow::DIVERGE.infinite_loop(), Flow::DIVERGE);
        assert_eq!(
            Flow::RETURN.join(Flow::CONTINUE).infinite_loop(),
            Flow::RETURN.join(Flow::DIVERGE)
        );
        assert_eq!(
            Flow::BREAK.join(Flow::FALLTHROUGH).infinite_loop(),
            Flow::FALLTHROUGH.join(Flow::DIVERGE)
        );
    }

    #[test]
    fn while_consumes_transfers_and_conservatively_completes() {
        for body in [
            Flow::FALLTHROUGH,
            Flow::BREAK,
            Flow::CONTINUE,
            Flow::BREAK.join(Flow::CONTINUE),
        ] {
            assert_eq!(body.while_loop(), Flow::FALLTHROUGH);
            assert_eq!(
                body.join(Flow::RETURN).while_loop(),
                Flow::FALLTHROUGH.join(Flow::RETURN)
            );
        }

        assert_eq!(
            Flow::RETURN.while_loop(),
            Flow::FALLTHROUGH.join(Flow::RETURN)
        );
        assert!(!Flow::RETURN.while_loop().returns_or_diverges());
        assert_eq!(
            Flow::BREAK.then(Flow::RETURN).while_loop(),
            Flow::FALLTHROUGH
        );
        assert_eq!(
            Flow::CONTINUE.then(Flow::RETURN).while_loop(),
            Flow::FALLTHROUGH
        );
        assert!(
            Flow::BREAK
                .while_loop()
                .then(Flow::RETURN)
                .returns_or_diverges()
        );
        assert_eq!(
            Flow::RETURN.while_loop().while_loop(),
            Flow::FALLTHROUGH.join(Flow::RETURN)
        );
    }
}
