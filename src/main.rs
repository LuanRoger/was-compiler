use crate::{lexer::Lexer, parser::Parser};

mod ast;
mod lexer;
mod parser;
mod tokens;

fn main() -> anyhow::Result<()> {
    let source = "fn main() -> void { let d: string = \"Hello World\"; }";
    let tokens = Lexer::new(source).tokenize();
    let program = Parser::new(&tokens).parse_program()?;

    dbg!(program);
    Ok(())
}
