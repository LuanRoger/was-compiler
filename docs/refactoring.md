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
| 1 — high | Split large backend tests; align subprocess support with the existing bounded harness | [tests.rs](../src/codegen/tests.rs), [memory_tests.rs](../src/codegen/memory_tests.rs), [tests/cli.rs](../tests/cli.rs), [tests/runtime.rs](../tests/runtime.rs) | Same coverage and ignored-runtime policy; bounded child cleanup and useful stdout/stderr. |
| 2 — high | Name ABI constants and strengthen cross-language layout fixtures | [abi.rs](../src/codegen/abi.rs), [memory.rs](../src/codegen/memory.rs), [values.rs](../src/codegen/values.rs), [runtime/abi.cjs](../runtime/abi.cjs) | Binary/layout fixtures unchanged; independent raw-ABI and explicit-conversion roundtrips agree. |
| 3 — medium | Add a small function-body emission facade and index/label types | [function.rs](../src/codegen/function.rs), [expression.rs](../src/codegen/expression.rs), [control_flow.rs](../src/codegen/control_flow.rs), [layout.rs](../src/codegen/layout.rs) | Exact instruction/index output plus nested control-flow/live-stack tests unchanged. |
| 4 — medium | Share structural typed-AST walking for planning | [memory.rs](../src/codegen/memory.rs), [locals.rs](../src/codegen/locals.rs), [temporaries.rs](../src/codegen/temporaries.rs) | Same preorder, static handles, declaration order, temporary count, and planning error order. |
| 5 — medium | Define a validated typed-program boundary | [typed_ast.rs](../src/typed_ast.rs), [semantic.rs](../src/semantic.rs), [compiler.rs](../src/compiler.rs), backend malformed-tree tests | Checked/legacy paths agree on bytes and diagnostics; no unchecked bypass hidden by the new API. |
| 6 — medium | Add common span accessors and codify configuration/output contracts | [tokens.rs](../src/tokens.rs), [parser.rs](../src/parser.rs), [compiler.rs](../src/compiler.rs), [cli.rs](../src/cli.rs), [diagnostic.rs](../src/diagnostic.rs) | Existing module split, errors/rendering, raw-byte compatibility, CLI formats/statuses, and output safety preserved. |

All starting points are present. Prefer small reviewable changes that preserve
existing characterization tests and add focused coverage before each move.

## 1. Smaller test modules and one subprocess harness

[tests.rs](../src/codegen/tests.rs) combines section decoding, typed-tree builders,
layout/lowering diagnostics, scalar operations, calls, control flow, and Node
execution in one large module. [memory_tests.rs](../src/codegen/memory_tests.rs)
contains another decoder, typed-tree helpers, and a JavaScript ABI prelude.
Both spawn Node and stream Wasm through stdin; the memory harness has a 15-second
poll/kill deadline, while the scalar harness waits without a deadline.

Move tests by responsibility without rewriting assertions: module layout/ABI,
numeric/boolean expressions, locals/calls, control flow, memory, and runtime
behavior. Keep forged typed-AST cases clearly separated from valid-source cases.
Extract only genuinely common builders/section-reading helpers into private test
support; do not force specialized instruction decoders into a generic framework.

[tests/cli.rs](../tests/cli.rs) already has a bounded `process_output` helper with
concurrent stdout/stderr draining, stdin writing, and guarded child cleanup.
[tests/runtime.rs](../tests/runtime.rs) has a separate Node subprocess helper.
Use the existing integration harness as the reference for consistent lifecycle
policy instead of inventing another harness. Adopt bounded cleanup in scalar
backend tests and share only genuinely reusable mechanics where Rust test-target
boundaries permit it. Keep specialized scripts and assertions independent.
A deadline protects the test process, not generated guest semantics. Preserve
`#[ignore = "requires Node.js"]` and inclusion of every existing runtime case.

[export_tests.rs](../src/codegen/export_tests.rs) adds selection-focused section
decoding and runtime cases; include it when identifying common test support.
Keep end-to-end CLI/provider tests separate from raw backend tests: the former
exercise argument/file/provider integration; the latter independently instantiate
raw library output. Split large modules by topic
only when their helpers remain easy to locate, not into a general test framework.

## 2. ABI constants and layout ownership

[abi.rs](../src/codegen/abi.rs) already centralizes language-to-Wasm value/signature
lowering. Keep that ownership. Descriptor header size, field offsets, element
width, and alignment appear as literals in
[memory planning/helpers](../src/codegen/memory.rs),
[array emission](../src/codegen/values.rs), [runtime/abi.cjs](../runtime/abi.cjs),
test JavaScript, and [host documentation](memory-abi.md). Helper IDs/signatures also depend on a shared
order between memory emission and [layout.rs](../src/codegen/layout.rs).

Introduce narrowly named internal constants or a compact layout description for
those existing ABI facts. Use it in planning and emission without changing
bytes, helper ordering, or representation. Do not add runtime type tags or a new
ownership policy as part of this extraction.

Explicit native-value conversion is already available through `createAbi` in
[runtime/abi.cjs](../runtime/abi.cjs). Preserve that optional boundary rather than
adding automatic marshaling to raw export calls or host callbacks. Test boolean
conversion, empty/Unicode strings, numeric-array copies, raw host return handles,
and memory growth/view refresh. Keep copy guards and error stages unchanged;
do not turn explicit decoded copies into zero-copy aliases.

Keep independent golden expectations for the eight-byte descriptor and raw memory
roundtrips. If every expected offset is taken from production constants, a wrong
constant can make both implementation and tests agree incorrectly. Compare CLI
and library bytes for matching source/options, and test both raw runtime calls
and explicit conversions separately from structural Wasm validation.

## 3. A modest body-emitter facade and typed indices

Emission currently threads `&mut wasm_encoder::Function`,
`function::Context`, and an enclosing statement `Span` through
[statement.rs](../src/codegen/statement.rs),
[expression.rs](../src/codegen/expression.rs),
[call.rs](../src/codegen/call.rs),
[values.rs](../src/codegen/values.rs), and
[control_flow.rs](../src/codegen/control_flow.rs).
`Context` owns local resolution, initialized bindings, module/helper lookup, and
a `Cell<u32>` temporary cursor. Function/type/local/helper indices are largely
plain `u32`; loop exits/back edges use literal relative depths `1`/`0`.

Wrap the body and context in a private `BodyEmitter` facade, initially delegating
to existing functions. Add only operations that make current responsibilities
clear: local reads/writes, target/helper lookup, boolean normalization, scoped
binding restoration, and structured control emission. Keep numeric/string/array
lowering in focused modules instead of creating a monolithic emitter class.

Introduce small internal newtypes for Wasm function, type, and local indices at
lookup boundaries. A minimal structured-label/depth type can express the
existing loop exit/back edge without expanding control-flow functionality.
Convert to encoder `u32` arguments at emission. This complements, rather than
replaces, the existing `FunctionId`, `HostFunctionId`, and `VariableId` types.

Acceptance cases include deduplicated signatures with different function indices,
forward/mutual calls, sparse variable IDs, shadowing, nested loops, logical
branches with live stack prefixes, and checked indexed assignment. Do not change
local allocation, opcode selection, normalization points, return analysis, or
branch depths merely to simplify the facade. Temporary-cursor ownership can move
only when the same planned slots are consumed in the same order.

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
