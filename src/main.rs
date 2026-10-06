use was_compiler::{Compiler, Type};

fn main() -> anyhow::Result<()> {
    let compiler = Compiler::new();

    let source = "
        fn main(value: number) -> number {
            let result: number = double(value);
            return result;
        }

        fn double(value: number) -> number {
            return value * 2;
        }
    ";

    let wasm = compiler.compile_source(source)?;

    let wat = wasmprinter::print_bytes(&wasm)?;

    print!("{wat}");

    Ok(())
}
