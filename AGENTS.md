# Agent guide

This guide applies to the entire repository. Keep it current when changing the
architecture, supported language, public contracts, or validation workflow.

## Purpose and working rules

`was-compiler` compiles a small, statically typed language to raw WebAssembly using
`wasm-encoder`. The priorities are understandable compiler stages, deterministic
output, explicit host integration, and maintainable, idiomatic Rust. It is not an
execution engine. Host registration declares imports; it does not install callbacks.

- Read the affected implementation and tests before editing. Keep changes focused;
  preserve user work and do not stage or commit unless requested.
- Distinguish behavior-preserving refactors from language, ABI, and optimization
  changes. Do not introduce speculative frameworks, another IR, or dependencies.
- Keep the core library portable to native targets and `wasm32-unknown-unknown`.
  Filesystem access, CLI dependencies, and JavaScript bindings belong in adapters.
- Use offline build commands. If dependencies or tooling are unavailable, report
  the prerequisite rather than silently installing or fetching anything.
- Treat implementation and tests as evidence of current behavior. Consult
  [README.md](README.md), [grammar.bnf](grammar.bnf), and the documents below.
  The BNF is not a perfect lexer specification: actual strings accept raw Unicode,
  and whitespace recognition is broader. Reconcile differences deliberately.

| Reference | Responsibility |
| --- | --- |
| [Compilation roadmap](docs/compilation-roadmap.md) | Architecture, milestones, acceptance criteria |
| [Memory ABI](docs/memory-abi.md) | Authoritative descriptor, allocation, ownership, and host contract |
| [Limitations](docs/limitations.md) | Implemented restrictions versus possible alternatives |
| [Runtime integration](docs/runtime-integration.md) | CLI, raw execution, imports, explicit marshaling |
| [Browser guide](docs/browser.md) | Compiler-in-Wasm API, packaging, playground integration |

## Compiler pipeline and ownership

```text
source -> lexer -> parser AST -> semantic analysis + host signatures
       -> TypedProgram -> optimization (currently no-op)
       -> ABI/module layout -> checked function-body lowering
       -> private ValidatedProgram -> wasm_encoder section assembly -> Wasm bytes
```

The library entry points are `Compiler::analyze`, `Compiler::compile`, and
`Compiler::compile_source`. Analysis produces an owned typed program; compilation
uses its host-declaration snapshot, not the compiler's subsequently edited registry.
`analyze` is independent of optimization settings. Direct `codegen::compile`
bypasses the compiler's optimization/options orchestration.

| Location | Responsibility |
| --- | --- |
| `src/lib.rs`, `src/compiler.rs` | Public API, configuration, pipeline, stage errors |
| `src/tokens.rs`, `src/lexer.rs` | Tokens and UTF-8 byte-offset spans |
| `src/parser.rs`, `src/ast.rs` | Recursive-descent parsing and untyped syntax |
| `src/semantic.rs`, `src/typed_ast.rs` | Types, scopes, return analysis, resolved semantic IDs |
| `src/host.rs`, `src/exports.rs` | Ordered host declarations and owned export selection |
| `src/optimization.rs` | Borrowed `Cow<TypedProgram>` no-op; future transformation boundary |
| `src/codegen/abi.rs` | Language-to-Wasm signatures and named memory-layout constants |
| `src/codegen/layout.rs`, `src/codegen/indices.rs` | Signatures, callees, exports, distinct Wasm index types |
| `src/codegen/walker.rs` | Shared structural traversal for separate planning policies |
| `src/codegen/locals.rs`, `src/codegen/temporaries.rs` | Local-slot and temporary planning |
| `src/codegen/function.rs` | `BodyEmitter`, scoped initialized bindings, private emission context |
| `src/codegen/expression.rs`, `src/codegen/statement.rs` | Expression and statement lowering |
| `src/codegen/control_flow.rs`, `src/codegen/call.rs`, `src/codegen/values.rs` | Structured control flow, calls, literal values |
| `src/codegen/memory.rs`, `src/codegen/numeric.rs` | Memory planning/helpers and optional floating remainder helper |
| `src/codegen/validation.rs`, `src/codegen/mod.rs` | Checked internal boundary and section assembly |
| `src/diagnostic.rs` | Shared native/browser diagnostic rendering |
| `src/main.rs`, `src/cli.rs` | Thin native entry point and clap/file/output adapter |
| `browser/src/lib.rs` | Separate wasm-bindgen compiler adapter using the same core pipeline |
| `runtime/`, `examples/browser/` | Optional Node execution helpers and browser playground |
| `tests/`, `src/codegen/tests/`, `src/codegen/memory_tests/` | Integration contracts and focused backend suites |

`src-python/` contains separate Python parser/REPL code. It is outside the Cargo
pipeline and is not authoritative for the Rust compiler's language behavior.
`target/` and `pkg/` are generated output, not implementation sources.

### Preserve the internal boundaries

- The planning walker is sequential and structural, not semantic analysis or code
  emission. Keep exhaustive node traversal centralized and policies separate.
  Locals skip expressions; remainder discovery stops at the first `%` before
  visiting its children. Successful block-exit hooks are not unwind cleanup.
- `ValidatedProgram` privately borrows the original typed tree, completes layout
  first, then checks/lowers every user function in source order and caches encoded
  bodies. Module assembly consumes that checked state. Do not clone the AST or add
  a second independently implemented validator to accomplish the same checks.
- Public typed nodes, IDs, and annotations are mutable and can be forged. Retain
  backend checks; do not assume every caller went through semantic analysis.
  The internal boundary is not a public checked-AST API or a guarantee that every
  arbitrary malformed tree produces a recoverable error.
- Keep `CompilerOptions` small and `Copy`; owned `ExportOptions` stays separate.
  Optimization levels `None`, `Basic`, `Standard`, and `Aggressive` currently have
  identical output and diagnostics. Do not add optimization as incidental cleanup.

## Current language support

- Types: `number`, `bool`, `string`, `number[]`, and `void`. Parameters, results,
  and initialized `let` declarations require explicit types; no implicit coercion.
- Named functions support forward, nested, self-recursive, mutual, and registered
  host calls. Statements include `let`, variable/index assignment, expression
  statements, `return`, `if`/`else`, and `while`.
- Expressions support literals, parentheses, unary `-`/`!`, arithmetic
  `+ - * / %`, numeric comparisons, boolean equality and short-circuit `&&`/`||`,
  string equality, numeric-array equality, indexing, and built-in `len(array)`.
- Strings are immutable raw UTF-8 values. Numeric arrays have fixed lengths and
  mutable shared storage; every evaluated array literal allocates fresh storage.
- No `for`, `foreach`, `break`, `continue`, comments, string escapes, exponent
  literals, `.5`, `1.`, string concatenation/indexing/`len`, or general postfix
  chaining. Source indexing starts from an identifier.
- No other array element types, modules, closures, generics, free, or garbage
  collection. Consult the limitations document before claiming support.
- Source parsing requires at least one function; the lower-level typed API can
  represent an empty program. An empty `void` body is valid.

## Compatibility contracts

These are deliberate semantics, not implementation accidents. Preserve them in
refactors; specify and test any intentional behavior change explicitly.

### Evaluation, scopes, and module layout

- Evaluate operands, call arguments, and array elements exactly once, left-to-right.
  Indexed assignment validates the target and bounds **before evaluating the RHS**.
- Logical operators short-circuit at runtime, but both operands, unreachable
  statements, and unselected function bodies still receive static/backend checks.
- Nonvoid functions require a statically recognized guaranteed return. A loop alone
  is not a guaranteed return, even with a literal `true` condition.
- Variable IDs are per-function and may be sparse. Local slots place parameters
  first, then declarations, then array temporaries. Preserve shadowing, scoped
  initialization restoration, and the current no-reuse temporary policy.
- Emit imports in registration order and user functions in source order. Deduplicate
  signatures without confusing type indices with function indices. Use the
  `FunctionIndex`, `TypeIndex`, `LocalIndex`, and `LabelDepth` newtypes internally.
- Keep output deterministic. Hash maps/sets are lookup tools, not emitted-order
  authorities. Preserve static-string first-discovery order and helper ordering.
- Export selection changes only the public surface: no body/import pruning or
  index shifts. Required memory exports remain even with no user-function exports.
- All registered imports must be supplied by the embedding, including unused ones.
  There is no implicit `print`, WASI, or automatically invoked guest start function.

### Numbers and equality

- `number` and array elements are IEEE-754 `f64`, even for integer-looking literals.
  Overflowing literals are errors; underflow is allowed. Arithmetic can produce
  NaN, infinities, and signed zero. Do not assume finite results or exact integers.
- `%` is truncating floating remainder, matching JavaScript `%`/`fmod`, not
  mathematical modulo or nearest-quotient IEEE remainder. Wasm has no `f64.rem`.
  The optional guest helper uses exact scaled subtraction and needs no implicit
  host import or memory. Never replace it with `a - trunc(a / b) * b`.
- Remainder preserves the dividend's signed zero; NaN operands, an infinite
  dividend, or a zero divisor produce NaN. Finite `%` infinite returns the dividend.
- Array equality compares lengths and exact elementwise `f64` values: `+0 == -0`,
  but NaN is unequal even when comparing an array handle to itself. No pointer or
  byte-equality shortcut. `!=` is the complement of `==`.
- Evaluate both array operands before reading compared elements; RHS mutations
  must be visible. Validate **both complete descriptors before early exits**.
  String equality compares UTF-8 bytes, without normalization or case folding.

### Memory and host ABI

- Centralize production constants/signatures in `src/codegen/abi.rs` and helper
  ordering in `Helper::ALL`; do not duplicate magic offsets in emitters.
- Raw types: `number = f64`; `bool = i32`; strings/arrays = `i32` handles despite
  being distinct language types. Normalize host boolean boundaries to `0`/`1`.
- A descriptor is eight bytes, eight-byte aligned, with little-endian `u32` data
  pointer at offset `0` and length at offset `4`. String length counts bytes;
  array length counts eight-byte `f64` elements with eight-byte-aligned payloads.
  Addresses below `8` are invalid. Valid host payloads need not be contiguous with
  their descriptors. Handles are instance-local and assignments can alias arrays.
- The bump allocator rounds to eight-byte alignment and reserves at least eight
  bytes. There is no freeing. Memory32 grows in 65,536-byte pages with a maximum
  of 65,535 pages; use widened unsigned range checks and explicit overflow checks.
- Preserve memory helper order: `Alloc`, `ValidateString`, `ValidateArray`,
  `ArrayAddress`, `ArrayLen`, `StringEqual`, `ArrayEqual`. The optional remainder
  helper follows user functions and any memory helpers. Scalar-only programs must
  not gain memory/global/data sections just to support `%`.
- Indices must be finite, integral, nonnegative, and in bounds; negative zero
  addresses element zero. Invalid indices/descriptors and allocation failure trap.
- Hosts must preserve descriptor metadata, valid UTF-8, and string immutability.
  Trusted hosts can mutate array elements. Recreate JavaScript views after possible
  memory growth. Guest bounds checks do not isolate malicious hosts or validate UTF-8.
- The compiler emits raw Wasm with no automatic JSON interface metadata. Optional
  runtime conversions are explicit copies, not part of the compiler ABI.
  There is no compiler-provided execution fuel, cancellation, or transactional rollback.

### Diagnostics and CLI

- Preserve error variant, message, span, and first-error ordering for unchanged
  behavior. Keep exhaustive `span() -> Option<Span>` extraction beside stage-error
  definitions and delegate through `CompileError`; do not silently discard locations.
- Spans are UTF-8 byte offsets. Expressions have no independent spans; nested
  expression errors generally use the enclosing statement. Return raw offsets
  from accessors; only the renderer clamps invalid ranges for presentation.
- Preserve character-based columns, four-space excerpt tabs, CRLF/EOF handling,
  and consistent native/browser diagnostics. Precise expression spans and recovery
  are separate features, not formatting refactors.
- The CLI requires a file, never compiles a fallback example, and does not read
  stdin. Source `-` is a literal filename; exact output `-` means text stdout.
  Default output is plain WAT; only exact lowercase `.wasm` infers binary.
  `--emit` overrides inference, and binary requires a filesystem output path.
- Help exits `0`, usage errors `2`, and compilation/registration/I/O failures `1`.
  Keep CLI binary output identical to library bytes for matching inputs/options.
- Preserve canonical same-input refusal, no automatic parent-directory creation,
  sibling `create_new`/write/sync/rename output, temporary cleanup, and unchanged
  destinations after compilation failure. Do not overstate race or durability guarantees.
- Rust host registration accepts arbitrary nonempty module/field names. CLI host
  syntax is delimiter-limited; do not impose its restrictions on the library API.

## Implementing a feature

1. Establish syntax, typing, evaluation order, failure behavior, and ABI impact.
   Read neighboring tests and the limitations/roadmap before choosing a design.
2. Update tokens/lexer, parser, `grammar.bnf`, and untyped AST where applicable.
   Keep precedence, associativity, assignment targets, and malformed-input handling explicit.
3. Extend semantic analysis and typed AST with exact type/scope/return rules and
   stable IDs. Update exhaustive matches; do not bypass the type system in codegen.
4. Extend the shared walker for new structural children, then adjust relevant
   memory, local, temporary, and numeric planning policies separately.
5. Update ABI/layout/helper signatures and typed indices if needed. Lower through
   the existing emitter modules, retaining checks for forged typed input and stack balance.
6. Add stage errors and span accessors where needed. Propagate configuration through
   the library and relevant CLI/browser adapters without duplicating compiler logic.
7. Add tests at every affected boundary, including rejection and observable effects.
   Update README and relevant docs to distinguish implemented behavior from proposals.
8. Run focused checks first, then broader validation appropriate to the change.
   Report actual commands/results and any unavailable validation. Update this guide
   when the architecture, features, or workflows change.

## Rust coding guide

- Use Rust 2024 and existing project idioms. Prefer small cohesive functions,
  descriptive names, early returns, exhaustive enum matches, and explicit ownership.
  Add blank lines between declarations and logical blocks; avoid dense method chains.
- Keep visibility minimal (`pub(super)`/private for backend internals). Prefer enums
  and index/ID newtypes over boolean mode flags, strings, or interchangeable integers.
  Unwrap typed indices with `.raw()` at encoder/comparison boundaries, not throughout planning.
- Borrow `&str`, slices, and references when ownership is unnecessary. Own values
  when they must outlive source/registry input. Clone deliberately, never just to
  evade borrowing; do not clone whole ASTs or large collections in hot paths.
  Use `Cow` only when borrowed-versus-owned behavior is meaningful.
- Use iterators when they clarify a transformation and loops when they clarify
  state, ordering, or errors. Reserve collection capacity when size is cheaply
  known; avoid repeated allocation, string formatting, and lookup scans in hot paths.
  Do not merge distinct planning policies merely to eliminate a traversal.
- Propagate recoverable errors with `Result`, typed errors, and `?`. Production
  `expect`/`assert` must express a justified internal invariant, not malformed source
  or routine I/O. Do not add broad panic catching or blanket lint suppressions.
- Use checked arithmetic/conversions for sizes, offsets, indices, and memory ranges.
  Distinguish byte alignment from Wasm's log2 alignment immediate. Explicit bit
  reinterpretation is valid when intentional; lossy `as` casts are not validation.
- Prefer safe Rust and standard-library/existing dependencies. New unsafe code,
  concurrency, global caches, or packages require a concrete need and documented
  tradeoffs. Keep planning sequential unless parallelism is explicitly requested.
- Optimize from measurements, not assumptions. Preserve IEEE-754 behavior,
  deterministic order, short-circuiting, side effects, and trap order. No algebraic
  rewrites, constant folding, or dead-code removal inside a nominal refactor.
- Comments should explain non-obvious intent, invariants, or tradeoffs, not narrate
  code. Document public contracts where useful. Do not reformat unrelated files.

## Mandatory testing

**Every feature and regression fix must include tests.** Refactors must preserve
existing tests and add focused coverage for newly introduced boundaries when needed.

- Test frontend acceptance/rejection, types/scopes, planner behavior, and lowering
  for every affected stage. Include malformed source and forged typed-AST cases.
- For emitted modules, use `wasmparser::Validator` plus independent decoded
  instruction/layout assertions. Valid Wasm alone does not prove correct semantics.
- Execute generated Wasm for observable behavior where appropriate: host effects,
  short-circuiting, evaluation/trap order, mutation/aliasing, loops, and returns.
  Check unexported, unreachable, and runtime-skipped code for static validation.
- Numeric tests cover NaN, infinities, signed zero, subnormals, and extremes as
  applicable. In JavaScript use `Number.isNaN` and `Object.is` for special values;
  use independent expected results/reference behavior, not the production algorithm.
- Preserve import snapshots, deterministic ordering, export-selection behavior,
  and optimization-level parity. Adapter changes need CLI/browser contract tests.
- Use `tests/support/process.rs` for bounded subprocess tests: it drains pipes
  concurrently, applies deadlines, and kills/reaps the direct child. It is not a
  process-tree controller or guest fuel mechanism. Keep specialized decoders local;
  avoid building a generic testing framework.
- Preserve the existing ignored-test policy for Node/browser dependencies. A passing
  default Cargo suite is not evidence that ignored runtime cases passed.
- Golden fixtures under `tests/fixtures/abi`, `tests/fixtures/emitter`, and
  `tests/fixtures/diagnostics` encode compatibility expectations. Do not regenerate
  them just to make tests pass. Intentional changes require an explicit contract
  decision; retain independent expected bytes/offsets instead of deriving all
  expectations from production constants. Preserve diagnostic fixture newline bytes.

### Validation commands

Run from the repository root. Start with relevant test filters; use these for
broader checks. Bound long-running commands in agent tools.

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

For core/browser changes, rebuild `pkg/` before browser tests or the full ignored
suite; stale generated bindings do not validate current source. Node.js 24+,
`wasm32-unknown-unknown`, wasm-pack, compatible wasm-bindgen tooling, and cached
Rust dependencies are prerequisites. `--mode no-install` avoids tooling installation.
There is no root `browser` feature; the dedicated workspace crate is the adapter.

```sh
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline
node tests/browser.cjs pkg
cargo test --offline -- --include-ignored
cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings
```

For CLI/runtime changes, a useful smoke test returns `65`:

```sh
cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main
node runtime/run.cjs target/array-sum.wasm
```

Format only edited Rust files, supplying their actual paths:

```text
rustfmt --edition 2024 --config skip_children=true <edited Rust files>
```

Do not run workspace-wide formatting that changes unrelated files. Finish with
`git --no-pager diff --check` and review the diff/status. For documentation-only
changes, check links/paths and whitespace; a code test run is normally unnecessary.
Never claim a test passed unless it ran successfully; report missing tooling,
ignored coverage, and unrelated failures without modifying unrelated code.
