use thiserror::Error;

use crate::{
    ast::Type,
    codegen::{self, CodegenError},
    exports::ExportOptions,
    host::{HostFunctionId, HostFunctions, HostRegistrationError},
    lexer::Lexer,
    optimization::{self, OptimizationLevel},
    parser::{ParseError, Parser},
    semantic::{SemanticAnalyzer, SemanticError},
    typed_ast::TypedProgram,
};

#[derive(Debug, Error, PartialEq)]
pub enum CompileError {
    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error(transparent)]
    Semantic(#[from] SemanticError),

    #[error(transparent)]
    Codegen(#[from] CodegenError),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompilerOptions {
    pub optimization_level: OptimizationLevel,
}

#[derive(Debug, Default)]
pub struct Compiler {
    options: CompilerOptions,
    exports: ExportOptions,
    host_functions: HostFunctions,
}

impl Compiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: CompilerOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    pub fn options(&self) -> &CompilerOptions {
        &self.options
    }

    pub fn set_optimization_level(&mut self, level: OptimizationLevel) {
        self.options.optimization_level = level;
    }

    pub fn set_exports(&mut self, exports: ExportOptions) {
        self.exports = exports;
    }

    pub fn exports(&self) -> &ExportOptions {
        &self.exports
    }

    pub fn register_host_function(
        &mut self,
        name: impl Into<String>,
        module: impl Into<String>,
        field: impl Into<String>,
        params: Vec<Type>,
        return_type: Type,
    ) -> Result<HostFunctionId, HostRegistrationError> {
        self.host_functions
            .register(name, module, field, params, return_type)
    }

    pub fn host_functions(&self) -> &HostFunctions {
        &self.host_functions
    }

    pub fn analyze(&self, source: &str) -> Result<TypedProgram, CompileError> {
        let tokens = Lexer::new(source).tokenize();
        let program = Parser::new(&tokens).parse_program()?;

        let typed_program =
            SemanticAnalyzer::with_host_functions(&self.host_functions).analyze(&program)?;

        Ok(typed_program)
    }

    pub fn compile(&self, program: &TypedProgram) -> Result<Vec<u8>, CodegenError> {
        let optimized = optimization::optimize(program, self.options.optimization_level);

        codegen::compile_with_exports(optimized.as_ref(), &self.exports)
    }

    pub fn compile_source(&self, source: &str) -> Result<Vec<u8>, CompileError> {
        let program = self.analyze(source)?;
        let wasm = self.compile(&program)?;

        Ok(wasm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        host::HostFunctionId,
        tokens::Span,
        typed_ast::{FunctionId, FunctionRef, TypedExprKind, TypedStatementKind},
    };

    fn compiler() -> Compiler {
        let mut compiler = Compiler::new();

        compiler
            .register_host_function("print", "env", "log", vec![Type::Number], Type::Void)
            .unwrap();
        compiler
            .register_host_function("ready", "env", "is_ready", vec![], Type::Bool)
            .unwrap();

        compiler
    }

    #[test]
    fn default_compiler_disables_optimization() {
        assert_eq!(
            Compiler::new().options().optimization_level,
            OptimizationLevel::None
        );
        assert_eq!(Compiler::default().options(), &CompilerOptions::default());
    }

    #[test]
    fn options_and_level_changes_preserve_host_registration() {
        let mut compiler = Compiler::with_options(CompilerOptions {
            optimization_level: OptimizationLevel::Standard,
        });

        assert_eq!(
            compiler.options().optimization_level,
            OptimizationLevel::Standard
        );

        compiler
            .register_host_function("clock", "env", "clock", vec![], Type::Number)
            .unwrap();
        compiler.set_optimization_level(OptimizationLevel::Aggressive);

        assert_eq!(
            compiler.options().optimization_level,
            OptimizationLevel::Aggressive
        );
        assert_eq!(compiler.host_functions().functions().len(), 1);
        assert_eq!(compiler.host_functions().functions()[0].name, "clock");
    }

    #[test]
    fn optimization_levels_preserve_wasm_and_the_input_program() {
        let mut compiler = compiler();
        let source = "
            fn main(value: number) -> number {
                let result: number = double(value);
                let items: number[] = [result, 2, 3];
                items[1] = items[0] + len(items);

                while result < 10 {
                    result = result + 1;
                }

                if result >= 10 && ready() {
                    print(result);
                } else {
                    result = -result;
                }

                return result;
            }

            fn flag(value: bool) -> bool {
                let copy: bool = value;
                copy = !copy || ready();
                return copy;
            }

            fn double(value: number) -> number {
                return value * 2;
            }

            fn same(left: string, right: string) -> bool {
                return left == right;
            }
        ";
        let program = compiler.analyze(source).unwrap();
        let original = program.clone();
        let expected = compiler.compile(&program).unwrap();

        for level in [
            OptimizationLevel::None,
            OptimizationLevel::Basic,
            OptimizationLevel::Standard,
            OptimizationLevel::Aggressive,
        ] {
            compiler.set_optimization_level(level);

            assert_eq!(compiler.compile(&program).unwrap(), expected);
            assert_eq!(compiler.compile_source(source).unwrap(), expected);
            assert_eq!(compiler.analyze(source).unwrap(), original);
            assert_eq!(program, original);
        }
    }

    #[test]
    fn optimization_levels_preserve_errors() {
        let mut compiler = Compiler::new();

        let overflow_source = format!("fn main() -> number {{ return {}; }}", "9".repeat(400));

        for source in [
            "fn",
            "fn main() -> void { missing(); }",
            overflow_source.as_str(),
        ] {
            compiler.set_optimization_level(OptimizationLevel::None);

            let expected = compiler.compile_source(source).unwrap_err();

            for level in [
                OptimizationLevel::Basic,
                OptimizationLevel::Standard,
                OptimizationLevel::Aggressive,
            ] {
                compiler.set_optimization_level(level);

                assert_eq!(compiler.compile_source(source).unwrap_err(), expected);
            }
        }
    }

    #[test]
    fn host_calls_are_typed_and_resolved_without_changing_user_ids() {
        let compiler = compiler();
        let program = compiler
            .analyze(
                "fn main() -> void { print(1); if ready() { helper(); } } fn helper() -> void {}",
            )
            .unwrap();

        assert_eq!(
            program.host_functions,
            compiler.host_functions().functions()
        );
        assert_eq!(program.functions[0].id, FunctionId(0));
        assert_eq!(program.functions[1].id, FunctionId(1));

        let TypedStatementKind::Expression(expr) = &program.functions[0].body[0].kind else {
            panic!("expected expression statement");
        };

        let TypedExprKind::Call { func, name, args } = &expr.kind else {
            panic!("expected host call");
        };

        assert_eq!(*func, FunctionRef::Host(HostFunctionId(0)));
        assert_eq!(name, "print");
        assert_eq!(args[0].ty, Type::Number);
        assert_eq!(expr.ty, Type::Void);

        let TypedStatementKind::If { cond, then, .. } = &program.functions[0].body[1].kind else {
            panic!("expected if statement");
        };

        assert_eq!(cond.ty, Type::Bool);
        assert!(matches!(
            cond.kind,
            TypedExprKind::Call {
                func: FunctionRef::Host(HostFunctionId(1)),
                ..
            }
        ));
        assert!(
            matches!(&then[0].kind, TypedStatementKind::Expression(expr) if matches!(expr.kind, TypedExprKind::Call { func: FunctionRef::User(FunctionId(1)), .. }))
        );
    }

    #[test]
    fn host_calls_follow_normal_semantic_rules() {
        let compiler = compiler();

        for (source, expected) in [
            (
                "fn main() -> void { print(); }",
                SemanticError::ArgumentCountMismatch {
                    name: "print".into(),
                    expected: 1,
                    found: 0,
                    span: Span::new(20, 28),
                },
            ),
            (
                "fn main() -> void { print(true); }",
                SemanticError::TypeMismatch {
                    expected: Type::Number,
                    found: Type::Bool,
                    span: Span::new(20, 32),
                },
            ),
            (
                "fn print() -> void {}",
                SemanticError::DuplicateFunction {
                    name: "print".into(),
                    span: Span::new(0, 21),
                },
            ),
            (
                "fn main() -> void { let print: number = 1; print(1); }",
                SemanticError::NotCallable {
                    name: "print".into(),
                    span: Span::new(43, 52),
                },
            ),
        ] {
            assert_eq!(
                compiler.analyze(source).unwrap_err(),
                CompileError::Semantic(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn hosts_survive_analyzer_reuse_and_typed_output_owns_a_snapshot() {
        let compiler = compiler();
        let mut analyzer = SemanticAnalyzer::with_host_functions(compiler.host_functions());
        let tokens = Lexer::new("fn main() -> void { print(1); }").tokenize();
        let ast = Parser::new(&tokens).parse_program().unwrap();
        let expected = analyzer.analyze(&ast).unwrap();

        let tokens = Lexer::new("fn main() -> void { missing(); }").tokenize();
        let invalid = Parser::new(&tokens).parse_program().unwrap();

        assert!(analyzer.analyze(&invalid).is_err());
        assert_eq!(analyzer.analyze(&ast).unwrap(), expected);

        let mut registry = HostFunctions::new();

        registry
            .register("clock", "env", "clock", vec![], Type::Number)
            .unwrap();

        let mut analyzer = SemanticAnalyzer::with_host_functions(&registry);

        registry
            .register("other", "env", "other", vec![], Type::Void)
            .unwrap();

        let program = analyzer
            .analyze(&crate::ast::Program { functions: vec![] })
            .unwrap();

        assert_eq!(program.host_functions.len(), 1);
        assert_eq!(program.host_functions[0].name, "clock");
    }

    #[test]
    fn foundation_compiles_empty_void_functions_with_imports() {
        let compiler = compiler();
        let wasm = compiler
            .compile_source("fn main() -> void {} fn helper(value: number, ready: bool) -> void {}")
            .unwrap();

        wasmparser::Validator::new().validate_all(&wasm).unwrap();

        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        assert_eq!(
            wasm,
            compiler
                .compile_source(
                    "fn main() -> void {} fn helper(value: number, ready: bool) -> void {}"
                )
                .unwrap()
        );
    }

    #[test]
    fn pipeline_preserves_errors_from_each_stage() {
        let compiler = compiler();

        assert!(matches!(
            compiler.compile_source("fn"),
            Err(CompileError::Parse(_))
        ));
        assert!(matches!(
            compiler.compile_source("fn main() -> void { missing(); }"),
            Err(CompileError::Semantic(
                SemanticError::UndefinedFunction { .. }
            ))
        ));
        let literal = "9".repeat(400);
        let source = format!("fn main() -> number {{ return {literal}; }}");
        let program = compiler.analyze(&source).unwrap();

        assert_eq!(
            compiler.compile_source(&source),
            Err(CompileError::Codegen(
                CodegenError::NumberLiteralOutOfRange {
                    function: "main".into(),
                    literal,
                    span: program.functions[0].body[0].span,
                }
            ))
        );
        assert!(
            compiler
                .compile_source("fn main() -> void { print(1); }")
                .is_ok()
        );
        assert!(compiler.compile_source("fn main() -> void {}").is_ok());
    }

    #[test]
    fn compiling_existing_typed_program_uses_its_import_snapshot() {
        let mut compiler = Compiler::new();

        compiler
            .register_host_function("clock", "env", "clock", vec![], Type::Number)
            .unwrap();

        let program = compiler.analyze("fn main() -> void {}").unwrap();
        let expected = compiler.compile(&program).unwrap();

        compiler
            .register_host_function("print", "env", "print", vec![Type::Number], Type::Void)
            .unwrap();

        assert_eq!(compiler.compile(&program).unwrap(), expected);
        assert_ne!(
            compiler.compile_source("fn main() -> void {}").unwrap(),
            expected
        );
    }
}
