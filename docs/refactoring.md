# Refactoring opportunities (excluding optimizations)

This is an incremental maintainability plan, not an implementation commitment.
It concerns the inspected Rust frontend/backend/tests and shipped M7 integration:
[cli.rs](../src/cli.rs), [diagnostic.rs](../src/diagnostic.rs), and the
[Node runtime](../runtime/index.cjs).
[limitations.md](limitations.md#status-and-scope) records current restrictions;
[runtime-integration.md](runtime-integration.md) covers usage. The recommendations
below build on existing boundaries rather than proposing extraction already done.

**Out of scope:** all optimization passes, constant folding, dead-code/dead-import
removal, temporary-slot reuse, instruction simplification, and an IR introduced
for an optimizer. No new framework, plugin system, execution engine, or dependency
is required for the refactors below. Language extensions and runtime bug fixes
must be tracked separately from behavior-preserving restructuring.

## Contracts to preserve

A refactor should not silently change any of these properties:

- Source syntax/type rules, first-error selection, error variants/spans, and
  validation of unreachable statements and short-circuited operands.
- Left-to-right expression/call/array-element evaluation, short-circuit effects,
  and array-assignment bounds checks before RHS evaluation.
- `number = f64`; boolean host normalization; distinct string/array language
  types represented by `i32`; [descriptor layout and ownership](memory-abi.md),
  allocator behavior, UTF-8 byte equality, and guest array aliasing.
- Registration-order import indices, source-order user-function indices after
  imports, runtime helpers after users, deterministic signature interning, and
  per-function locals ordered as parameters then declarations then temporaries.
- The typed program's owned host snapshot, rather than the compiler's later
  registry state, determines compilation imports.
- Deterministic Wasm bytes, sections, instructions, static-string handles, and
  errors for the same inputs/options. CLI and library binary output remain the
  same raw bytes; WAT remains plain textual output.
- M7's separate `ExportOptions` and unchanged `Copy` `CompilerOptions`. Export
  selection must not remove functions/imports or shift indices; required
  `memory`/`__alloc` exports remain. The Node runtime passes raw arguments/results
  and does not reserve function names. Explicit `createAbi` conversions copy
  strings/arrays; direct memory access retains reference semantics. Only genuine
  memory ABI export collisions prevent user exports with those names.
- CLI format inference/overrides and filesystem-only binary output, exit `2` for
  usage versus `1` for compilation/registration/I/O errors, required source-file
  input (no stdin or built-in fallback), help exit `0`, and current
  source-excerpt diagnostics. Preserve sibling `create_new`/sync/rename writes,
  refusal of canonical same-input paths, unchanged destinations after compilation
  failure, and no implicit parent-directory creation.

Tests should expose a deliberate contract change, not be weakened to accept a
refactor. If a bug or feature requires new behavior, use a separate change with
new acceptance criteria instead of calling it a refactor.

## Prioritized sequence

| Order | Opportunity | Evidence / starting point | Incremental acceptance gate |
| --- | --- | --- | --- |
| 1 — high, implemented | Split large backend tests; share bounded subprocess support | [scalar tests](../src/codegen/tests/), [memory tests](../src/codegen/memory_tests/), [process harness](../tests/support/process.rs), [harness tests](../tests/process.rs) | Existing assertions and ignored-runtime policy preserved; bounded child cleanup and useful stdout/stderr. |
| 2 — high, implemented | Name ABI constants and strengthen cross-language layout fixtures | [abi.rs](../src/codegen/abi.rs), [memory.rs](../src/codegen/memory.rs), [runtime/abi.cjs](../runtime/abi.cjs), [ABI tests](../tests/abi.rs) | Pre-refactor module bytes preserved; independent raw-ABI and explicit-conversion roundtrips agree. |
| 3 — medium, implemented | Add a small function-body emission facade and index/label types | [function.rs](../src/codegen/function.rs), [indices.rs](../src/codegen/indices.rs), [emitter tests](../tests/emitter.rs), [layout.rs](../src/codegen/layout.rs) | Pre-refactor bytes and instruction/index output preserved, including nested control flow and live stack prefixes. |
| 4 — medium | Share structural typed-AST walking for planning | [memory.rs](../src/codegen/memory.rs), [locals.rs](../src/codegen/locals.rs), [temporaries.rs](../src/codegen/temporaries.rs) | Same preorder, static handles, declaration order, temporary count, and planning error order. |
| 5 — medium | Define a validated typed-program boundary | [typed_ast.rs](../src/typed_ast.rs), [semantic.rs](../src/semantic.rs), [compiler.rs](../src/compiler.rs), backend malformed-tree tests | Checked/legacy paths agree on bytes and diagnostics; no unchecked bypass hidden by the new API. |
| 6 — medium | Add common span accessors and codify configuration/output contracts | [tokens.rs](../src/tokens.rs), [parser.rs](../src/parser.rs), [compiler.rs](../src/compiler.rs), [cli.rs](../src/cli.rs), [diagnostic.rs](../src/diagnostic.rs) | Existing module split, errors/rendering, raw-byte compatibility, CLI formats/statuses, and output safety preserved. |

All starting points are present. Prefer small reviewable changes that preserve
existing characterization tests and add focused coverage before each move.

## 1. Smaller test modules and one subprocess harness: implemented

[tests.rs](../src/codegen/tests.rs) now owns shared private scalar decoding and
typed-tree helpers, with 93 existing tests split into responsibility modules in
[tests/](../src/codegen/tests/): layout, expressions, locals, calls, control flow,
malformed trees, and runtime execution. [memory_tests.rs](../src/codegen/memory_tests.rs)
keeps its specialized decoder and builders; its 21 tests live in
[memory_tests/](../src/codegen/memory_tests/) layout, validation, and runtime modules.
The memory runtime module owns its JavaScript ABI prelude. Existing test function
names, assertions, scripts, and ignore attributes are preserved; fully qualified
Rust test paths now include the responsibility module.

[tests/support/process.rs](../tests/support/process.rs) extracts the existing CLI
subprocess lifecycle into one private test-only implementation, included by the
library test build and integration test targets. Scalar, memory, export, numeric
helper, CLI, runtime, browser, array-equality, and remainder tests all use it.
Specialized builders, decoders, JavaScript scripts, and success/error assertions
remain local rather than becoming a generic test framework.

The harness drains stdout and stderr concurrently while writing optional stdin,
closes ordinary stdin after writing, and retains the CLI's explicit held-open
stdin mode. It polls a deadline, kills and reaps a timed-out direct child, and
uses a guard for cleanup on unwinding. Early child exits tolerate broken pipes
without hiding exit status; timeout diagnostics include the command, deadline,
and both captured output streams. CLI/runtime/export/memory/scalar/numeric-helper
cases use 15-second deadlines; browser/array-equality/remainder cases retain their
30-second deadlines. These are test-process deadlines, not guest execution fuel
or compiler resource limits; the harness does not manage descendant process trees.

[tests/process.rs](../tests/process.rs) exercises pipe-pressure handling, stdin EOF,
held-open stdin, nonzero exits, early exits during a large write, timeout output,
blocked-writer cleanup, and spawn diagnostics using the Rust test executable as
its own child fixture. These tests require no Node installation. Existing Node
ignore attributes and every end-to-end CLI/provider and raw-backend case remain.
No compiler semantics, public APIs, generated bytes, or optimization behavior
change as part of this refactor.

## 2. ABI constants and layout ownership: implemented

[abi.rs](../src/codegen/abi.rs) owns language-to-Wasm value/signature lowering and
private constants for descriptor size/field offsets/alignment, numeric and string
element sizes, numeric access alignment, reserved low addresses, and allocation
alignment/minimum size. [Memory planning/helpers](../src/codegen/memory.rs),
[array emission](../src/codegen/values.rs), and indexed loads in
[expression.rs](../src/codegen/expression.rs) use these definitions. Page sizes and
memory limits remain owned by memory code, with the page shift derived from the
page size. Local indices, branch depths, and unrelated numeric values are not
replaced with ABI constants.

`Helper::ALL` in memory code is the single canonical helper ordering. Helper
indices derive from it, helper signatures use an exhaustive match, and both
[layout planning](../src/codegen/layout.rs) and
[module emission](../src/codegen/mod.rs) consume that ordering. Independent tests
pin the seven existing indices/signatures; the optional remainder helper still
follows users and any memory helpers.

[runtime/abi.cjs](../runtime/abi.cjs) separately names its private descriptor and
element-layout constants. There is no generated cross-language schema or exported
configuration surface. Independent host fixtures and guest/host roundtrips check
agreement, rather than making tests depend on production definitions. `createAbi`
remains opt-in; raw callbacks/exports do not acquire automatic marshaling.

[tests/abi.rs](../tests/abi.rs) compares native library output at all optimization
levels and CLI output with a complete pre-refactor binary fixture, and checks
literal little-endian static descriptor/payload bytes and raw/explicit host
roundtrips. [tests/runtime.rs](../tests/runtime.rs) adds host-only golden bytes,
boolean conversions, empty/Unicode/NUL strings, numeric-array copy isolation, and
view refresh after external or allocator-triggered growth. Existing bounds,
copy-limit, error-stage, aliasing, and runtime tests remain.

The fixture [memory.wasm](../tests/fixtures/abi/memory.wasm) was captured from the
pre-refactor native CLI for [memory.was](../tests/fixtures/abi/memory.was), with
`echo_array=env.echo_array(number[])->number[]`,
`echo_string=env.echo_string(string)->string`, and
`echo_bool=env.echo_bool(bool)->bool` registered in that order, default exports,
and no optimization. Do not regenerate it merely to make a refactor pass: changed
bytes require an explicit compatibility decision.

No representation, helper ordering, generated bytes, host contract, ownership
policy, copy guards, error messages/stages, or optimization behavior changes. No
runtime type tags or zero-copy conversion aliases were introduced.

## 3. A modest body-emitter facade and typed indices: implemented

[`BodyEmitter`](../src/codegen/function.rs) privately owns the encoded function
body and its context. Statement, expression, call, value, and control-flow
lowering now receive one mutable emitter plus the existing enclosing statement
span rather than separate body/context references. Focused modules still own
numeric/string/array lowering, call validation, statement return analysis, and
control-flow policy; no monolithic emitter or new IR was introduced.

The facade provides typed local reads/writes and calls, module/helper lookup,
boolean normalization, scoped binding restoration, and thin structured
block/loop/if/branch operations. Call-target references borrow the immutable
module layout for its lifetime, allowing recursive argument emission without
cloning signatures. The private context keeps the existing initialized-binding
set and `Cell<u32>` temporary cursor. Scopes restore bindings on success and
returned errors but deliberately do not rewind temporary allocation; scope
restoration on panic is not a new guarantee.

[`indices.rs`](../src/codegen/indices.rs) defines private `FunctionIndex`,
`TypeIndex`, `LocalIndex`, and `LabelDepth` wrappers. Layout plans and call targets
carry typed function/type indices, local lookup returns a typed local index, and
conversion to raw `u32` happens at encoder boundaries. Raw count/overflow checks
remain before construction. `LabelDepth::LOOP_EXIT` and `LOOP_BACK_EDGE` retain
the existing relative depths 1 and 0; these names describe the existing immediate
block/loop pair, not arbitrary labels or new break/continue support. Memory-helper
bodies keep their separate low-level emission; no general label allocator was added.
The wrappers complement existing semantic IDs instead of replacing them.

[`tests/emitter.rs`](../tests/emitter.rs) compares all optimization levels against
[`control.wasm`](../tests/fixtures/emitter/control.wasm), captured before this
refactor from [`control.was`](../tests/fixtures/emitter/control.was) with
`marker=env.marker(number)->number` and `signal=env.signal(bool)->bool` registered
in that order, default exports, and no optimization. It checks immutable typed
input, export-only section changes, nested loops/shadowing, forward/mutual calls,
short-circuit effects, canonical host booleans, live operand prefixes, nested
array temporaries, and host-effect order. Private facade tests verify restoration
on returned errors without temporary reuse, and distinct function indices with
shared type indices. Existing sparse-ID, malformed-tree, indexed-assignment,
nested-control-flow, and ABI fixture tests remain; local-index assertions unwrap
only at comparison with their independent raw expectations.

No public API, byte output, local allocation order, opcode selection,
normalization point, return analysis, branch depth, diagnostic precedence/span,
or optimization behavior changes. Golden fixtures are compatibility evidence,
not outputs to regenerate automatically when a refactor fails.

## 4. One structural walker, separate planning policies

`MemoryPlan::visit_block`/`visit_expr` and
`temporaries::visit_block`/`visit_expr` repeat recursive matches for the same typed
statements/expressions. `LocalLayout::collect` independently descends nested
blocks to collect declarations. A new AST variant currently requires updating
these traversals separately.

Add a private, fallible structural walker with exhaustive matches and a small
visitor interface: statement/expression enter hooks and explicit block entry/exit
where needed. Walk source preorder, left operand before right, arguments/elements
in order, and condition before branch/body. Keep the three planning policies
separate: memory need/string interning, declaration collection, and array-literal
temporary counting. The local collector can ignore expression hooks rather than
inventing another AST representation.

Do not make code emission or semantic analysis blindly use that walker. Their
scope, validation precedence, short-circuit lowering, and return rules have
meaning beyond structural traversal. A planning walk still visits unreachable
code and both logical operands; runtime execution does not necessarily do so.
The walker must not accidentally change which planning error is reported first.

Characterize signature-driven memory inclusion, literal interning preorder,
branch/loop declaration order, nested arrays, unused memory-backed imports, and
unreachable literals before migrating one planner at a time. Keep each planner's
results identical; sharing recursion is the objective, not combining passes or
reducing visits.

## 5. An explicit validated typed-program boundary

[typed_ast.rs](../src/typed_ast.rs) exposes mutable fields, IDs, and type annotations.
Semantic analysis creates well-formed trees, but library callers can forge them.
The backend deliberately checks many malformed trees. Meanwhile
`Context::helper`, `Context::string`, and `Context::temporary` in
[function.rs](../src/codegen/function.rs) use `expect`/`assert` for invariants
established by planning. Distinguish invalid external input from an internal
planning defect instead of promising that any public tree is safe to emit.

Introduce an additive validated-program view or checked constructor boundary.
Validate function/host references, signatures/types, per-function bindings/scopes,
return contracts, and planner preconditions through existing checks wherever
possible. Keep the original program and owned host snapshot; a wrapper need not
clone or mutate the AST. An immutable borrowed checked view prevents subsequent
mutation for its lifetime.

Do not immediately privatize all public fields, remove legacy compile entrypoints,
or delete backend defenses. Start with an internal adapter, then expose an
optional checked API after behavior is characterized. Any future API restriction
needs an explicit compatibility/deprecation decision. Malformed-tree tests remain
valuable and can continue targeting the legacy/raw validation boundary.

Moving checks earlier can change first-error precedence. Compare legacy and
checked paths on both valid and multiply-invalid trees before adopting a new
path by default. Preserve statement-wide spans and errors for unreachable code.
Keep private assertions only for genuinely internal defects, or return a precise
internal error where recovery is meaningful; do not replace them with unchecked
emission. A discovered crash fix should be isolated from the API restructuring.

## 6. Shared span access and configuration contracts

### Diagnostics: refactor first, enhancement separately

`Span` in [tokens.rs](../src/tokens.rs) stores byte offsets. Parser, semantic, and
code-generation errors carry locations in different variants; `CompileError`
wraps stage errors. The shared library module [diagnostic.rs](../src/diagnostic.rs)
serves CLI and browser presentation with source excerpts, UTF-8-safe clamping, CRLF/tab
handling, and safe EOF/malformed-span rendering. It currently extracts locations
through its own `parse_span`, `semantic_span`, and `codegen_span` matches; the
code-generation fallback maps other variants to `None`.

Add a common `span() -> Option<Span>` accessor to stage errors and delegate through
`CompileError`, then make the existing renderer consume it. Keep extraction
ownership near error definitions so a new located variant cannot silently lose
its location through the renderer's wildcard fallback. Preserve `None` for
unlocated registration/layout errors, variants/messages, and first-error order.
Do not invent another renderer or move presentation into `wasm_encoder`/ABI code.
Retain existing UTF-8/CRLF/tab/EOF tests and add accessor coverage for located and
unlocated variants. Preserve character-column and four-space excerpt-tab behavior;
terminal/grapheme-width changes require separate diagnostic acceptance criteria.

Expressions in [ast.rs](../src/ast.rs) and [typed_ast.rs](../src/typed_ast.rs) still
lack independent spans, so analysis/emission pass an enclosing statement location.

Adding expression spans is useful **diagnostic enhancement**, not a completely
behavior-preserving refactor: it changes public node construction and reported
locations. Plan it separately after common access/rendering is stable, retain an
enclosing-statement fallback, and deliberately update span assertions. Multiple
errors/recovery and Wasm debug/source-map sections are also separate features.

### Library compilation versus CLI artifacts

The split is already implemented: [main.rs](../src/main.rs) is a thin entrypoint;
[cli.rs](../src/cli.rs) owns parsing, host declarations, source loading, compiler
orchestration, format selection, and output writes;
[diagnostic.rs](../src/diagnostic.rs) owns shared CLI/browser error presentation.
[browser/src/lib.rs](../browser/src/lib.rs) exposes the pipeline through
`wasm-bindgen` without filesystem or CLI dependencies.
[compiler.rs](../src/compiler.rs) remains the library pipeline,
and [codegen/mod.rs](../src/codegen/mod.rs) emits raw bytes. Preserve this split
rather than recommending another extraction of these modules.

Codify the small configuration contract at existing boundaries: `clap` derives
syntax parsing, help, and usage errors from `Arguments`, which normalize into CLI
`Options`. Keep host/compiler validation separate from syntax parsing. `Options`
resolve through `output_format`; explicit format overrides the exact lowercase
`.wasm` inference; binary requires a file and publishes raw compiler bytes.
Strengthen focused mapping tests so adding an option cannot silently change format,
stdout, or output policy. A small normalized policy value/helper is sufficient
if reuse requires it; do not introduce a new configuration hierarchy just for
cleanup.

Keep the owned [ExportOptions](../src/exports.rs) separate from `Copy`
`CompilerOptions`. Layout validates selection and preserves body/import indices;
the CLI uses the same policy and raw compiler output as the library. Share a
narrow selection predicate only if useful, without bypassing layout validation
or changing error precedence. Tests should compare CLI and library binary bytes
and check every registered import from the typed snapshot.

The existing [CLI tests](../tests/cli.rs) are contract fixtures for required source
paths, literal `-` filenames, flags, exit statuses, located/unlocated diagnostics,
raw binary output, canonical same-input refusal, parent-directory policy,
and output preservation/temporary cleanup. Retain them through restructuring;
atomic replacement is not a reason to promise crash durability or race-proof
file identity. Preserve the distinction between delimiter-limited CLI host
syntax and the Rust API's arbitrary nonempty module/field strings.

[runtime/run.cjs](../runtime/run.cjs) already separates argument/provider loading
from `instantiate` and `createAbi`. Keep both integration targets: actual
CLI-to-Node raw binaries and independent raw-library/descriptor fixtures.
Provider execution remains runtime work, not compiler host registration.

## Rollout and verification

1. Capture exact bytes, decoded sections/instructions, error variants/spans,
   static handles, and observable effects for each targeted area.
2. Move/extract a single responsibility while retaining the old public boundary.
   Use transitional differential tests where two paths exist; remove redundant
   implementation paths only after those tests agree.
3. Run focused tests, then `cargo test --offline`. With Node.js available, run
   `cargo test --offline -- --include-ignored` for runtime behavior. Default test
   success alone does not cover ignored runtime cases.
4. Run the shipped M7 suites with Node enabled:
   `cargo test --offline --test cli -- --include-ignored` and
   `cargo test --offline --test runtime -- --include-ignored`. Preserve copied
   explicit array conversions versus raw aliases, raw calls/imports,
      format/status/diagnostic cases,
   and filesystem-output safety assertions.
5. Keep source/ABI/output compatibility fixtures independent enough to detect a
   shared implementation mistake. Validate Wasm with `wasmparser`, but also execute
   it: structural validity does not prove the intended results or side effects.

No refactor in this plan is intended to change emitted code or language/runtime
behavior. Changes to literal syntax, remainder support, GC/freeing, resource
quotas, new control-flow constructs, asynchronous hosts, or ABI compatibility
policy belong in separately scoped feature/bug-fix work with explicit acceptance
criteria.
