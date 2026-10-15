use was_compiler::{
    Compiler, ExportOptions, Type, codegen, lexer::Lexer, parser::Parser,
    semantic::SemanticAnalyzer,
};

fn main() {
    let source = "fn main() -> number { return echo(21); }";
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("echo", "env", "echo", vec![Type::Number], Type::Number)
        .unwrap();
    let hosts = compiler.host_functions().clone();
    let typed = compiler.analyze(source).unwrap();
    let expected = compiler.compile_source(source).unwrap();
    assert_eq!(&expected[..8], b"\0asm\x01\0\0\0");

    compiler
        .register_host_function("later", "env", "later", vec![], Type::Void)
        .unwrap();
    assert_eq!(compiler.compile(&typed).unwrap(), expected);
    assert_eq!(codegen::compile(&typed).unwrap(), expected);

    let tokens = Lexer::new(source).tokenize();
    let ast = Parser::new(&tokens).parse_program().unwrap();
    let analyzed = SemanticAnalyzer::with_host_functions(&hosts)
        .analyze(&ast)
        .unwrap();
    assert_eq!(compiler.compile(&analyzed).unwrap(), expected);

    compiler.set_exports(ExportOptions::None);
    assert_ne!(compiler.compile(&typed).unwrap(), expected);
    println!("downstream compiler API and owned host snapshot passed");
}
