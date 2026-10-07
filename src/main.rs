use was_compiler::Compiler;

fn main() -> anyhow::Result<()> {
    let compiler = Compiler::new();

    let source = "
        fn main() -> number {
            let values: number[] = [10, 20, 30];
            values[1] = 25;

            return sum(values);
        }

        fn sum(values: number[]) -> number {
            let total: number = 0;
            let index: number = 0;

            while index < len(values) {
                total = total + values[index];
                index = index + 1;
            }

            return total;
        }
    ";

    let wasm = compiler.compile_source(source)?;

    let wat = wasmprinter::print_bytes(&wasm)?;

    print!("{wat}");

    Ok(())
}
