use crate::lexer::Lexer;

mod lexer;
mod span;
mod tokens;

fn main() {
    let source = "fn main() {}";
    let tokens = Lexer::new(source).tokenize();

    dbg!(tokens);
}
