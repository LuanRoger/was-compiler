# was-compiler

A typed language compiler targeting WebAssembly with `wasm-encoder`.

See the [compilation roadmap](docs/compilation-roadmap.md) for the architecture,
host ABI, milestones, and acceptance criteria.

## Current compilation support

The compiler emits valid modules containing registered function imports, scalar
parameters, scalar `let` declarations, variable reads and assignments, expression
statements, numeric/boolean returns, and user/host function calls. Empty `void`
functions and bare `return;` statements are supported too. Calls can be forward,
nested, self-recursive, or mutually recursive; arguments are evaluated left-to-right.

```text
fn main(value: number) -> number {
    let result: number = double(value);
    return result;
}

fn double(value: number) -> number {
    return value * 2;
}
```

It supports `+`, `-`, `*`, `/`, unary negation, and nested/parenthesized expressions.
Resolved variable IDs map to per-function Wasm slots, with parameters first and
locals in declaration order. Reads use `local.get`; initialization and assignment
use `local.set`. Scalar expression statements end in `drop` to keep the stack
balanced. Boolean literals use `i32.const 0` or `i32.const 1`.

Numeric literals become `f64.const`; arithmetic uses `f64.add`, `f64.sub`,
`f64.mul`, `f64.div`, and `f64.neg`. Integers and decimals use the same `f64`
representation. Invalid and overflowing literals return explicit errors;
arithmetic follows IEEE-754 behavior, including infinity, NaN, and signed zero.

All user functions are exported. `%` is explicitly unsupported until floating
remainder lowering is implemented; Wasm has no `f64.rem`. Boolean operators,
control flow, strings, arrays, and built-in `len` remain unsupported until their
milestones. Guest-only recursive base cases require the control-flow milestone.
The local layout accounts for distinct shadowed IDs in nested blocks, but executing
those blocks still requires the control-flow milestone.

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

let typed = compiler.analyze("fn main() -> number { print(1 + 2 * 3); return 7; }")?;
let wasm = compiler.compile(&typed)?;
```

The embedding runtime must provide `env.log_number`. Registration declares its
signature and import names; it does not register a runtime callback. Function
references resolve through the precomputed module layout, independently of names
or deduplicated type indices. Numeric/boolean call results used as statements are
`drop`ped; void calls leave no stack value and receive no `drop`.

Host `number` values use `f64`, including IEEE-754 NaN and infinities. Boolean
arguments sent to hosts and boolean results received from hosts are normalized to
`0` or `1`, treating any nonzero `i32` as true. Hosts must obey their registered
signatures. Every registered import must be provided, including unused ones.

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
