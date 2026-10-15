use was_compiler::{Compiler, ExportOptions, OptimizationLevel, Type};
use wasmparser::Validator;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];
const SOURCE: &str = r#"
fn digits(a: number, b: number, c: number) -> number requires a > 0 {
    mark(90); return a * 100 + b * 10 + c;
}
fn identity(x: number) -> number { return x; }
fn progression(start: number, end: number) -> range { return start..=end by 2; }
fn total(xs: range) -> number { let n = 0; for x in xs { n += x; } return n; }
fn main() -> number { return mark(1) |> digits(mark(2), mark(3)) |> identity(); }
fn bare() -> number { return 5 |> identity; }
fn empty() -> number { return 5 |> identity(); }
fn ranges() -> number { return 1 |> progression(end: 5) |> total; }
fn defaults(a: number = 9, b: number = a + 1, c: number = b + 1) -> number
    requires a > 0 requires b > a requires c > b {
    return a * 100 + b * 10 + c;
}
fn named() -> number { return mark(1) |> digits(c: mark(3), b: mark(2)) |> identity; }
fn mixed() -> number { return mark(1) |> digits(mark(2), c: mark(3)); }
fn default_bare() -> number { return 2 |> defaults; }
fn default_empty() -> number { return 2 |> defaults(); }
fn default_named() -> number { return 2 |> defaults(c: 8); }
fn default_mixed() -> number { return 2 |> defaults(4, c: 8); }
fn default_chain() { return 2 |> defaults |> identity(); }
fn array_default(xs: number[], n: number = len(xs)) -> number { return n; }
fn arrays() -> number { return [1, 2, 3] |> array_default; }
fn text_default(s: string, same: string = s) -> bool { return s == same; }
fn strings() -> bool { return "é🦀" |> text_default; }
fn ordinary(a: number = 2, b: number = a + 1) -> number { return a * 10 + b; }
fn ordinary_calls() -> number { return ordinary() + ordinary(b: 4, a: 3); }
fn sink(a: number, b: number = a + 1) -> void
    requires a > 0 requires b > a { mark(a); mark(b); }
fn sink_wrapper() { 4 |> sink; }
fn sink_trap_wrapper() { mark(1) |> sink(b: mark(0)); mark(99); }
fn discarded(flag: bool) -> number {
    mark(1) /* é🦀 */ |> identity |> /* λ */ sink(
        b: mark(2)
    );
    (3 |> sink());
    { /* é🦀: discarded tail */ (4 |> sink) };
    if flag { 6 |> sink } else { 8 |> sink() }
    (if flag { 10 |> sink } else { 12 |> sink });
    match flag { true => 14 |> sink, false => 16 |> sink(), }
    { if flag { { (18 |> sink) } } else { 20 |> sink } };
    return 65;
}
fn discarded_never() -> number { (loop {}) |> sink; }
"#;
const SELECTED: &[&str] = &[
    "main",
    "bare",
    "empty",
    "ranges",
    "ordinary_calls",
    "named",
    "mixed",
    "defaults",
    "default_bare",
    "default_empty",
    "default_named",
    "default_mixed",
    "default_chain",
    "arrays",
    "strings",
    "sink",
    "sink_wrapper",
    "sink_trap_wrapper",
    "discarded",
    "discarded_never",
];

fn compiler() -> Compiler {
    let mut compiler = Compiler::new();
    compiler
        .register_host_function("mark", "env", "mark", vec![Type::Number], Type::Number)
        .unwrap();
    compiler.set_exports(ExportOptions::Only(
        SELECTED.iter().map(|name| (*name).into()).collect(),
    ));
    compiler
}

#[test]
fn named_default_pipeline_compilation_retains_owned_host_snapshot_and_determinism() {
    let mut native = compiler();
    let analyzed = native.analyze(SOURCE).unwrap();
    let baseline = native.compile_source(SOURCE).unwrap();
    native
        .register_host_function("later", "extra", "later", vec![], Type::Void)
        .unwrap();
    for (_, level) in LEVELS {
        native.set_optimization_level(level);
        for _ in 0..2 {
            assert_eq!(native.compile(&analyzed).unwrap(), baseline);
        }
        let updated = native.compile_source(SOURCE).unwrap();
        Validator::new().validate_all(&updated).unwrap();
        assert_ne!(updated, baseline);
        assert_eq!(native.compile_source(SOURCE).unwrap(), updated);
        let baseline_wat = wasmprinter::print_bytes(&baseline).unwrap();
        let updated_wat = wasmprinter::print_bytes(&updated).unwrap();
        assert!(!baseline_wat.contains("(import \"extra\" \"later\""));
        assert!(updated_wat.contains("(import \"extra\" \"later\""));
        assert!(baseline_wat.contains("(import \"env\" \"mark\""));
    }
}
