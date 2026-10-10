use std::ops::ControlFlow;

use crate::{
    ast::BinaryOp,
    typed_ast::{TypedExpr, TypedExprKind, TypedProgram, TypedStatement, TypedStatementKind},
};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::walker::{self, Visitor};

pub(super) fn needs_remainder(program: &TypedProgram) -> bool {
    program
        .functions
        .iter()
        .any(|function| walker::walk_block(&function.body, &mut RemainderVisitor).is_break())
}

struct RemainderVisitor;

impl Visitor for RemainderVisitor {
    type Break = ();

    fn enter_statement(&mut self, statement: &TypedStatement) -> ControlFlow<()> {
        if matches!(
            statement.kind,
            TypedStatementKind::CompoundAssignment {
                op: BinaryOp::Remainder,
                ..
            }
        ) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn enter_expression(&mut self, expr: &TypedExpr) -> ControlFlow<()> {
        if matches!(
            &expr.kind,
            TypedExprKind::Binary {
                op: BinaryOp::Remainder,
                ..
            }
        ) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

pub(super) fn emit_remainder() -> Function {
    use Instruction::*;

    let mut body = Function::new([(2, ValType::F64)]);

    for instruction in [
        LocalGet(0),
        F64Abs,
        LocalSet(3),
        LocalGet(1),
        F64Abs,
        LocalSet(1),
        LocalGet(0),
        LocalGet(0),
        F64Ne,
        LocalGet(1),
        LocalGet(1),
        F64Ne,
        I32Or,
        LocalGet(3),
        F64Const(f64::INFINITY.into()),
        F64Eq,
        I32Or,
        LocalGet(1),
        F64Const(0.0.into()),
        F64Eq,
        I32Or,
        If(BlockType::Empty),
        F64Const(f64::NAN.into()),
        Return,
        End,
        LocalGet(3),
        LocalGet(1),
        F64Lt,
        If(BlockType::Empty),
        LocalGet(0),
        Return,
        End,
        LocalGet(1),
        LocalSet(2),
        Block(BlockType::Empty),
        Loop(BlockType::Empty),
        LocalGet(2),
        LocalGet(2),
        F64Add,
        LocalGet(3),
        F64Le,
        I32Eqz,
        BrIf(1),
        LocalGet(2),
        LocalGet(2),
        F64Add,
        LocalSet(2),
        Br(0),
        End,
        End,
        Block(BlockType::Empty),
        Loop(BlockType::Empty),
        LocalGet(3),
        LocalGet(2),
        F64Ge,
        If(BlockType::Empty),
        LocalGet(3),
        LocalGet(2),
        F64Sub,
        LocalSet(3),
        End,
        LocalGet(2),
        LocalGet(1),
        F64Eq,
        BrIf(1),
        LocalGet(2),
        F64Const(0.5.into()),
        F64Mul,
        LocalSet(2),
        Br(0),
        End,
        End,
        LocalGet(3),
        LocalGet(0),
        F64Copysign,
        End,
    ] {
        body.instruction(&instruction);
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ast::{Type, UnaryOp},
        tokens::Span,
        typed_ast::{
            FunctionId, FunctionRef, TypedBlock, TypedFunction, TypedStatement, TypedStatementKind,
            VariableId,
        },
    };
    use wasm_encoder::{
        CodeSection, ExportKind, ExportSection, FunctionSection, Module, TypeSection,
    };

    fn expr(kind: TypedExprKind) -> TypedExpr {
        TypedExpr {
            kind,
            ty: Type::Number,
        }
    }

    fn number() -> TypedExpr {
        expr(TypedExprKind::Number("1".to_owned()))
    }

    fn binary(left: TypedExpr, op: BinaryOp, right: TypedExpr) -> TypedExpr {
        expr(TypedExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        })
    }

    fn remainder() -> TypedExpr {
        binary(number(), BinaryOp::Remainder, number())
    }

    fn statement(kind: TypedStatementKind) -> TypedStatement {
        TypedStatement {
            kind,
            span: Span::new(0, 0),
        }
    }

    fn program(body: TypedBlock) -> TypedProgram {
        TypedProgram {
            host_functions: vec![],
            functions: vec![TypedFunction {
                id: FunctionId(0),
                name: "test".to_owned(),
                params: vec![],
                return_type: Type::Void,
                body,
                span: Span::new(0, 0),
            }],
        }
    }

    #[test]
    fn scanner_does_not_require_remainder_for_other_operators_or_leaf_expressions() {
        assert!(!needs_remainder(&TypedProgram {
            host_functions: vec![],
            functions: vec![],
        }));

        let expressions = [
            number(),
            expr(TypedExprKind::String("%".to_owned())),
            expr(TypedExprKind::Bool(false)),
            expr(TypedExprKind::Identifier {
                id: VariableId(0),
                name: "remainder".to_owned(),
            }),
            binary(number(), BinaryOp::Divide, number()),
            expr(TypedExprKind::Array(vec![])),
            expr(TypedExprKind::Call {
                func: FunctionRef::User(FunctionId(0)),
                name: "remainder".to_owned(),
                args: vec![number()],
            }),
        ];
        let mut body = vec![statement(TypedStatementKind::Return(None))];

        body.extend(
            expressions
                .into_iter()
                .map(|value| statement(TypedStatementKind::Expression(value))),
        );

        assert!(!needs_remainder(&program(body)));
    }

    #[test]
    fn scanner_finds_remainder_in_every_expression_child() {
        let expressions = [
            remainder(),
            expr(TypedExprKind::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(remainder()),
            }),
            binary(remainder(), BinaryOp::Add, number()),
            binary(number(), BinaryOp::Multiply, remainder()),
            expr(TypedExprKind::Array(vec![number(), remainder()])),
            expr(TypedExprKind::Call {
                func: FunctionRef::User(FunctionId(0)),
                name: "call".to_owned(),
                args: vec![number(), remainder()],
            }),
            expr(TypedExprKind::Index {
                expr: Box::new(expr(TypedExprKind::Array(vec![remainder()]))),
                index: Box::new(number()),
            }),
            expr(TypedExprKind::Index {
                expr: Box::new(number()),
                index: Box::new(remainder()),
            }),
            binary(expr(TypedExprKind::Bool(false)), BinaryOp::And, remainder()),
        ];

        for value in expressions {
            assert!(needs_remainder(&program(vec![statement(
                TypedStatementKind::Expression(value),
            )])));
        }
    }

    #[test]
    fn scanner_finds_remainder_in_every_statement_child_even_after_return() {
        let statements = [
            TypedStatementKind::Let {
                id: VariableId(0),
                name: "value".to_owned(),
                ty: Type::Number,
                value: remainder(),
            },
            TypedStatementKind::Assignment {
                target: remainder(),
                value: number(),
            },
            TypedStatementKind::Assignment {
                target: number(),
                value: remainder(),
            },
            TypedStatementKind::CompoundAssignment {
                target: number(),
                op: BinaryOp::Remainder,
                value: number(),
            },
            TypedStatementKind::CompoundAssignment {
                target: remainder(),
                op: BinaryOp::Add,
                value: number(),
            },
            TypedStatementKind::CompoundAssignment {
                target: number(),
                op: BinaryOp::Add,
                value: remainder(),
            },
            TypedStatementKind::Expression(remainder()),
            TypedStatementKind::Return(Some(remainder())),
            TypedStatementKind::If {
                cond: Box::new(remainder()),
                then: vec![],
                else_: None,
            },
            TypedStatementKind::If {
                cond: Box::new(expr(TypedExprKind::Bool(false))),
                then: vec![statement(TypedStatementKind::Return(Some(remainder())))],
                else_: None,
            },
            TypedStatementKind::If {
                cond: Box::new(expr(TypedExprKind::Bool(true))),
                then: vec![],
                else_: Some(vec![statement(TypedStatementKind::Expression(remainder()))]),
            },
            TypedStatementKind::While {
                id: crate::typed_ast::LoopId(0),
                cond: Box::new(remainder()),
                body: vec![],
            },
            TypedStatementKind::While {
                id: crate::typed_ast::LoopId(0),
                cond: Box::new(expr(TypedExprKind::Bool(false))),
                body: vec![statement(TypedStatementKind::Expression(remainder()))],
            },
        ];

        for kind in statements {
            assert!(needs_remainder(&program(vec![
                statement(TypedStatementKind::Return(None)),
                statement(kind),
            ])));
        }
    }

    #[test]
    fn compound_remainder_statement_hook_stops_before_expression_traversal() {
        for op in [
            BinaryOp::Add,
            BinaryOp::Subtract,
            BinaryOp::Multiply,
            BinaryOp::Divide,
            BinaryOp::Remainder,
        ] {
            let statement = statement(TypedStatementKind::CompoundAssignment {
                target: number(),
                op,
                value: number(),
            });
            assert_eq!(
                RemainderVisitor.enter_statement(&statement),
                if op == BinaryOp::Remainder {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            );
            assert_eq!(
                needs_remainder(&program(vec![statement])),
                op == BinaryOp::Remainder
            );
        }
    }

    #[test]
    fn scanner_checks_later_functions_without_following_calls() {
        let mut program = program(vec![]);

        program.functions.push(TypedFunction {
            id: FunctionId(1),
            name: "uncalled".to_owned(),
            params: vec![],
            return_type: Type::Number,
            body: vec![statement(TypedStatementKind::Return(Some(remainder())))],
            span: Span::new(0, 0),
        });

        assert!(needs_remainder(&program));
    }

    fn helper_module() -> Vec<u8> {
        let mut module = Module::new();
        let mut types = TypeSection::new();
        let mut functions = FunctionSection::new();
        let mut exports = ExportSection::new();
        let mut code = CodeSection::new();

        types
            .ty()
            .function([ValType::F64, ValType::F64], [ValType::F64]);
        functions.function(0);
        exports.export("remainder", ExportKind::Func, 0);
        code.function(&emit_remainder());

        module.section(&types);
        module.section(&functions);
        module.section(&exports);
        module.section(&code);

        module.finish()
    }

    #[test]
    fn emitted_helper_validates_without_memory_or_imports() {
        wasmparser::Validator::new()
            .validate_all(&helper_module())
            .unwrap();
    }

    #[test]
    #[ignore = "requires Node.js"]
    fn node_executes_exact_remainder_for_extremes_and_random_binary64_inputs() {
        use std::{process::Command, time::Duration};

        let script = r#"
            const fs = require('node:fs');
            const assert = require('node:assert/strict');
            const instance = new WebAssembly.Instance(
                new WebAssembly.Module(fs.readFileSync(0))
            );
            const remainder = instance.exports.remainder;
            const view = new DataView(new ArrayBuffer(8));
            function fromBits(bits) {
                view.setBigUint64(0, bits);
                return view.getFloat64(0);
            }
            function check(x, y) {
                const actual = remainder(x, y);
                const expected = x % y;
                assert(
                    Number.isNaN(expected) ? Number.isNaN(actual) : Object.is(actual, expected),
                    `${x} % ${y}: expected ${expected}, received ${actual}`
                );
            }
            const values = [
                0, -0, 1, -1, 3, -3, 5.5, -5.5, 0.1, -0.1,
                Number.MIN_VALUE, -Number.MIN_VALUE,
                3 * Number.MIN_VALUE, 2 ** -1022,
                fromBits(0x000fffffffffffffn),
                fromBits(0x0010000000000001n),
                Number.MAX_VALUE, -Number.MAX_VALUE,
                fromBits(0x7feffffffffffffen),
                2 ** 1023, 1e308, Infinity, -Infinity, NaN,
                fromBits(0xfff8000000000001n),
            ];
            for (const x of values) {
                for (const y of values) check(x, y);
            }
            for (let exponent = -1074; exponent <= 1023; exponent++) {
                const y = 2 ** exponent;
                check(Number.MAX_VALUE, y);
                check(-Number.MAX_VALUE, -y);
                check(y, Number.MIN_VALUE);
            }
            let state = 0x123456789abcdefn;
            function random() {
                state ^= state << 13n;
                state ^= state >> 7n;
                state ^= state << 17n;
                state = BigInt.asUintN(64, state);
                return fromBits(state);
            }
            for (let i = 0; i < 4096; i++) check(random(), random());
        "#;
        let output = crate::test_process::output(
            Command::new("node").args(["-e", script]),
            Some(&helper_module()),
            Duration::from_secs(15),
        );

        assert!(
            output.status.success(),
            "Node.js failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
