# was-compiler

A typed language compiler targeting WebAssembly with `wasm-encoder`.

See the [compilation roadmap](docs/compilation-roadmap.md) for the architecture,
host ABI, milestones, and acceptance criteria.

## Current compilation support

The compiler emits valid modules containing registered function imports, empty
`void` functions, bare `return;` statements, and numeric-literal returns such as
`fn main() -> number { return 42; }`. Numeric literals become `f64.const`; integers
and decimals use the same `f64` representation. Invalid and overflowing literals
return explicit code-generation errors.

All user functions are exported. Arithmetic, variable access, calls, and control
flow are deliberately rejected until their milestones are implemented.

```rust
use was_compiler::{Compiler, Type};

let mut compiler = Compiler::new();

compiler.register_host_function(
    "print",
    "env",
    "log_number",
    vec![Type::Number],
    Type::Void,
)?;

let typed = compiler.analyze("fn main() -> number { return 42; }")?;
let wasm = compiler.compile(&typed)?;
```

The embedding runtime must provide `env.log_number`. Registration declares its
signature and import names; it does not register a runtime callback. Host calls
can already be type-checked, but instruction emission for calls is a later
milestone.

Use `Compiler::compile_source` for the full pipeline, or `Compiler::analyze` and
`Compiler::compile` separately to inspect the typed AST. The typed program owns
its host-import snapshot, so later registry changes do not alter its compilation.

## Optimization levels

The compiler supports configuring `OptimizationLevel::None` (the default),
`Basic`, `Standard`, or `Aggressive`, corresponding to the intended O0–O3 presets.
**No optimization passes exist yet:** all levels currently produce identical Wasm
and preserve the same diagnostics.

```rust
use was_compiler::{Compiler, CompilerOptions, OptimizationLevel};

let mut compiler = Compiler::with_options(CompilerOptions {
    optimization_level: OptimizationLevel::Standard,
});

compiler.set_optimization_level(OptimizationLevel::None);
```

Both `compile` and `compile_source` go through an optimization stage between the
typed AST and code generation. `analyze` remains independent of the selected
level. The stage currently borrows the input without cloning or changing it;
future transformations can return an owned typed AST. Calling the lower-level
`codegen::compile` directly bypasses compiler options and optimization.

Run the sample with `cargo run --offline` to print plain WAT with `wasmprinter`.
Run tests with `cargo test --offline`. Optional runtime tests require Node.js:

```text
cargo test --offline codegen::tests::node_executes -- --include-ignored
```
