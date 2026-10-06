use crate::{lexer::Lexer, parser::Parser, semantic::SemanticAnalyzer};

mod ast;
mod lexer;
mod parser;
mod semantic;
mod tokens;
mod typed_ast;

fn main() -> anyhow::Result<()> {
    let source = "fn main() -> void { let d: string = \"Hello World\"; }";
    let tokens = Lexer::new(source).tokenize();
    let program = Parser::new(&tokens).parse_program()?;

    let typed_program = SemanticAnalyzer::new().analyze(&program)?;

    dbg!(typed_program);
    Ok(())
}
