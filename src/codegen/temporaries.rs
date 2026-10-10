use std::ops::ControlFlow;

use crate::typed_ast::{TypedBlock, TypedExpr, TypedExprKind, TypedStatement, TypedStatementKind};
use wasm_encoder::ValType;

use super::{
    CodegenError,
    walker::{self, Visitor},
};

pub(super) fn plan(block: &TypedBlock) -> Result<Vec<ValType>, CodegenError> {
    let mut visitor = TemporaryVisitor {
        count: 0,
        types: Vec::new(),
    };

    if let ControlFlow::Break(error) = walker::walk_block(block, &mut visitor) {
        return Err(error);
    }

    Ok(visitor.types)
}

#[cfg(test)]
fn count(block: &TypedBlock) -> Result<u32, CodegenError> {
    super::checked_u32(plan(block)?.len(), "temporary local count")
}

struct TemporaryVisitor {
    count: u32,
    types: Vec<ValType>,
}

impl TemporaryVisitor {
    fn reserve(&mut self, types: &[ValType]) -> ControlFlow<CodegenError> {
        let additional = match super::checked_u32(types.len(), "temporary local count") {
            Ok(count) => count,
            Err(error) => return ControlFlow::Break(error),
        };
        let Some(count) = self.count.checked_add(additional) else {
            return ControlFlow::Break(CodegenError::LimitExceeded {
                what: "temporary local count",
            });
        };

        self.count = count;
        self.types.extend_from_slice(types);

        ControlFlow::Continue(())
    }
}

impl Visitor for TemporaryVisitor {
    type Break = CodegenError;

    fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<CodegenError> {
        if let TypedStatementKind::CompoundAssignment { target, .. } = &statement.kind
            && matches!(target.kind, TypedExprKind::Index { .. })
        {
            // Reserve before target/RHS children, matching place lowering.
            self.reserve(&[ValType::I32, ValType::F64])?;
        }

        ControlFlow::Continue(())
    }

    fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<CodegenError> {
        if matches!(expr.kind, TypedExprKind::Array(_)) {
            self.reserve(&[ValType::I32])?;
        }

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ast::{BinaryOp, Type, UnaryOp},
        compiler::Compiler,
        tokens::Span,
        typed_ast::{FunctionId, FunctionRef, TypedStatement, TypedStatementKind},
    };

    fn expr(kind: TypedExprKind) -> TypedExpr {
        TypedExpr {
            kind,
            ty: Type::Number.into(),
        }
    }

    fn array() -> TypedExpr {
        expr(TypedExprKind::Array(vec![]))
    }

    fn statement(kind: TypedStatementKind) -> TypedStatement {
        TypedStatement {
            kind,
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn empty_block_has_no_temporary_slots() {
        assert_eq!(count(&vec![]), Ok(0));
    }

    #[test]
    fn arrays_in_every_expression_child_are_counted_without_type_validation() {
        for (value, expected) in [
            (array(), 1),
            (expr(TypedExprKind::Array(vec![array(), array()])), 3),
            (
                expr(TypedExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(array()),
                }),
                1,
            ),
            (
                expr(TypedExprKind::Binary {
                    left: Box::new(array()),
                    op: BinaryOp::And,
                    right: Box::new(array()),
                }),
                2,
            ),
            (
                expr(TypedExprKind::Call {
                    func: FunctionRef::User(FunctionId(0)),
                    name: "call".into(),
                    args: vec![array(), array()],
                }),
                2,
            ),
            (
                expr(TypedExprKind::Index {
                    expr: Box::new(array()),
                    index: Box::new(array()),
                }),
                2,
            ),
        ] {
            let block = vec![statement(TypedStatementKind::Expression(value))];
            assert_eq!(count(&block), Ok(expected));
        }
    }

    #[test]
    fn assignment_targets_conditions_and_returns_are_counted_before_runtime_filtering() {
        for (kind, expected) in [
            (
                TypedStatementKind::Assignment {
                    target: array(),
                    value: array(),
                },
                2,
            ),
            (
                TypedStatementKind::If {
                    cond: Box::new(array()),
                    then: vec![statement(TypedStatementKind::Return(Some(array())))],
                    else_: Some(vec![statement(TypedStatementKind::Expression(array()))]),
                },
                3,
            ),
            (
                TypedStatementKind::While {
                    id: crate::typed_ast::LoopId(0),
                    cond: Box::new(array()),
                    body: vec![statement(TypedStatementKind::Expression(array()))],
                },
                2,
            ),
        ] {
            let block = vec![statement(TypedStatementKind::Return(None)), statement(kind)];
            assert_eq!(count(&block), Ok(expected));
        }
    }

    #[test]
    fn allocation_counts_are_per_occurrence_and_do_not_reuse_branch_or_loop_slots() {
        let program = Compiler::new()
            .analyze(
                "fn test(flag: bool) -> void {
                let a: number[] = [];
                if flag { let b: number[] = [1]; } else { let c: number[] = [2]; }
                while flag { let d: number[] = [3]; }
                return;
                let e: number[] = [4];
            }",
            )
            .unwrap();
        let original = program.clone();

        assert_eq!(count(&program.functions[0].body), Ok(5));
        assert_eq!(program, original);
    }

    #[test]
    fn compound_scratch_types_follow_preorder_without_reusing_skipped_slots() {
        let program = Compiler::new()
            .analyze(
                "fn test(a: number[], n: number) -> void {
                let first = [1];
                n += len([2]);
                if false { a[len([3]) - 1] *= len([4]); }
                while false { a[0] /= 2; }
                return;
                a[0] %= 3;
                let last = [5];
            }",
            )
            .unwrap();
        let original = program.clone();

        assert_eq!(
            plan(&program.functions[0].body).unwrap(),
            [
                ValType::I32, // first array
                ValType::I32, // local compound RHS array, no local-place scratch
                ValType::I32,
                ValType::F64, // skipped indexed place
                ValType::I32,
                ValType::I32, // its index and RHS arrays
                ValType::I32,
                ValType::F64, // loop place
                ValType::I32,
                ValType::F64, // unreachable place
                ValType::I32, // last array
            ]
        );
        let locals = super::super::locals::LocalLayout::new(&program.functions[0]).unwrap();
        assert_eq!(locals.types, [ValType::I32, ValType::I32]);
        assert_eq!(program, original);
    }

    #[test]
    fn compound_pair_overflow_aborts_before_target_and_rhs_children() {
        let block = vec![statement(TypedStatementKind::CompoundAssignment {
            target: expr(TypedExprKind::Index {
                expr: Box::new(array()),
                index: Box::new(array()),
            }),
            op: BinaryOp::Add,
            value: array(),
        })];
        let mut visitor = TemporaryVisitor {
            count: u32::MAX - 1,
            types: Vec::new(),
        };

        assert_eq!(
            walker::walk_block(&block, &mut visitor),
            ControlFlow::Break(CodegenError::LimitExceeded {
                what: "temporary local count"
            })
        );
        assert_eq!(visitor.count, u32::MAX - 1);
        assert!(visitor.types.is_empty());
    }

    #[test]
    fn overflow_aborts_at_the_array_before_visiting_children() {
        let block = vec![statement(TypedStatementKind::Expression(expr(
            TypedExprKind::Array(vec![array()]),
        )))];
        let mut visitor = TemporaryVisitor {
            count: u32::MAX,
            types: Vec::new(),
        };

        assert_eq!(
            walker::walk_block(&block, &mut visitor),
            ControlFlow::Break(CodegenError::LimitExceeded {
                what: "temporary local count"
            },)
        );
        assert_eq!(visitor.count, u32::MAX);
    }
}
