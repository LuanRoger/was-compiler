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
  → Function-body validation/lowering
  → Internal ValidatedProgram (borrowed AST + layout + checked bodies)
  → wasm_encoder section assembly
  → WebAssembly bytes
```

`TypedProgram` owns a snapshot of its host declarations. Its function references
are semantic IDs, not raw Wasm indices. This lets an analyzed program be compiled
later, even after the compiler's host registry changes or the analyzer is dropped.

The public untyped AST represents the optional local annotation as
`StatementKind::Let.ty: Option<Type>` (previously `Type`): `Some(type)` for an
annotated declaration and `None` for an inferred one. Callers constructing or
matching untyped declarations must account for this API change. Semantic analysis
resolves both forms to `TypedStatementKind::Let.ty: Type`; the typed AST retains
concrete types, with no walker, backend, or ABI changes for local inference.

WP3 separately changes `TypedExpr.ty` from `Type` to
`ExprResult::{Value(Type), Void, Never}`. Use `ty.into()` in existing expression
constructors; `Type::Void` maps to `Void`. Bindings/signatures still use `Type`.
Raw/typed `ValueBlock` wrappers add statements, an optional boxed tail, and span;
function and statement-block vector aliases remain unchanged.

Issue #10 adds user-function named arguments as an incremental WP6 slice. Raw
calls carry `CallArgument { name, value, span }`; existing positional constructors
must wrap operands in this record. Named calls use `TypedNormalizedCall` with
written-order owned bindings, formal mapping and a final full-arity direct call.
Checked lowering verifies ownership, scope, mapping and signatures using the
existing emitter, not a second validator. Planning visits bindings then the call;
ordinary positional calls preserve their legacy path. Issue #12 enables trailing
pure defaults: raw `Param.default: Option<DefaultExpression>` owns an expression
and declaration span; existing raw constructors add `default: None`. Private
`semantic/defaults.rs` checks declarations through the existing expression checker,
and `semantic/call_binding.rs` instantiates earlier-formal templates into caller
bindings. Shared numeric-literal rules live in private `number_literal.rs` without
changing ordinary body errors. Default-origin bindings follow explicit bindings,
and checked lowering admits only owned earlier-formal reads and whitelist nodes.
WP6's release gate (#13) passed; see the
[release evidence](was_language_extensions_plan.md#issue-13-wp6-release-validation).
Issue #11 integrates initial result inference with released WP6. Raw
`Function.return_type` is `Option<Type>`; typed signatures remain concrete. Private
`semantic/inference.rs` discovers lexical dependencies and conservative recursive
components; the existing expression checker and private `Flow` compose reachable
return constraints. Issue #14 completes acyclic chain/diamond inference with
source-order ties among ready functions. Full-call-graph recursion checks remain
separate: annotated scheduling barriers cannot hide recursive omitted results.
Owned typed signatures remain concrete; IDs, typed functions and Wasm layout stay
source-ordered. Staging is declaration/default checks in source order, recursive
annotation errors in source order, inference in stable dependency order, then
final body checks in source order. Combined-feature acceptance (#15) and the full
WP7 release gate remain pending. See
[the #14 inference contract](result-inference-development.md#issue-14-acyclic-chains-and-diamonds). See the
[interface gate](named-call-interface.md) and [handoff](wp6-wp7-development.md).

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
| `src/diagnostic.rs` | Shared CLI/browser renderer consuming stage-owned `span()` access through `CompileError`, preserving messages, source excerpts, and located/unlocated presentation. |
| `tests/diagnostics.rs`, `src/cli_contract_tests.rs` | Exhaustive span access/delegation, exact pre-refactor diagnostic text, and CLI format/destination/configuration mappings. |
| `runtime/index.cjs`, `runtime/abi.cjs`, `runtime/run.cjs` | Instantiate raw binaries, offer explicit ABI conversions, and expose a raw-call Node runner. |
| `tests/cli.rs`, `tests/runtime.rs` | Exercise source-file/raw-binary/provider integration separately from backend tests. |
| `src/optimization.rs` | O0–O3 level configuration and a typed-AST optimization boundary; no passes implemented yet. |
| `src/host.rs` | Ordered host declarations, language-visible names, import names, signatures, and private external-type eligibility shared with backend checks. |
| `src/semantic.rs` | Resolve and type-check user, host, and built-in calls; produce the typed AST. |
| `src/typed_ast.rs` | Typed expressions, resolved function/variable/loop references, and owned import metadata. |
| `src/flow.rs` | Private completion sets shared by semantic and backend summaries; distinguish normal completion, return, and loop transfers without pruning checks. |
| `src/codegen/abi.rs` | Internal language-type → Wasm-type mapping and signatures, separate from external eligibility checks for every host snapshot and selected user exports. |
| `src/codegen/layout.rs` | Deduplicate signatures, assign indices, and build the complete typed callee table before body emission. |
| `src/codegen/validation.rs` | Borrow the typed AST, complete layout and existing body checks in diagnostic order, and retain checked bodies behind a private construction boundary. |
| `src/codegen/validation_tests.rs` | Compare checked and former compilation sequences for valid bytes, multiply-invalid diagnostics, and immutable host/export snapshots. |
| `src/codegen/call.rs` | Validate resolved call references/signatures, emit user/host calls and built-in `len`, normalize host booleans, and validate memory handles. |
| `src/codegen/function.rs` | Per-function emission context, scoped initialized bindings, unique labels/loop contexts, return-path checks, and function termination. |
| `src/codegen/control_flow.rs` | Emit structured branches and loops, preserve scope boundaries, and track guaranteed returns. |
| `src/codegen/locals.rs` | Collect declarations, including inside expressions, and map typed variable IDs to dense Wasm slots in declaration preorder. |
| `src/codegen/temporaries.rs` | Preallocate ordered typed scratch slots for array construction and indexed compounds, including nested and unreachable expressions. |
| `src/codegen/mod.rs` | Validate raw typed input through the private checked constructor, then assemble Type, Import, Function, optional Memory/Global, Export, Code, and optional Data sections from checked state. |
| `src/codegen/tests.rs` | Inspect and validate emitted modules with `wasmparser`. |
| `src/codegen/expression.rs` | Check result categories and emit literals, reads, operators, calls, and MVP block/if value regions while reconstructing expression completion. |
| `src/codegen/statement.rs` | Emit blocks, scalar declarations/assignments, expression cleanup, returns, and delegate control flow. |
| `src/codegen/values.rs` | Emit fresh array construction and checked element addressing. |
| `src/codegen/place.rs` | Shared checked local/index places and ordinary/compound assignment lowering. |
| `src/codegen/memory.rs` | Plan static strings and emit allocation, descriptor validation, checked array addressing/length, and string/numeric-array equality helpers. |
| `src/codegen/numeric.rs` | Detect remainder throughout the typed AST and emit the optional memory-independent `(f64, f64) -> f64` guest remainder helper. |
| `src/codegen/range.rs` | Discover ranges/membership and emit private construction, record-validation, and exact membership helpers. |
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
| `range` | Internal-only `i32` handle to an immutable private record; host imports and selected range user exports are rejected. |

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

## Implemented lexical extension: comments

The lexer skips `//` line comments through LF, CR, or EOF and multiline
`/* ... */` block comments through the first `*/` (no nesting). Comments are
trivia between tokens anywhere whitespace can appear; delimiters inside strings
remain text. They separate tokens rather than joining identifier fragments or
operator bytes.

Skipping occurs in the lexer, not through source preprocessing, preserving
original UTF-8 byte spans. An unterminated block produces an `Invalid` token from
the opening slash to EOF, followed by `Eof`, and is reported by existing parser
errors. This extension adds no error stage and changes no AST, codegen, or ABI
contract.

Acceptance covers LF/CR/EOF line endings, multiline and non-nesting block
comments, adjacency to tokens, string delimiters, token separation, original byte
spans, and unterminated-block parser errors. Comments do not add a numbered
backend milestone.

## Implemented language extension: local initializer inference

Local declarations accept `let name = initializer;` as well as the existing
`let name: type = initializer;`. Every nonvoid expression already has a concrete
`number`, `bool`, `string`, or `number[]` type; an omitted annotation uses that
type. Empty `[]` infers `number[]` because only numeric arrays exist. Explicit
annotations continue to require exact initializer-type matches.

Semantic analysis checks the initializer before introducing the new binding.
A same-named initializer reference therefore resolves only to an existing outer
binding; otherwise it is unknown. Nested scopes may shadow bindings, while
same-scope duplicate checks and existing first-error ordering are preserved.
Once initialized, the binding's type is fixed for subsequent assignments.
An inferred void-returning call initializer uses the existing
`SemanticError::InvalidVoidType` for the local, not a new error variant.

This is local initializer inference, not function inference or whole-program
type unification. Parameter annotations remain mandatory; function results follow
the separate #11/#14 contract above. Every
local declaration still requires an initializer. No `i32` language type,
promotions, or implicit coercions are introduced: `number` remains IEEE-754 `f64`,
including integer-looking literals. Inference ends at semantic analysis; typed
local layout, traversal, lowering, and the host/memory ABI are unchanged.

Acceptance covers inferred literals and compound/call expressions of all nonvoid
types, empty arrays, exact explicit annotations, fixed-type reassignment,
outer-binding reads during shadowing, unknown self-references, void initializers,
and duplicate/error-ordering compatibility. Emission checks compare equivalent
annotated and inferred programs across optimization levels; runtime checks cover
initializer effects and shadowing. This extension adds no numbered backend milestone.

## Implemented language extension: compound assignments (WP1)

Statement-only `+=`, `-=`, `*=`, `/=`, and `%=` accept `number` variables and
identifier-rooted numeric-array elements. Target and RHS types must both be
`number`; assignment expressions, whole-array arithmetic, and general postfix
places remain unsupported.

Lowering resolves the target once, validates bounds, reads the old value, then
evaluates the RHS and stores to the original place. If the RHS changes an element
from `10` to `100` and returns `2`, addition stores `12`, not `102`. Invalid targets
trap before RHS effects. `src/codegen/place.rs` shares checked places with ordinary
assignment; indexed compounds reserve ordered `i32` address / `f64` old-value
scratch slots without reuse. Scalar updates keep the old value on the stack.
Both raw and typed statement enums add `CompoundAssignment { target, op, value }`;
public consumers matching statements must handle the new variant. Its `BinaryOp`
field is checked to allow only the five arithmetic operations.
Legacy array temporary groups and old-source Wasm bytes remain unchanged.

`%=` itself triggers exact remainder-helper discovery even without a binary `%`,
including hidden, unreachable, and runtime-skipped code. Scalar-only compounds
add no memory or implicit imports. There is no new language type, ABI, or optimizer.
Acceptance includes frontend errors/spans, forged typed nodes, decoded instruction
and scratch layout, validator checks, aliases, once-only effects, bounds-before-RHS
traps, memory growth, numerical special values, and optimization/export parity.

See the [extension specification](was_language_extensions_spec.md) and
[implementation plan](was_language_extensions_plan.md) for later work packages;
Closure-dependent captured scalar mutation and array-binding rebinding tests
belong to WP11, not this milestone.

## Implemented language extension: while loop transfers (WP2)

`break;` exits the nearest enclosing while and `continue;` reevaluates its
condition, including under nested branches and loops. Transfers cannot cross
function boundaries. WP2 introduces bare semicolon-terminated transfers; WP4
below adds statement/value loop and value breaks. WP5 adds range/array
statement for; labels remain future work. Break/continue reserve previously usable source and host-language names;
rename existing bindings/functions using them. Host module/field names remain
unaffected.

The raw statement enum gains unit `Break` / `Continue` variants. The typed API
adds per-function `LoopId`, `While.id`, and `Break { target }` /
`Continue { target }`; public constructors and matches must be updated. Semantic
IDs are assigned in source preorder. Checked lowering accepts sparse unique IDs
but rejects duplicate IDs and unknown, inactive, outer-instead-of-inner, or
cross-function targets at their statement spans.

Private `Flow` completion sets separate return from loop transfers. Sequence only
fallthrough paths in summaries, but analyze and emit all statements, including
ones skipped by transfers. A while conservatively may complete normally; even
`while true` is not a guaranteed function return. Return statements after transfers
do not turn those transfers into guaranteed returns.

`BodyEmitter` tracks unique identities for every structured label, including
short-circuit expression ifs, and calculates relative `LabelDepth` from the active
stack. Break targets the exit block; continue targets the loop condition. Scope,
label, and loop contexts restore on errors. No new Wasm feature, helper, memory,
ABI, optimizer, or execution-fuel mechanism is introduced. Legacy loop bytes and
diagnostic fixtures remain unchanged.

Acceptance covers syntax and spans, keyword host validation, per-function nearest
targets, flow composition, forged IDs, cleanup, independent decoded branch depths,
validator checks, recursion, condition effects, skipped calls/traps, numeric-array
scratch planning, optimization/export/import-snapshot parity, and CLI/browser
behavior. WP3 extends these transfers through expression regions and conditions.

## Implemented language extension: block/if values and `Never` (WP3)

Expression blocks yield an optional unterminated tail; a semicolon discards it.
Value-context `if` / `else if` requires else, with exact branch-category joins
except that Never joins its counterpart. All existing concrete types work; Void
is discardable, not a local value. Statement-if may omit else and discards branch
tails. Named functions still require explicit return; function/while ordinary
expression tails and standalone block statements without semicolons are rejected.
A final bare if in a value block is a tail, a nonfinal one is a statement. Bare
statement-if ends before a following unary-minus statement, regardless of trivia;
parenthesize the whole if for arithmetic continuation in those positions.

`ValueBlock` / `TypedValueBlock` wrap statements, an optional boxed tail, and span.
`TypedExpr.ty` now carries `ExprResult::{Value(Type), Void, Never}`; parameters,
bindings and function signatures retain Type. Never means no normal completion,
not a source/ABI/storage type or termination proof. Inferred Never initialization
is rejected; annotated divergent initialization emits no reachable store.
`Value(Type::Void)` is malformed. Public result equality is identity, not join
compatibility. Semantic analysis propagates transient expression flow; checked
lowering reconstructs it, checking every child even after abrupt control.

MVP single-result block/if regions use f64 for numbers, i32 for bool/handles, and
empty for Void/Never. No artificial result or drop is introduced on divergent
paths. While loop context is active before its condition, so condition blocks
can break/continue that loop. An unconditional explicit return in the condition
guarantees return before the body; a literal-true body alone still does not.
The walker visits value-block statements then tail within scope hooks. Locals
traverse expressions for declarations, reserving an outer let slot before its
initializer declarations. Separate memory, scratch and remainder discovery,
helper ordering, host ABI, and optimization no-op behavior are unchanged.

Coverage includes `tests/block_values.rs`, frontend/planner/checked-backend units,
independent decoded MVP region/local/drop checks plus Validator, legacy byte
expectations, and Node tests of effects, scopes, transfers, saved compound places
and trap order. CLI/browser parity and diagnostics, forged annotations, hidden
bodies and skipped operands remain covered. WP4 extends these regions.

## Implemented language extension: statement/value loops (WP4)

`loop` is now a reserved source/host-language keyword. Statements allow bare
breaks; loop expressions produce number, bool, string or numeric-array results
through value breaks. Raw/typed statement and expression enums add `Loop { body }`
(with `id` in typed nodes), while preserving strict statement-vector bodies.
Bare Break variants remain; raw `BreakValue(Expr)` and typed
`BreakValue { target, value }` are separate. No new ABI or storage Type is added.
Prefix bare loops, including `loop {};`, are statements. A final bare loop in a
value block without `;` is a tail; nonfinal/semicolon-terminated loops are
statements. Parenthesized loops are expressions. Whitespace never chooses context.

Only reachable breaks targeting a value loop constrain its result, with exact
nonvoid joins. Nested-loop constraints are consumed at their own boundary. Private
Flow carries reachable constraints through sequence/branch composition and now
separates explicit divergence. Unreachable breaks still require the right loop
kind, nonvoid RHS and valid children, but do not type the loop. A Never RHS supplies
no value or actual break. A no-exit loop is Never; inferred initialization fails,
annotated initialization has no reachable store. Nonvoid functions may return or
explicitly diverge, never fall through. Ordinary while remains conservative even
with literal true; noncompleting conditions consume their own transfers accurately.

MVP lowering uses a typed outer block and empty backedge loop, with identity-based
branch depths and no artificial result local. The declared result chooses the
region type; checked lowering reconstructs and verifies the outcome. An unreachable
break with an incompatible raw payload emits Unreachable instead of an invalid
branch, while a reachable conflict still rejects compilation. Scope/label cleanup,
helper order and separate memory/local/scratch/remainder planning are preserved.

`tests/value_loops.rs` and focused frontend/planner/backend tests cover result
joins, Never, unreachable checks, forged targets/kinds/annotations, legacy bytes,
exports and owned imports. Independent decoded regions/branches/locals plus
Validator accompany bounded Node effects/alias/transfer/stack-prefix tests and
CLI/browser parity/diagnostics. Infinite cases are compile-only; resource controls
remain the embedding's responsibility. The implemented WP5 checkpoints follow below.

## Implemented language extension: range construction/internal calls and membership (WP5)

This checkpoint adds public `Type::Range` and raw/typed range expressions with
boxed start/end, optional boxed step, and inclusivity. Source `..` / `..=` are
non-associative, below arithmetic and above comparisons; optional contextual `by`
sets the step, defaulting to `+1`. Numeric operands retain Never compatibility.
Range locals, assignments, internal parameters/results, blocks, ifs, and value
loops use exact type joins with Never; range equality is prohibited.

All normally completing operands evaluate once, left-to-right, before validation,
even with an invalid earlier operand or an empty/discarded range. Executed
construction requires finite integral values within ±(2^53−1) and nonzero step
(reject ±0); negative-zero endpoints become integer zero. Invalid constants trap
at runtime, not as new static errors. Each construction bump-allocates an immutable
32-byte, eight-byte-aligned private record; copies share its handle. Internal
parameters/results use `i32`, with record bounds/field validation at parameter
entry. See the [private layout](memory-abi.md#private-range-record).

Reject every range host-import signature, including unused/forged snapshots, and
only selected user range signatures for export. Default export-all therefore
fails; compatible `Only` / `None` permits internal calls without pruning bodies or
checks. There is no public range marshaling. Private `Construct`, then `Validate`
helpers append after memory and optional remainder helpers; optional `Contains`
follows only when membership exists anywhere in the typed AST. Existing indices,
construction-only bytes, and feature-free bytes remain unchanged.

`number in range` returns `bool` at comparison precedence with existing left
associativity; `in` is reserved for source/host-language names, not host module/field
names. Both operands evaluate once, left-to-right, using exact types with Never
compatibility and checking all children, including skipped/unreachable code.
The helper validates the consumed record before the candidate guard: malformed
records trap even for invalid candidates. Nonfinite, fractional, or out-of-safe-domain
candidates return false; both signed zeros are accepted. Signed `i64` interval and
`I64RemS` congruence checks need no floating remainder helper; see the
[arithmetic bounds proof](memory-abi.md#private-range-record).

`tests/range_frontend.rs`, `tests/ranges.rs`, and `src/codegen/range_tests.rs` cover
syntax/type rejection, exact/Never joins, forged input, independent record bytes,
Validator and decoded layout/instructions, internal signatures, export/import
selection, and optimization parity. Bounded Node effects/trap/record-validation
cases retain the ignored-test policy; default tests alone do not prove they ran.
Membership coverage in `tests/range_frontend.rs` and `tests/ranges.rs` includes
reserved names, precedence/associativity, exact/Never typing, checking both children,
and decoded integer lowering without floating remainder.
Range and array statement for checkpoints follow below. WP5 implementation and
all release gates are complete; exact commands/results are recorded in
[extension plan §5](was_language_extensions_plan.md#5-documentation-and-validation-workflow).

### Range-only statement for checkpoint (issue #4)

Issue #4 introduced reserved `for name in range_expr { body }` with existing
Never compatibility; issue #5 below extends the same node to numeric arrays.
Raw statements add
`For { name, iterable, body }`; typed statements add
`For { id, variable, name, iterable: Box<TypedExpr>, body }`. For is a void statement,
not a value expression, and retains a strict statement-vector body. Loop and
variable IDs use existing per-function uniqueness/scope/target checks.

Evaluate the iterable once in the enclosing context before activating the new
loop or iteration binding. Break/continue within it targets the outer nearest
loop under its usual rules, or is invalid without one; preserve return/divergence
without inventing fallthrough, while still checking the body. A normally completing
range may be empty, so a returning body alone does not guarantee return.
The numeric iteration binding is read-only for ordinary and compound updates,
including unreachable writes; distinct inner shadowing lets remain mutable.
Checked lowering reconstructs write restrictions from For provenance, with no
new public mutable-binding metadata. Bare breaks are permitted and value breaks
are rejected even when unreachable; nested value loops retain their own rules.

Validate the consumed record and cache exact `i64` current/end/step and `i32`
inclusivity scratch. Signed endpoint checks precede exact `f64` conversion/yield,
with integer zero yielding +0. A body latch block makes continue exit to `I64Add`
advancement before backedge/retest; break and return skip advancement. Resolve
branch depths from label identities through nested expression regions. The walker
visits iterable then body within body block hooks; local/memory/temporary/remainder
policies remain separate, with no-reuse scratch slots. Range helpers and ordering
are unchanged: for adds no helper, `Contains` remains membership-only, and
feature-free bytes/indices retain their compatibility contract.

Let `M = 2^53−1`. Every yielded current and validated nonzero step have magnitude
≤ M, hence `|current + step| <= 2M < 2^54` fits signed i64. Next can leave the safe
domain, but is endpoint-tested before conversion/yield and exhausts normally.
The loop exits on the first failed signed check, so no subsequent addition uses
that out-of-domain current. See the [full proof](memory-abi.md#private-range-record).

`tests/for_frontend.rs` covers syntax/types/scopes, read-only writes, mutable
shadowing, statement-only contexts, iterable transfers, and unreachable checks.
`tests/for_lowering.rs` covers forged trees/annotations/IDs, planner slot types and
preorder, Validator plus independent decoded cache/conversion/latch checks, and
error/optimization/export-selection parity. `tests/for_loops.rs` covers deterministic
output, helper counts/no-reuse scratch, owned import snapshots and export indices;
its bounded Node cases compare yields to an independent BigInt progression and
membership, exercise ±safe limits/large steps/+0/normal exhaustion, and check
once-only effects, nested transfers, consumed-record validation, and cached fields.
All optimization levels are covered. Node cases retain the ignored-test policy;
default Cargo success alone does not establish runtime coverage.

### Numeric-array statement for checkpoint (issue #5)

The same `for name in iterable { body }` accepts `number[]` or `range` with Never
compatibility, without parser or raw/typed AST shape changes. Array evaluation is
once in the enclosing context before activating the loop/binding; shared transfer,
return/divergence, read-only numeric binding and mutable inner-shadowing rules are
preserved. An empty array can skip the body and does not establish definite return.

Fully validate the descriptor/payload with existing `ValidateArray`, even when
empty, then cache handle and unsigned `u32` length once. Use unsigned index tests
and load each current element through existing checked `ArrayAddress` then `f64`
load. Do not snapshot elements or cache a raw payload pointer: alias mutations to
unvisited elements are visible, while source reassignment leaves the captured
handle unchanged. Continue exits the body latch before index advancement/retest;
break/return skip advancement. ABI and helper order are unchanged, and array-only
iteration needs no range helpers.

`tests/array_for_loops.rs` is the array-for suite, alongside shared frontend and
forged-lowering coverage in `tests/for_frontend.rs` and `tests/for_lowering.rs`.
CLI/browser compilation uses the shared core for both iterable types. Issue #6
completes adapter/interaction support and all WP5 release gates passed, including
rebuilt browser bindings and the full ignored-test run; see extension plan §5.
The [WP5 checkpoint](was_language_extensions_plan.md#wp5--safe-ranges-rangearray-for-membership-spec-56)
records the cross-feature acceptance coverage.
The default Cargo suite still skips Node cases. WP6/WP7 have incremental
development slices described above; their release gates and WP8–WP13 remain
future work. External range signatures, range equality, and allocation-free range optimization
remain out of scope.

## Remaining work

M0–M9 are implemented; this is not a claim that every language/runtime feature is
complete. [Limitations and alternatives](limitations.md) records unsupported
features, ownership/resource policies, raw-call/ABI constraints, and current
workarounds versus possible future designs.

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
