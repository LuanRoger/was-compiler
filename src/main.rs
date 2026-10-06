use was_compiler::Compiler;

fn main() -> anyhow::Result<()> {
    let compiler = Compiler::new();
    let source = "
        fn main(value: number) -> number {
            let result: number = value * 2;
            result = result + 1;
            result;
            return result;
        }
    ";

    let wasm = compiler.compile_source(source)?;

    let wat = wasmprinter::print_bytes(&wasm)?;

    print!("{wat}");

    Ok(())
}
