#[path = "../../../tests/support/integer_corpus.rs"]
mod corpus;

use divan::{Bencher, black_box, black_box_drop};
use wasmparser::Validator;

fn main() {
    let level = corpus::measurement_level();
    eprintln!("level={level:?}; workload,source_bytes,module_bytes");
    for case in corpus::cases() {
        let bytes = corpus::compiler(level)
            .compile_source(&case.source)
            .unwrap();
        Validator::new().validate_all(&bytes).unwrap();
        eprintln!("{},{},{}", case.name, case.source.len(), bytes.len());
    }
    divan::main();
}

#[divan::bench(args = corpus::cases())]
fn source_compile(bencher: Bencher, case: &corpus::Case) {
    let level = corpus::measurement_level();
    let bytes = corpus::compiler(level)
        .compile_source(&case.source)
        .unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    bencher.bench_local(|| {
        // Compiler construction, host declarations and destruction are included.
        black_box_drop(
            corpus::compiler(level)
                .compile_source(black_box(&case.source))
                .unwrap(),
        );
    });
}

#[divan::bench(args = corpus::cases())]
fn typed_compile(bencher: Bencher, case: &corpus::Case) {
    let level = corpus::measurement_level();
    let compiler = corpus::compiler(level);
    let typed = compiler.analyze(&case.source).unwrap();
    let bytes = compiler.compile(&typed).unwrap();
    Validator::new().validate_all(&bytes).unwrap();
    bencher.bench_local(|| {
        black_box_drop(corpus::compiler(level).compile(black_box(&typed)).unwrap());
    });
}
