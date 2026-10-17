use std::collections::HashMap;

use crate::{
    ast::{BinaryOp, Type},
    number_literal,
    typed_ast::{TypedExpr, TypedExprKind},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericPolicy {
    #[default]
    Baseline,
    ProvenIntegers,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum Representation {
    #[default]
    F64,
    I32,
}

// A witness of integrality, signed-i32 bounds, and absence of negative zero.
// Unknown facts (including any possible negative zero) have no witness.
#[derive(Clone, Copy)]
struct IntegerBounds {
    min: i32,
    max: i32,
}

#[derive(Default)]
pub(super) struct Facts {
    policy: NumericPolicy,
    // Keys identify borrowed, immutable nodes for this one body emission only.
    // The map is lookup-only: addresses never determine emitted order or bytes.
    integers: HashMap<*const TypedExpr, Option<IntegerBounds>>,
}

impl Facts {
    pub fn new(policy: NumericPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn representation(&mut self, expr: &TypedExpr) -> Representation {
        if self.policy == NumericPolicy::ProvenIntegers && self.integer(expr).is_some() {
            Representation::I32
        } else {
            Representation::F64
        }
    }

    fn integer(&mut self, expr: &TypedExpr) -> Option<IntegerBounds> {
        let key = std::ptr::from_ref(expr);
        if let Some(facts) = self.integers.get(&key) {
            return *facts;
        }

        let facts = if expr.ty != Type::Number {
            None
        } else {
            match &expr.kind {
                TypedExprKind::Number(literal) => {
                    number_literal::parse(literal).ok().and_then(|value| {
                        if value.fract() == 0.0
                            && !(value == 0.0 && value.is_sign_negative())
                            && value >= f64::from(i32::MIN)
                            && value <= f64::from(i32::MAX)
                        {
                            // Bounds and integrality were checked before conversion.
                            let value = value as i32;
                            Some(IntegerBounds {
                                min: value,
                                max: value,
                            })
                        } else {
                            None
                        }
                    })
                }
                TypedExprKind::Binary {
                    left,
                    op: op @ (BinaryOp::Add | BinaryOp::Subtract),
                    right,
                } => self.binary(left, *op, right),
                _ => None,
            }
        };
        self.integers.insert(key, facts);
        facts
    }

    fn binary(
        &mut self,
        left: &TypedExpr,
        op: BinaryOp,
        right: &TypedExpr,
    ) -> Option<IntegerBounds> {
        let left = self.integer(left)?;
        let right = self.integer(right)?;
        // Widen before arithmetic: even two endpoint bounds fit comfortably in i64.
        let (min, max) = match op {
            BinaryOp::Add => (
                i64::from(left.min) + i64::from(right.min),
                i64::from(left.max) + i64::from(right.max),
            ),
            BinaryOp::Subtract => (
                i64::from(left.min) - i64::from(right.max),
                i64::from(left.max) - i64::from(right.min),
            ),
            _ => return None,
        };
        // All integer intermediates in this range are exact binary64 values.
        // Addition/subtraction of non-negative-zero integers cannot produce -0.
        Some(IntegerBounds {
            min: i32::try_from(min).ok()?,
            max: i32::try_from(max).ok()?,
        })
    }
}
