use std::ops::ControlFlow;

use crate::typed_ast::{TypedBlock, TypedExpr, TypedExprKind};

use super::{
    CodegenError,
    walker::{self, Visitor},
};

pub(super) fn count(block: &TypedBlock) -> Result<u32, CodegenError> {
    let mut visitor = TemporaryVisitor { count: 0 };

    if let ControlFlow::Break(error) = walker::walk_block(block, &mut visitor) {
        return Err(error);
    }

    Ok(visitor.count)
}

struct TemporaryVisitor {
    count: u32,
}

impl Visitor for TemporaryVisitor {
    type Break = CodegenError;

    fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<CodegenError> {
        if matches!(expr.kind, TypedExprKind::Array(_)) {
            match self.count.checked_add(1) {
                Some(count) => self.count = count,
                None => {
                    return ControlFlow::Break(CodegenError::LimitExceeded {
                        what: "temporary local count",
                    });
                }
            }
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
            ty: Type::Number,
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
    fn overflow_aborts_at_the_array_before_visiting_children() {
        let block = vec![statement(TypedStatementKind::Expression(expr(
            TypedExprKind::Array(vec![array()]),
        )))];
        let mut visitor = TemporaryVisitor { count: u32::MAX };

        assert_eq!(
            walker::walk_block(&block, &mut visitor),
            ControlFlow::Break(CodegenError::LimitExceeded {
                what: "temporary local count"
            },)
        );
        assert_eq!(visitor.count, u32::MAX);
    }
}
