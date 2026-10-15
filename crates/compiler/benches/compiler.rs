#[path = "support/corpus.rs"]
mod corpus;

use corpus::{CASES, Workload};
use divan::{Bencher, black_box, black_box_drop};
use was_compiler::{Compiler, lexer::Lexer, parser::Parser, semantic::SemanticAnalyzer};
use wasmparser::Validator;

fn validated_source(workload: Workload) -> String {
    let source = corpus::source(workload);
    let bytes = Compiler::new().compile_source(&source).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    source
}

fn main() {
    eprintln!("workload,source_bytes,module_bytes");
    for workload in CASES {
        let source = validated_source(workload);
        let bytes = Compiler::new().compile_source(&source).unwrap();
        eprintln!("{workload},{},{}", source.len(), bytes.len());
    }

    divan::main();
}

#[divan::bench(args = CASES)]
fn lex(bencher: Bencher, workload: Workload) {
    let source = validated_source(workload);
    bencher.bench_local(|| {
        black_box_drop(Lexer::new(black_box(&source)).tokenize());
    });
}

#[divan::bench(args = CASES)]
fn parse(bencher: Bencher, workload: Workload) {
    let source = validated_source(workload);
    let tokens = Lexer::new(&source).tokenize();
    bencher.bench_local(|| {
        black_box_drop(Parser::new(black_box(&tokens)).parse_program().unwrap());
    });
}

#[divan::bench(args = CASES)]
fn semantic(bencher: Bencher, workload: Workload) {
    let source = validated_source(workload);
    let tokens = Lexer::new(&source).tokenize();
    let ast = Parser::new(&tokens).parse_program().unwrap();
    bencher.bench_local(|| {
        black_box_drop(SemanticAnalyzer::new().analyze(black_box(&ast)).unwrap());
    });
}

#[divan::bench(args = CASES)]
fn typed_compile(bencher: Bencher, workload: Workload) {
    let source = validated_source(workload);
    let typed = Compiler::new().analyze(&source).unwrap();
    bencher.bench_local(|| {
        black_box_drop(Compiler::new().compile(black_box(&typed)).unwrap());
    });
}

#[divan::bench(args = CASES)]
fn source_compile(bencher: Bencher, workload: Workload) {
    let source = validated_source(workload);
    bencher.bench_local(|| {
        black_box_drop(Compiler::new().compile_source(black_box(&source)).unwrap());
    });
}
