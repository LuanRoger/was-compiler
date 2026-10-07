# was-compiler

A typed language compiler targeting WebAssembly with `wasm-encoder`.

See the [compilation roadmap](docs/compilation-roadmap.md) for the architecture,
host ABI, milestones, and acceptance criteria. The [memory ABI](docs/memory-abi.md)
documents string/array layouts, allocation, ownership, and host examples.

## Build and run

```text
cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main
node runtime/run.cjs target/array-sum.wasm
```

The runner prints `65`. No source argument still compiles the built-in example and
prints plain WAT; `--help` lists file/stdin, output-format, export, and host options.
The Node harness uses built-in modules only and requires no npm installation.

- [Runtime integration](docs/runtime-integration.md): CLI usage, host providers,
  export selection, typed calls, artifact metadata, and diagnostic stages.
- [Limitations and alternatives](docs/limitations.md): current restrictions,
  available workarounds, and possible future solutions with tradeoffs.
- [Refactoring plan](docs/refactoring.md): prioritized maintainability work,
  explicitly excluding optimization work.

## Current compilation support

The compiler emits valid modules containing registered function imports,
parameters and locals of every nonvoid language type, variable reads and
assignments, expression statements, returns, user/host function calls, `if`/`else`,
and `while`. Strings, numeric arrays, checked array access/assignment, and built-in
`len` are supported. Empty `void` functions and bare `return;` statements work too.
Calls can be forward, nested, self-recursive, or mutually recursive; arguments
are evaluated left-to-right.

```text
fn factorial(value: number) -> number {
    if value <= 1 {
        return 1;
    }

    return value * factorial(value - 1);
}

fn sum(limit: number) -> number {
    let total: number = 0;
    let current: number = 1;

    while current <= limit {
        total = total + current;
        current = current + 1;
    }

    return total;
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

Numeric comparisons support `==`, `!=`, `<`, `<=`, `>`, and `>=`; booleans support
`==`, `!=`, `!`, `&&`, and `||`. Logical operators short-circuit, so skipped operands
never execute host calls or other side effects. Boolean operations treat any
nonzero `i32` as true and produce canonical `0`/`1` results; numeric comparisons
follow IEEE-754 behavior, including NaN and signed zero.

Branches and loops use structured Wasm control flow, support nested returns, and
preserve shadowed local bindings. Nonvoid functions must return on every path;
a loop alone is not considered a guaranteed return, even with a literal `true`
condition. The backend still validates statements after returns and runtime-skipped
operands, and rejects forged typed ASTs that access branch/loop locals outside
of their scopes.

Strings support literals, parameters/locals/returns, and content-based `==`/`!=`.
Numeric arrays support literals (including `[]`), reference assignment, parameters,
returns, element reads/writes, and `len(array)` returning a number. Array indices
must be finite, integral, nonnegative, and below the length; invalid indices trap
rather than being truncated or reading unrelated memory. `len` does not accept
strings, matching the existing language rules.

```text
fn sum(values: number[]) -> number {
    let total: number = 0;
    let index: number = 0;

    while index < len(values) {
        total = total + values[index];
        index = index + 1;
    }

    return total;
}

fn main() -> number {
    let values: number[] = [10, 20, 30];
    values[1] = 25;

    return sum(values);
}
```

Memory-backed modules export `memory` and `__alloc`. Strings and arrays cross
function boundaries as `i32` descriptor handles, not JavaScript strings/arrays.
Strings are immutable UTF-8 values; array assignments alias mutable storage.
Allocation is an aligned, growable bump allocator with **no freeing or garbage
collection yet**. Scalar-only modules do not include the memory runtime.

All user functions are exported by default. `ExportOptions` or CLI `--export` /
`--no-exports` select the public user-function surface without removing bodies or
imports. Required memory ABI exports remain. `%` is explicitly unsupported until floating
remainder lowering is implemented; Wasm has no `f64.rem`.

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
or deduplicated type indices. All nonvoid call results used as statements are
`drop`ped; void calls leave no stack value and receive no `drop`.

Host `number` values use `f64`, including IEEE-754 NaN and infinities. Boolean
arguments sent to hosts and boolean results received from hosts are normalized to
`0` or `1`, treating any nonzero `i32` as true. Hosts must obey their registered
signatures. Host string/array arguments and results have their descriptor bounds
validated; hosts construct return values using the module's allocator and memory.
See the [memory ABI](docs/memory-abi.md) for the required host contract.
Every registered import must be provided, including unused ones.

Use `Compiler::compile_source` for the full pipeline, or `Compiler::analyze` and
`Compiler::compile` separately to inspect the typed AST. The typed program owns
its host-import snapshot, so later registry changes do not alter its compilation.

```rust
use was_compiler::{Compiler, ExportOptions};

let mut compiler = Compiler::new();
compiler.set_exports(ExportOptions::Only(vec!["main".into()]));
```

Library output is raw Wasm. CLI binary output additionally contains versioned
`was-compiler.interface` metadata for the typed Node runner. Plain WAT has no
packaging metadata. The runner accepts native JavaScript strings/arrays by copying
across the descriptor ABI; wrapped host-array mutations do not mutate guest data.
Use direct embedding and the memory ABI when reference mutation is required.

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
cargo test --offline -- --include-ignored
```
