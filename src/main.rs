use was_compiler::Compiler;

fn main() -> anyhow::Result<()> {
    let compiler = Compiler::new();
    let wasm = compiler.compile_source("fn main() -> number { return 42; }")?;

    let wat = wasmprinter::print_bytes(&wasm)?;

    print!("{wat}");

    Ok(())
}
