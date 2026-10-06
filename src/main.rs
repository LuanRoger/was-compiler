use was_compiler::Compiler;

fn main() -> anyhow::Result<()> {
    let compiler = Compiler::new();
    let wasm = compiler.compile_source("fn main() -> void {}")?;

    println!("Compiled {} bytes of WebAssembly", wasm.len());

    Ok(())
}
