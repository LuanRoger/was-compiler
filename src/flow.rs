/// Possible completions of a statement or block, independent of static checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Flow(u8);

impl Flow {
    pub(crate) const FALLTHROUGH: Self = Self(1);
    pub(crate) const RETURN: Self = Self(2);
    pub(crate) const BREAK: Self = Self(4);
    pub(crate) const CONTINUE: Self = Self(8);

    /// Only normally completing paths reach the next statement.
    pub(crate) fn then(self, next: Self) -> Self {
        if self.can_fall_through() {
            Self((self.0 & !Self::FALLTHROUGH.0) | next.0)
        } else {
            self
        }
    }

    pub(crate) fn join(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) fn always_returns(self) -> bool {
        self == Self::RETURN
    }

    pub(crate) fn can_fall_through(self) -> bool {
        self.0 & Self::FALLTHROUGH.0 != 0
    }

    /// A while may execute zero times; loop transfers never escape its boundary.
    pub(crate) fn while_loop(self) -> Self {
        Self(Self::FALLTHROUGH.0 | (self.0 & Self::RETURN.0))
    }
}

#[cfg(test)]
mod tests {
    use super::Flow;

    #[test]
    fn only_a_pure_return_guarantees_function_return() {
        assert!(Flow::RETURN.always_returns());

        for outcome in [Flow::FALLTHROUGH, Flow::BREAK, Flow::CONTINUE] {
            assert!(!outcome.always_returns());
            assert!(!outcome.join(Flow::RETURN).always_returns());
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
        assert!(!transfers.then(Flow::RETURN).always_returns());
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
        assert!(!conditional_break.then(Flow::RETURN).always_returns());
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
        assert!(!Flow::RETURN.while_loop().always_returns());
        assert_eq!(
            Flow::BREAK.then(Flow::RETURN).while_loop(),
            Flow::FALLTHROUGH
        );
        assert_eq!(
            Flow::CONTINUE.then(Flow::RETURN).while_loop(),
            Flow::FALLTHROUGH
        );
        assert!(Flow::BREAK.while_loop().then(Flow::RETURN).always_returns());
        assert_eq!(
            Flow::RETURN.while_loop().while_loop(),
            Flow::FALLTHROUGH.join(Flow::RETURN)
        );
    }
}
