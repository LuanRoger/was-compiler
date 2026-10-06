use crate::{lexer::Lexer, parser::Parser, semantic::SemanticAnalyzer};

mod ast;
mod lexer;
mod parser;
mod semantic;
mod tokens;

fn main() -> anyhow::Result<()> {
    let source = "fn main() -> void { let d: string = \"Hello World\"; }";
    let tokens = Lexer::new(source).tokenize();
    let program = Parser::new(&tokens).parse_program()?;

    SemanticAnalyzer::new().analyze(&program)?;

    dbg!(program);
    Ok(())
}
