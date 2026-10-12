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

The runner prints `65`. A source file is required; no arguments produce a usage
error (exit `2`). `--help` exits `0` and lists source-file, output-format, export,
and host options. `-` is a literal source filename, not stdin.
The Node harness uses built-in modules only and requires no npm installation.

- [Runtime integration](docs/runtime-integration.md): CLI usage, host providers,
  export selection, raw Wasm calls, explicit marshaling, and diagnostic stages.
- [Limitations and alternatives](docs/limitations.md): current restrictions,
  available workarounds, and possible future solutions with tradeoffs.


## Compile in the browser

The dedicated `browser/` workspace crate (`was-compiler-browser`) builds the
compiler library as WebAssembly. Compile and execute source locally in the
dependency-free [browser playground](examples/browser/index.html):

```text
rustup target add wasm32-unknown-unknown
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --offline --no-opt
```

Run the command from the project root; `../pkg` is relative to the `browser/`
crate, and `--out-name was_compiler` preserves the demo's generated module paths.
There is no root `browser` feature; the native root crate remains the default
workspace member. Install `wasm-pack` if needed (`cargo install wasm-pack --locked`).
The build uses release mode by default; offline builds require cached dependencies
and available tooling. Serve the project root over HTTP (for example,
`python -m http.server 8000`) and open
`http://localhost:8000/examples/browser/` rather than a `file:` URL.

The demo compiles and runs in a terminable worker, displays diagnostics and plain
WAT, and downloads raw Wasm. See the [browser guide](docs/browser.md) for the
JavaScript `Compiler` API, host imports, configuration, memory boundaries, and
MIME/CSP requirements. Browser use does not depend on `runtime/*.cjs` and does not
support `cwasm` artifacts.

## Current compilation support

The compiler emits valid modules containing registered function imports,
parameters and locals of every nonvoid language type, variable reads and
assignments (including numeric compound updates), expression statements, returns,
user/host function calls, `if`/`else`, and `while` / range-only `for` with
`break;` / `continue;`. Strings, numeric arrays, checked array access/assignment, and built-in
`len` are supported. Empty `void` functions and bare `return;` statements work too.
Calls can be forward, nested, self-recursive, or mutually recursive; arguments
are evaluated left-to-right.

Local declarations support both `let name = initializer;` and the existing
`let name: type = initializer;`. Without an annotation, the initializer determines
the local's concrete type: `number`, `bool`, `string`, `number[]`, or `range`. This works for
all value-producing expressions, including calls, blocks, and `if`; `[]` infers
`number[]` because numeric arrays are the only array type. Explicit annotations
still require an exact match.

```text
fn main() -> number {
    let values = [10, 20];
    let index = 1;
    let result: number = values[index] * 2;

    return result;
}
```

The initializer is analyzed before the new binding is introduced. In a nested
scope, `let value = value + 1;` can read an outer `value` and then shadow it; without
an outer binding, it is an unknown-variable error. The local's type is fixed after
initialization, so later assignments must match it. A void-returning call cannot
initialize a local. Every declaration still needs an initializer, and parameter
and function result types remain mandatory. There is no function inference,
whole-program type unification, promotion, or implicit coercion. `number` remains
IEEE-754 `f64`, even for `let index = 1;`; there is no source `i32` type.

`//` line comments run to LF, CR, or end of file. `/* ... */` block comments can
span lines and end at the first `*/`; they do not nest. Both are skipped between
tokens anywhere whitespace can appear. Comment delimiters inside string literals
remain text. Comments separate tokens rather than joining identifier fragments or
operator bytes: `foo/* note */bar` is two identifiers, not `foobar`.

```text
fn main() -> number {
    // A line comment
    return 6 /* A block comment
                spanning lines */ * 7;
}
```

The lexer skips comments directly, preserving original UTF-8 byte spans without
source preprocessing. An unterminated block comment produces an `Invalid` token
from the opening slash to EOF, followed by `Eof`, and is reported through existing
parser errors; it does not introduce a new error stage.

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

Numeric variables and numeric-array elements support statement-only `+=`, `-=`,
`*=`, `/=`, and `%=`. The target is evaluated and bounds-checked once, its old value
is read before the RHS, and the result is written to the original location. RHS
array mutations do not change that saved old value; target failures trap before
RHS effects. Both operands must be `number`; strings, booleans, and whole arrays
cannot be compound-assigned. `%=` uses the same exact remainder semantics as `%`.

```text
fn main() -> number {
    let values = [10, 20];
    values[0] += 2;
    let total = values[0];
    total *= 3;
    total %= 10;

    return total;
}
```

It supports `+`, `-`, `*`, `/`, `%`, unary negation, and nested/parenthesized expressions.
Resolved variable IDs map to per-function Wasm slots, with parameters first and
locals in declaration order. Reads use `local.get`; initialization and assignment
use `local.set`. Scalar expression statements end in `drop` to keep the stack
balanced. Boolean literals use `i32.const 0` or `i32.const 1`.

Numeric literals become `f64.const`; arithmetic uses `f64.add`, `f64.sub`,
`f64.mul`, `f64.div`, and `f64.neg`. Integers and decimals use the same `f64`
representation. Invalid and overflowing literals return explicit errors;
arithmetic follows IEEE-754 behavior, including infinity, NaN, and signed zero.

`number % number` produces truncating floating remainder, matching JavaScript
`%`/`fmod`, not mathematical modulo or nearest-quotient IEEE remainder. Operands
are evaluated exactly once, left-to-right. A NaN operand, infinite dividend, or
zero divisor produces NaN; a finite dividend with an infinite divisor is returned
unchanged. Otherwise the result has the dividend's sign, including signed zero,
and its magnitude is less than the finite nonzero divisor's magnitude.

Wasm has no `f64.rem`, so `%` calls an optional guest `(f64, f64) -> f64` helper.
It uses exact binary scaled subtraction rather than `a - trunc(a / b) * b`,
requires no implicit host import or Memory/Global/Data sections, and is appended
after all user functions and any seven memory helpers. It is included only when
`%` or `%=` occurs anywhere in the typed AST, including unexported functions,
unreachable statements, and runtime-skipped code. Programs without either retain the same
output. The browser compiler uses the same pipeline and remainder semantics.

Numeric comparisons support `==`, `!=`, `<`, `<=`, `>`, and `>=`; booleans support
`==`, `!=`, `!`, `&&`, and `||`. Logical operators short-circuit, so skipped operands
never execute host calls or other side effects. Boolean operations treat any
nonzero `i32` as true and produce canonical `0`/`1` results; numeric comparisons
follow IEEE-754 behavior, including NaN and signed zero.

`break;` exits the nearest active `while`, `for`, or statement loop; `continue;`
skips the rest of the body and starts the next iteration (reevaluating a while
condition or advancing a for). Transfers work beneath nested branches and expression blocks, but
cannot cross function boundaries. An active loop is required even in unreachable
source. `break`, `continue`, and `loop` are reserved source/host-language names;
rename existing bindings/functions using them. Host import module/field names
retain their unrestricted nonempty contract. `for` is also reserved for range
iteration; labels and array iteration remain future work.

```text
fn main() -> number {
    let i = 0;
    let total = 0;

    while i < 10 {
        i += 1;
        if i == 3 { continue; }
        if i == 7 { break; }
        total += i;
    }

    return total;
}
```

This returns `18`. Transfers skip later effects and runtime traps, but skipped
statements still receive static and backend checks. They do not count as function
returns. The typed API adds per-function `LoopId`s to `While` and resolved targets
to `Break` / `Continue`; public statement matches/construction must account for
these additions. IDs may be sparse, but must be unique within a function, and
transfers must refer to its innermost active loop.

### Statement and value loops

`loop` repeats until a break exits it. In statement position, `loop { ... }`
(optionally followed by `;`) accepts only bare `break;`. In value position, its
result comes from `break expression;`, not a body tail:

```text
fn first_positive(values: number[]) -> number {
    let index = 0;
    let result = loop {
        if index >= len(values) { break 0; }
        let value = values[index];
        index += 1;
        if value <= 0 { continue; }
        break value;
    };

    return result;
}
```

Value loops support numbers, bools, strings, and numeric-array handles. Reachable
breaks targeting that loop must agree exactly; nested loops do not affect its
result. Unreachable breaks do not constrain the result but still receive type,
name, and context checks. A bare break in a value loop, a value break in a
statement loop/while/for, or a Void break value is a semantic error. A divergent break
RHS supplies no value and does not reach its break.

Loop bodies contain strict statements, not implicit tails. Bare loops in function
or loop bodies are statements, even with `;`; use parentheses to discard a value
loop, as in `(loop { break 1; });`. In an expression block, a final bare loop
without `;` is its value tail; a nonfinal or semicolon-terminated bare loop is a
statement. Bare statement loops end at the closing brace, regardless of trivia.
Parenthesize a whole loop for arithmetic continuation in these positions.

A loop with no reachable exit is `Never`. `let x = loop {};` cannot infer a storage
type; `let x: number = loop {};` is accepted, without a reachable store. Explicit
noncompletion is valid in a nonvoid function: `fn forever() -> number { loop {} }`
compiles, although it never returns. Later unreachable code is still checked.
There is no compiler-provided fuel or cancellation; avoid calling an unbounded
loop unless the embedding supplies its own resource controls.

**AST additions (WP4):** raw/typed expressions and statements add `Loop { body }`
(with `id: LoopId` in typed nodes). Existing bare `Break` variants are retained;
raw statements add `BreakValue(Expr)` and typed statements add
`BreakValue { target, value }`. Update exhaustive matches to handle these additions.
Loop IDs are unique per function across while, for, statement, and value loops.

### Block values and `if` expressions

An expression block's final expression supplies its value; a trailing semicolon
discards it. Value-context `if` requires `else` (including `else if` chains), and
normally completing branches must have exactly the same type. Numbers, booleans,
strings, and numeric-array handles are supported without changing their ABI.

```text
fn choose(ready: bool) -> number {
    let result = if ready {
        let base = 20;
        base * 2
    } else {
        return 0;
    };

    return { let extra = 2; result + extra };
}
```

Block-local bindings do not escape. Both branches and unreachable statements are
still checked, although only the selected branch executes. Two normally completing
branches without values produce `void`, which can be discarded but cannot initialize
a local. A path that unconditionally returns, breaks, or continues is internally
`Never`: it has no normal result, storage, or host ABI type. It can join another
branch's concrete result. `let x = { return 1; };` cannot infer a storage type;
`let x: number = { return 1; };` is accepted without storing a nonexistent value.
A possible runtime trap alone does not infer `Never`.

Named functions still require explicit `return` for results;
`fn f() -> number { 42 }` is not an implicit return. Function and while bodies require semicolons
for ordinary expression statements. A standalone block expression also requires
`;`. Bare statement-if may omit `else` and discards branch tails. Within an
expression block, a final bare `if` is a tail, while a nonfinal bare `if` is a
statement. A bare statement-if ends at its final arm: `if true {} -1;` is two
statements regardless of whitespace. Parenthesize the whole if to continue it
arithmetically in these positions: `(if true { 2 } else { 3 }) - 1;`.

Branches and loops use structured Wasm control flow and preserve shadowed bindings.
Transfers also work through expression blocks, including in their own while's
condition. Continue reevaluates that condition. Nonvoid functions must return or
explicitly diverge on every path; `while true { return 1; }` alone remains
conservative and is not a guaranteed return. An
unconditional explicit return in the condition, as in
`while { return 1; } {}`, does guarantee return before the body. Skipped operands,
unreachable statements, and forged scope escapes still receive backend checks.

**Typed-AST migration (WP3):** `TypedExpr.ty` is now `ExprResult::Value(Type)`,
`ExprResult::Void`, or `ExprResult::Never`, rather than `Type`. Use `ty.into()`
for existing expression constructors (`Type::Void` converts to `Void`), and
`value_type()` to extract a concrete value type. `Never` is not a source `Type`;
`Value(Type::Void)` is malformed. Bindings and function signatures retain `Type`.
Handle the new raw/typed `Block` and `If` expression variants. `ValueBlock` and
`TypedValueBlock` hold statement vectors, an optional boxed tail, and a span;
existing function/statement-block vector aliases are unchanged.

Strings support literals, parameters/locals/returns, and content-based `==`/`!=`.
Numeric arrays support literals (including `[]`), reference assignment, parameters,
returns, element reads/writes, content-based `==`/`!=`, and `len(array)` returning
a number. Equality requires matching lengths and exact elementwise `f64` equality:
signed zeros compare equal, while NaN makes equality false even for an array
compared to itself. Array assignment still aliases storage. Array indices
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
imports. Required memory ABI exports remain. M0–M9 are implemented; no optimization
passes were added.

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

Library and CLI binary output are the same raw Wasm bytes for the same source,
host declarations, and compiler/export options. The Node runtime's
`instantiate(bytes, imports)` returns `{ instance, call }`; `call` passes raw
argument values and returns raw results. Numbers use `f64`; booleans and
string/array descriptor handles use `i32`. Native strings/arrays are not marshaled
automatically. Use `createAbi` from `runtime/abi.cjs` for explicit conversion, or
access memory directly when reference mutation is required.

### Range construction, internal calls, and numeric membership (partial WP5)

`range` values support `start..end` and `start..=end`, with an optional contextual
`by step`; the default step is `+1`. Range operators bind below arithmetic and
above comparisons and are non-associative. Endpoints and steps must be `number`.

```text
fn identity(value: range) -> range { return value; }

fn main() -> number {
    let value: range = identity(10..=0 by -2);
    return 7;
}
```

Compile this example with `--export main` or
`ExportOptions::Only(vec!["main".into()])`. Range parameters/results use private
`i32` handles internally; every range host-import signature and every selected
range user-export signature is rejected. Default export-all therefore rejects
`identity`; `Only` or `None` permits internal range functions without pruning or
skipping checks. There is no public range marshaling API.

Construction evaluates start, end, and explicit step once, left-to-right, **all
before validation**, even when an earlier operand is invalid or the range is empty
or discarded. Executed construction traps unless all inputs are finite integers
within `-(2^53 - 1)..=(2^53 - 1)` and the step is nonzero (both signed zeros are
invalid). Negative-zero endpoints become integer zero. Each construction allocates
an immutable private record; assignment and internal calls share its handle.
See the [private layout and validation contract](docs/memory-abi.md#private-range-record).

Range locals, assignments, parameters/results, blocks, `if` values, and value loops
use exact type joins with `Never` compatibility. Range `==`/`!=` are prohibited.

`number in range` returns `bool`: `6 in (0..10 by 2)` is true, while
`5 in (0..10 by 2)` is false. `in` is a reserved source/host-language name, not a
restriction on host module/field names. It binds at comparison precedence with
existing left associativity. Both operands evaluate once, left-to-right, with
exact `number`/`range` types and Never compatibility; all children remain checked,
including unreachable or runtime-skipped code. Membership validates the consumed
record before checking the candidate, so malformed records trap even for invalid
candidates. Nonfinite, fractional, or out-of-safe-domain candidates return false;
both signed zeros are accepted. Interval and congruence checks use exact signed
`i64` arithmetic, not floating remainder.

Range-only statement `for` is implemented:

```text
fn main() -> number {
    let total = 0;
    for value in 0..=6 by 2 {
        if value == 2 { continue; }
        total += value;
    }
    return total;
}
```

This yields `0`, `2`, `4`, `6` and returns `10`. `for name in range_expr { body }`
accepts only a `range`, not an array. The iterable evaluates once in the enclosing
context **before** activating the new loop or introducing its read-only `number`
binding. Break/continue inside that expression targets the outer nearest loop,
or is invalid without one; return/divergence is preserved. The body may introduce
an inner shadowing `let` that is mutable; assigning or compound-updating the
iteration binding itself is rejected.

Bare `break;` exits the for; `continue;` advances once before retesting; `return`
exits the function. Value breaks are rejected even when unreachable. For is a void
statement with a strict statement body, not a value-producing expression. An empty
range can skip the body, so a returning body alone does not guarantee a function
return. Cached validated fields use exact `i64` advancement and signed endpoint
checks **before** `f64` conversion/yield; yielded integers are exact and zero is
`+0`. Advancing past the endpoint, including outside the safe-integer domain,
exhausts normally without a trap or out-of-range yield. The bounds proof is in the
[memory ABI](docs/memory-abi.md#private-range-record).

**AST addition (issue #4):** typed statements add
`For { id, variable, name, iterable: Box<TypedExpr>, body }`; public consumers must
update constructors and exhaustive matches. Read-only checks derive from For
provenance, without new public mutable-binding metadata.

This remains **partial WP5**: array iteration and broader WP5 adapter/interaction
acceptance remain proposed. The CLI and browser already support range-only for
through the shared core pipeline. `tests/for_frontend.rs` covers syntax, types,
scopes, read-only bindings, and transfers; `tests/for_lowering.rs` covers forged
input, scratch layout, cached fields, and decoded conversion/latch ordering.
`tests/for_loops.rs` covers deterministic output, import snapshots/export selection,
and bounded execution against an independent BigInt progression reference at all
optimization levels. Node execution cases retain the ignored-test policy: default
Cargo success alone does not establish runtime coverage.
No optimization passes were added. For adds no range helpers.
The optional `Contains` helper follows `Construct` and `Validate` only when membership exists;
construction-only programs retain their bytes, as do programs without ranges.
Membership alone needs no floating remainder helper or host callback.

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

Run `cargo run --offline -- examples/array-sum.was` to print plain WAT with
`wasmprinter`.
Run tests with `cargo test --offline`. The full suite includes optional Node.js
runtime tests and compiler-in-Wasm tests; build `pkg/` with the browser command
above and use Node.js 24+ before including ignored tests:

```text
cargo test --offline -- --include-ignored
```
