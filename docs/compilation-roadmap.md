# WebAssembly compilation roadmap

## Goal

Build a small backend that consumes the typed AST without repeating parsing,
name resolution, or type checking. Keep host integration, ABI decisions, module
layout, and instruction emission separate so each milestone extends one layer.

The first deliverables after the foundation are **number literals from the typed
AST**, then **arithmetic operations**. This roadmap does not introduce a second IR,
optimizer, plugin system, or execution engine before those are needed.

## Pipeline and ownership

```text
Source
  → Lexer
  → Parser AST
  → Semantic analysis + registered host signatures
  → TypedProgram
  → Optimization stage (currently a no-op at every level)
  → ABI and module layout
  → Function-body emission
  → wasm_encoder sections
  → WebAssembly bytes
```

`TypedProgram` owns a snapshot of its host declarations. Its function references
are semantic IDs, not raw Wasm indices. This lets an analyzed program be compiled
later, even after the compiler's host registry changes or the analyzer is dropped.

The compiler emits bytes. The embedding application decides whether to write a
`.wasm` file, instantiate it in a browser, or pass it to another runtime.

## Structure

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | Public Rust library entry point; native CLI and browser adapter use the same pipeline. |
| `browser/src/lib.rs` | Dedicated compiler-in-Wasm adapter exposing source compilation and configuration through `wasm-bindgen`. |
| `examples/browser/` | Browser editor, Worker-based compilation/execution, WAT display, and raw Wasm downloads. |
| `tests/browser.rs`, `tests/browser.cjs` | Execute the built browser compiler package and instantiate its emitted modules. |
| `src/compiler.rs` | Compiler options, host/export configuration, and pipeline orchestration; stage-specific errors. |
| `src/exports.rs` | User-export policy (`All`, `None`, or explicit source names), separate from optimization presets. |
| `src/cli.rs` | Parse arguments, load a required source file, register hosts, and publish plain WAT/raw binary Wasm. |
| `src/diagnostic.rs` | Shared library renderer for CLI/browser source locations/excerpts and stage errors without changing core error types. |
| `runtime/index.cjs`, `runtime/abi.cjs`, `runtime/run.cjs` | Instantiate raw binaries, offer explicit ABI conversions, and expose a raw-call Node runner. |
| `tests/cli.rs`, `tests/runtime.rs` | Exercise source-file/raw-binary/provider integration separately from backend tests. |
| `src/optimization.rs` | O0–O3 level configuration and a typed-AST optimization boundary; no passes implemented yet. |
| `src/host.rs` | Ordered host declarations, language-visible names, import names, signatures, and registration validation. |
| `src/semantic.rs` | Resolve and type-check user, host, and built-in calls; produce the typed AST. |
| `src/typed_ast.rs` | Typed expressions, resolved references, and owned import metadata. |
| `src/codegen/abi.rs` | Central language-type → Wasm-type mapping and function signatures. |
| `src/codegen/layout.rs` | Deduplicate signatures, assign indices, and build the complete typed callee table before body emission. |
| `src/codegen/call.rs` | Validate resolved call references/signatures, emit user/host calls and built-in `len`, normalize host booleans, and validate memory handles. |
| `src/codegen/function.rs` | Per-function emission context, scoped initialized-binding checks, return-path checks, and function termination. |
| `src/codegen/control_flow.rs` | Emit structured branches and loops, preserve scope boundaries, and track guaranteed returns. |
| `src/codegen/locals.rs` | Map typed variable IDs to dense Wasm slots and declare value/handle local types. |
| `src/codegen/temporaries.rs` | Preallocate independent array-construction scratch slots, including nested and unreachable expressions. |
| `src/codegen/mod.rs` | Assemble Type, Import, Function, optional Memory/Global, Export, Code, and optional Data sections in order. |
| `src/codegen/tests.rs` | Inspect and validate emitted modules with `wasmparser`. |
| `src/codegen/expression.rs` | Emit scalar literals, variable reads, arithmetic/comparisons, short-circuit boolean operators, and user/host calls. |
| `src/codegen/statement.rs` | Emit blocks, scalar declarations/assignments, expression cleanup, returns, and delegate control flow. |
| `src/codegen/values.rs` | Emit fresh array construction, checked element addressing, and variable/element assignments. |
| `src/codegen/memory.rs` | Plan static strings and emit allocation, descriptor validation, checked array addressing/length, and string/numeric-array equality helpers. |
| `src/codegen/numeric.rs` | Detect remainder throughout the typed AST and emit the optional memory-independent `(f64, f64) -> f64` guest remainder helper. |
| `tests/remainder.rs` | Validate remainder lowering, optional helper layout, result types/spans, exact binary64 edge cases, and operand effects against an independent reference. |
| `tests/array_equality.rs` | Validate array comparison lowering and execute IEEE-754, aliasing, evaluation-order, and descriptor-safety cases. |
| `src/codegen/memory_tests.rs` | Validate memory ABI/layout and execute memory-backed guest/host scenarios. |

These are concrete modules with narrow responsibilities, not a generic plugin or
pass framework. Add modules when a milestone requires them.

`CompilerOptions` selects `OptimizationLevel::None`, `Basic`, `Standard`, or
`Aggressive`; the default is `None`. These are intended O0–O3 presets, not
implemented transformations. All levels currently preserve the same Wasm and
errors. `Compiler::compile` and `compile_source` invoke the optimization boundary,
while semantic analysis is independent of it. The boundary uses `Cow<TypedProgram>`
to borrow the input today and allow owned transformations later without mutating
the caller's AST. Future passes must preserve resolved IDs and the host ABI unless
they explicitly update the corresponding metadata and references.

## Initial ABI and index conventions

| Language type | Wasm representation |
| --- | --- |
| `number` | `f64`, including integer-looking and decimal literals. |
| `bool` | `i32`; the host contract uses `0` for false and `1` for true. |
| `void` | No result value; never a parameter or local value type. |
| `string` | `i32` handle to an immutable UTF-8 descriptor in the module's linear memory. |
| `number[]` | `i32` handle to a mutable numeric-array descriptor in the module's linear memory. |

- `HostFunctionId` follows registration order. Host functions occupy the first
  Wasm function indices.
- `FunctionId` remains the source-order index in `TypedProgram.functions`.
  Its Wasm function index is `host_function_count + FunctionId.0`.
- Memory-using modules append seven runtime helpers after all user functions.
  Existing host/user IDs and index offsets are unchanged. The allocator is exported
  as `__alloc`; linear memory is exported as `memory`. User functions with these names
  cannot be selected as exports in memory-backed modules; unexported names are legal.
- If `%` occurs anywhere in the typed AST, append one internal `(f64, f64) -> f64`
  remainder helper after all user functions and, when present, the seven memory
  helpers. Scan unexported bodies, unreachable statements, and runtime-skipped
  operands too; export selection is not dead-code elimination. This helper adds
  no implicit host import and requires no Memory, Global, or Data sections.
  Existing host/user/memory-helper indices remain unchanged, and programs without
  `%` retain byte-for-byte output.
- Type indices are separate from function indices. Identical signatures share
  one Type-section entry, including signatures shared by hosts and users.
- `VariableId` is function-local. The local layout places parameters first, then
  declarations in traversal order, including nested blocks. Each resolved ID maps
  explicitly to one dense Wasm slot; names and sparse IDs do not affect the slot.
  Shadowed bindings get distinct slots, and IDs are never looked up across functions.
  Initializers are emitted before a local is marked initialized; malformed typed
  ASTs cannot read or assign a later local just because Wasm initializes slots to zero.
  Array-construction scratch slots follow declared locals and do not alter any
  resolved variable index.
- User functions are exported under their language names by default. M7 adds
  `ExportOptions::All`, `None`, and `Only(Vec<String>)` / CLI selection. Export entries
  retain source order; selection never removes function bodies or registered imports.
  Required `memory`/`__alloc` exports remain regardless of the user-export policy.
  Hosts are imports, not exports.
- `len` remains a compiler built-in. It is not implicitly registered as a host
  function; M6 lowers its array-only signature to the checked runtime length helper.

The numeric model is IEEE-754 binary64. Literal conversion checks the language's
integer/decimal grammar, uses Rust's `f64` conversion, and rejects overflow to
infinity. It never replaces an invalid literal with zero. Finite conversion uses
binary64 rounding; sufficiently small decimals can underflow to zero.
Arithmetic can produce
IEEE-754 infinity or NaN; this does not require an integer overflow policy.

## Host functions

Host registration declares an import; it does **not** install or execute a Rust
closure. The runtime must provide the matching module/field when instantiating
the resulting module.

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

let typed = compiler.analyze("fn main() -> void { print(1); }")?;
```

Here `print` is the language name and `env.log_number` is the runtime import.
Semantic analysis validates the argument and resolves the call to a
`FunctionRef::Host` ID. M4 emits the call using the precomputed import index, with
no name lookup or ad-hoc registration during function emission.

Host `number` values retain the `f64` representation, including NaN and infinity.
Boolean arguments passed to hosts and boolean results received from hosts use a
double `i32.eqz` to normalize them to `0` or `1`; a nonzero incoming `i32` is true.
Hosts must obey the registered signatures, and the Wasm runtime applies its usual
scalar signature conversions. User calls preserve scalar values without inserting
host-boundary normalization.

Unknown references, argument-count mismatches, forged call result/argument types,
and malformed built-in calls return span-aware errors before a module is returned.

Registration accepts numeric, boolean, string, and numeric-array parameters/results,
with an optional `void` result. Void parameters, invalid identifiers, reserved names,
duplicate language names, and empty import names return explicit errors.
Memory-backed values use the [documented descriptor ABI](memory-abi.md), not runtime
native strings or arrays. Host inputs/results are checked for descriptor bounds;
hosts must preserve descriptor metadata, UTF-8 validity, and string immutability.
Source functions cannot redeclare a registered host name. Local variables follow
the existing shadowing rules and are not callable.

All registered functions are included as imports, even if unused. Consequently,
the runtime must supply every registered import. Reachability-based import pruning
is a later optimization and must not be mixed into the first index-layout work.

A foundation example that already compiles:

```rust
let wasm = compiler.compile_source("fn main() -> void {}")?;
std::fs::write("module.wasm", wasm)?;
```

The resulting module can be instantiated in JavaScript with:

```javascript
const bytes = await (await fetch("module.wasm")).arrayBuffer();
const { instance } = await WebAssembly.instantiate(bytes, {
  env: { log_number: value => console.log(value) }
});

instance.exports.main();
```

This foundation `main` does not call the host yet. Registration is intentionally
shared by semantic analysis and emission, rather than maintained in unrelated
symbol tables.

## Milestones and acceptance criteria

### M0 — Foundation and host declarations: implemented

- Expose one reusable library pipeline and a thin sample binary.
- Type-check registered host functions and retain their declarations in the typed
  AST, without changing user function IDs.
- Build deterministic, import-first module layout and deduplicated signatures.
- Emit valid empty modules, import-only modules, and empty `void` user functions.
- Validate emitted bytes with `wasmparser`; verify signatures, imports, exports,
  function-index offsets, and deterministic output.
- Reject unsupported features and unsupported ABI types explicitly. Do not emit
  `unreachable` placeholders or silently discard a body.

### M1 — Number literals from the typed AST: implemented

- Add expression emission for `TypedExprKind::Number` with `Type::Number`.
- Convert the literal to `f64` and emit `Instruction::F64Const`.
- Add value-returning `return` emission and close each function correctly.
- Keep unsupported expression/statement diagnostics explicit and span-aware.
- Acceptance: compile, validate, instantiate, and execute
  `fn main() -> number { return 42; }` and a decimal-returning function.
- Include invalid-literal and overflow tests against manually constructed typed
  ASTs, in addition to source-to-Wasm tests.
- Preserve unsupported-feature errors after a return rather than silently
  ignoring unreachable statements.
- Run optional Node.js execution tests with
  `cargo test --offline codegen::tests::node_executes -- --include-ignored`.

### M2 — Numeric arithmetic: implemented; remainder checkpoint complete

- Emit operands left-to-right, followed by `F64Add`, `F64Sub`, `F64Mul`, or
  `F64Div`; emit unary negation with `F64Neg`.
- Trust the parser's expression tree for precedence; do not implement a second
  precedence parser in the backend.
- Acceptance: execute nested expressions such as `1 + 2 * 3`, `(1 + 2) * 3`,
  `-(1 + 2)`, and decimal division; check operand order and floating-point edge
  cases as well as bytecode validation.
- Instruction-sequence tests verify nesting, associativity, and operand order.
  Node.js runtime tests verify decimal rounding, NaN, positive/negative infinity,
  signed zero, overflow from finite operands, and import-index offsets.
- Every optimization level still emits the same expression instructions without
  constant folding. Malformed typed-AST operands and unsupported operators retain
  enclosing statement spans in their errors.
- **Separate remainder checkpoint (M9): complete.** Wasm has no `f64.rem`;
  `%` now lowers to an optional guest helper implementing truncating floating
  remainder. It preserves operand order, special values, and signed zero without
  integer remainder or a naive quotient-based rewrite. See M9 for the algorithm
  and acceptance criteria.

### M3 — Variables, parameters, assignments, and stack discipline: implemented

- Build a per-function local layout from parameter and declaration IDs.
- Emit scalar `let`, variable reads, and assignments with `local.get`/`local.set`.
- Add expression-statement cleanup: drop scalar results, but not `void` results.
  M4 additionally verifies cleanup for real numeric, boolean, and void calls.
- Boolean literals use `i32.const` so boolean declarations, assignments, reads,
  expression statements, and returns work without introducing boolean operators.
- Acceptance: execute parameterized arithmetic, numeric/boolean mutations, mixed
  local types, and discarded scalar results at all optimization levels.
- Layout tests verify sparse IDs, nested shadow bindings, then/else/while preorder,
  and per-function ID isolation. M5 adds runtime nested-shadowing coverage for
  branches and loops.
- Reject duplicate/unknown IDs, uninitialized reads, type mismatches, unsupported
  local types, and nonscalar assignment targets with descriptive errors.

### M4 — User and host calls: implemented

- Resolve all call indices from the precomputed layout, never by interleaving
  ad-hoc registration with body emission.
- Emit argument expressions in order and then `call` for user and host functions.
- Validate numeric/boolean values at the host ABI boundary as required.
- Acceptance: execute forward/nested calls, host numeric/boolean results, and host
  `void` side effects; verify left-to-right argument effects, missing/noncallable
  imports, shifted user indices, and host snapshots at all optimization levels.
- Validate self/mutual call links and execute recursion bounded by a host callback
  that throws a sentinel. M5 adds guest-only terminating recursion with `if` base
  cases; unbounded recursion is never executed in the tests.
- Verify scalar call statements receive `drop`, void call statements do not, and
  malformed typed calls after a return still report errors.
- Keep built-in `len` separate from user/host references; M6 adds its runtime lowering.

### M5 — Boolean expressions and control flow: implemented

- Emit numeric `==`, `!=`, `<`, `<=`, `>`, `>=`, boolean equality/inequality,
  and unary `!`. String equality remains part of the memory-backed milestone.
- Preserve short-circuit behavior for `&&` and `||` using `if (result i32)`,
  not eager bitwise operations. Boolean operations treat any nonzero `i32` as
  true and produce canonical `0`/`1` results.
- Emit `if`/`else`, `while`, and nested returns while maintaining branch stack
  types and labels. Loops use an outer empty `block`, an inner empty `loop`,
  `br_if 1` for exit, and `br 0` for the back edge.
- Restore initialized-binding state after each scoped branch/loop body. Outer
  assignments persist at runtime; branch-local bindings cannot leak into sibling
  branches or later statements in manually constructed typed ASTs.
- A conditional guarantees return only when both branches do. An empty-result
  `if` whose arms both return is followed by `unreachable` to express its impossible
  continuation to the Wasm validator, never as a placeholder for unsupported code.
  Loops remain conservatively fallthrough-capable, matching semantic analysis.
- Validate unreachable statements and runtime-skipped operands rather than
  silently discarding errors. Nonvoid functions still require a return on every path.
- Acceptance: validate decoded instructions and execute branching, nested loops,
  early returns, shadowing, repeated host conditions, and short-circuit host-side
  effects at all optimization levels. Include IEEE-754 comparisons, noncanonical
  boolean inputs, factorial, and mutually recursive guest-only base cases.

### M6 — Memory-backed values and built-ins: implemented

- Use one aligned `i32` handle to a `{ data_pointer: u32, length: u32 }` descriptor
  per string/array. Strings are immutable UTF-8 byte values; arrays are mutable
  `f64` sequences with reference/alias semantics. See [memory ABI](memory-abi.md).
- Add Memory/Global/Data sections only when needed by signatures, declarations,
  or expression kinds/types. Preserve byte-for-byte scalar-only output.
- Intern static strings deterministically, emit content-based string equality,
  and allocate a fresh array for every literal evaluation, including inside loops.
- Use an eight-byte-aligned bump allocator that checks widened arithmetic,
  grows memory, and traps on overflow/exhaustion. There is no free or GC yet;
  allocations persist until the instance is discarded.
- Check descriptors and finite, integral, nonnegative array indices before
  computing addresses. Reads and indexed writes trap out of bounds. Evaluate
  base/index and bounds before an indexed assignment's RHS. `len` accepts only
  numeric arrays and returns their element count as `f64`.
- Support memory-backed user/host signatures and locals. Validate memory parameters
  at user-function entry and host arguments/results at host boundaries. Export
  `memory` and `__alloc` for host construction of memory-backed return values.
- Acceptance: execute array sums, mutation, aliasing, fresh loop literals, nested
  calls/recursion, string UTF-8/empty/NUL equality, host roundtrips, reentrant host
  allocation, memory growth, invalid descriptors/indices, and bounded allocator
  exhaustion. Validate emitted modules at every optimization level.

### M7 — CLI and runtime integration: implemented

- Add a thin CLI with one required UTF-8 source-file path, raw binary or plain-WAT
  output, atomic file publication, explicit host declarations, and export selection.
  No arguments are a usage error (exit `2`); help exits `0`. There is no built-in
  example or stdin input; `-` is a literal source filename.
- Expose additive user-export policy APIs without changing copyable optimization
  options or removing/renumbering any body, import, signature, or runtime helper.
- Keep CLI and library binary output identical for matching source/options, with
  no interface or package metadata.
- Provide a dependency-free Node library/runner with raw Wasm calls and CommonJS
  host providers. `instantiate(bytes, imports)` returns `{ instance, call }`;
  numbers use `f64`, booleans and descriptor handles use `i32`. All imports must
  be supplied. Strings/arrays are not automatically marshaled; existing `createAbi`
  helpers support explicit conversion and raw host return handles.
- Render parse, semantic, and located code-generation errors with path, line/column,
  excerpt, and caret. Report file, binary compilation, instantiation, and invocation
  failures in the runner with actionable context.
- Acceptance: validate required files, literal `-`, help/usage statuses, output
  safety, raw byte equality, export indices, provider failures, raw arguments and
  results, explicit conversions, and guest traps through end-to-end CLI/Node tests.
  See [runtime integration](runtime-integration.md).
- Keep optimization and import elimination separate from export visibility and
  correct lowering. No optimization transformations were added.

### M8 — Numeric-array content equality: implemented

- Allow `number[] == number[]` and `number[] != number[]`, producing a `bool`.
  Compare lengths first, then corresponding elements in order. Equality means
  equal contents, not identical descriptor handles; two empty arrays are equal.
- Use exact IEEE-754 numeric equality for elements: `+0` equals `-0`, matching
  infinities compare equal, and any compared NaN makes equality false. Even an
  array compared to itself is unequal if it contains NaN. `!=` is the complement
  of `==`; it is not a separate comparison policy.
- Evaluate each operand exactly once, left-to-right, before comparing current
  contents. A right-hand host call may mutate the left array; comparisons observe
  that mutation, not an earlier snapshot. Preserve array reference assignment and
  mutable alias semantics; comparison does not allocate, copy, or mutate arrays.
- Append a checked `(i32, i32) -> i32` array-equality helper after the existing
  memory helpers. Validate both full descriptors before length-based or element
  early exits, including same-handle and empty comparisons. Read elements using
  aligned `f64.load` and compare with `f64.ne`; return canonical `0` or `1`.
- Preserve the descriptor ABI, existing host/user/helper indices, and byte-for-byte
  scalar-only output. Reject mixed operand types and array ordering/arithmetic;
  continue validating malformed typed ASTs, unreachable code, and skipped operands.
- Acceptance: compile literal examples such as `[1, 2, 3] == [1, 2, 3]` (true),
  `[1, 2, 3] == [1, 2]` (false), and their `!=` counterparts. Validate modules,
  decoded helper calls/instructions, and typed results/spans. Execute empty arrays,
  prefixes, differing elements/order, aliases and mutation, NaN/infinities/signed
  zero, nested host effects, memory growth, and malformed descriptor traps at
  every optimization level, including the compiler-in-Wasm browser API.
- Equality takes O(n) time for matching lengths in the worst case and O(1)
  auxiliary space. Approximate/tolerance equality and generalized array element
  types are separate language policies, not part of this milestone.

### M9 — Floating-point remainder (`%`): implemented

- Lower `number % number` to a `number` using truncating floating remainder,
  matching JavaScript `%`/`fmod`, not nearest-quotient IEEE remainder or mathematical
  modulo. Evaluate both operands exactly once, left-to-right.
- A NaN operand, infinite dividend, or zero divisor yields NaN. A finite dividend
  with an infinite divisor yields the dividend. For finite operands and a nonzero
  divisor, the result has magnitude less than the divisor's magnitude and the
  dividend's sign, including signed zero for a zero dividend or exact multiple.
- Wasm has no `f64.rem`. `src/codegen/numeric.rs` emits an optional internal guest
  `(f64, f64) -> f64` helper, appended after all user functions and any existing
  seven memory helpers. Include it only if `%` occurs anywhere in the typed AST,
  including unexported functions, unreachable statements, and runtime-skipped
  operands. Preserve host/user/memory-helper indices and byte-for-byte output
  for programs without `%`. No implicit host import or Memory/Global/Data sections
  are required by remainder.
- After handling special values, use an independent exact binary scaled-subtraction
  algorithm: start with the absolute divisor, double it until the next double
  would exceed the absolute dividend, then descend by halving back to the original
  divisor, conditionally subtracting each scale from the running remainder.
  Doubling and halving are exact relative to the original divisor, including
  subnormals. Whenever subtraction occurs, `scaled <= r < 2 * scaled`, so Sterbenz's
  lemma makes the subtraction exact. Restore the dividend's sign, including zero.
- At most 2,097 doublings and 2,098 reduction iterations are needed for binary64;
  auxiliary space is constant. No quotient is formed, so quotient overflow is
  avoided. Do not use integer casts, integer remainder, or `a - trunc(a / b) * b`:
  overflow and intermediate rounding make that formula incorrect for extreme
  magnitudes, subnormals, and special values.
- Acceptance: `tests/remainder.rs` validates modules, decoded helper calls/layout,
  result types/spans, and absence of implicit imports or memory sections. Execute
  positive and negative operands/divisors, fractions, exact multiples, signed zero,
  NaN, infinities, zero divisors, subnormals, and very large magnitude ratios against
  an independent remainder reference. Preserve nested host effects and once-only
  left-to-right evaluation; validate malformed typed ASTs, unexported bodies,
  unreachable code, and skipped operands at every optimization level.
- The compiler-in-Wasm browser API uses the same compilation pipeline and guest
  helper; exercise its emitted modules as well as native compilation. No
  optimization transformations were added.

## Remaining work

M0–M9 are implemented; this is not a claim that every language/runtime feature is
complete. [Limitations and alternatives](limitations.md) records unsupported
features, ownership/resource policies, raw-call/ABI constraints, and current
workarounds versus possible future designs. [Refactoring](refactoring.md) proposes
incremental, behavior-preserving maintainability work without optimization passes.

## Testing strategy

Every emission milestone should have three layers:

1. Focused lowering/layout tests to catch operand and index mistakes.
2. `wasmparser::Validator` checks plus decoded-section/instruction assertions.
3. Runtime execution tests, initially using JavaScript's WebAssembly API, to
   verify observable values and host side effects.

Validation proves structural and stack correctness, not the intended arithmetic
result. For floating-point results, use exact checks where appropriate and a
stated tolerance for decimal calculations. Errors must not return partially
compiled modules.
