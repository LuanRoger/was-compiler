# Memory evolution design interview

Status: all interview decisions settled; awaiting final confirmation of the
consolidated design. This document is not an implementation authorization or a
change to current language/ABI contracts.
Current behavior remains specified in [the memory ABI](memory-abi.md).

The conversation was synthesized into [the feature specification](memory-evolution-spec.md)
and published as [GitHub issue #37](https://github.com/LuanRoger/was-compiler/issues/37)
with `ready-for-agent`. Publication does not start compiler implementation.

## Confirmed direction — 2026-10-15

- Preserve the distinction between fixed-length mutable shared arrays and proposed
  growable lists. Adding element types does not introduce resizing operations.
- Prioritize extensibility, allocation-pressure measurements, and focused workloads
  over an unmeasured promise of speed or reduced resident memory.
- Breaking changes are permitted when necessary and justified. Compatibility is
  not an absolute requirement, but unnecessary breaks are not a goal. The agreed
  descriptor ABI is retained; the Rust type-model migration is explicit below.
- Target `bool[]` first, then `string[]`. Evaluate the design against nested arrays
  such as `number[][]`, but defer their implementation.
- Prepare for future optimization without authorizing allocation elimination,
  check elimination, reclamation, or WP13 transformations in the initial scope.
- Do not change compiler implementation until the complete design is confirmed.

## Confirmed element contracts — round 2

These are proposed contracts for the planned extension, not implemented support.

- `bool[]` uses one byte per element, canonically `0` or `1`; scalar Wasm
  booleans remain `i32`. Bit packing is outside this milestone.
- `string[]` uses four-byte handles to immutable strings. Updating an array element
  replaces its reference, not string contents. No reference counting is introduced.
- Array entry validation checks descriptor and complete payload bounds, without an
  eager scan of all elements. Checked element access validates a consumed value;
  typed writes validate the value before storing it. Equality and decoding check
  the elements they inspect. Failure ordering is specified in round 4 below.
- New arrays use same-length, element-value equality: booleans compare values and
  strings compare contents. Different array element types do not compare. Existing
  numeric equality retains IEEE-754 behavior. Nested equality remains undecided.
- An available expected array type determines the type of `[]`; an unconstrained
  `[]` remains `number[]`. Propagation and inference limits are specified below.
- No implicit conversions between array types are introduced.

## Confirmed type and integration contracts — round 3

- Replace the dedicated `Type::NumberArray` shape with a closed explicit element
  model, conceptually `Type::Array(ArrayElementType::{Number, Bool, String})`.
  Retain a small `Copy` type model; document the Rust API migration. This does not
  introduce generics, recursive type representation, `range[]`, or `void[]`.
- Retain untagged eight-byte pointer/count descriptors and eight-byte descriptor
  and allocator alignment. Minimum payload alignment is eight for numeric elements,
  one for boolean elements, and four for string handles. Array lengths count elements.
  Static types or explicit host types determine payload interpretation; bounds checks
  cannot reliably detect a host deliberately passing the wrong array type.
- Each type must cover internal signatures, locals, returns, captures, host imports,
  selected exports, CLI host declarations, browser host declarations, and explicit
  Node marshaling before release. Preserve external range restrictions.
- Node encoding accepts plain JavaScript arrays of booleans or strings respectively;
  encode/decode copy values. Raw memory access remains the shared-storage option.
- Generalize literals, indexing, replacement, `len`, equality and `for`. Iteration
  bindings have the element type and remain read-only. Numeric compounds remain
  numeric-only; no resizing, slicing, concatenation, or chained indexing is added.
- Known destinations supply expected types: annotations, assignment targets, known
  call parameters, and explicitly declared named-function results. Anonymous
  callable results remain inferred without new annotation syntax. Propagate through
  parentheses and value-producing block/if/match/loop paths. No sibling-driven
  inference or whole-program unification: unconstrained `[]` remains `number[]`.
- Consuming a boolean array byte other than `0` or `1` traps instead of normalizing
  it. Preserve existing scalar host-boolean normalization independently.

The layout/validation rationale is recorded in
[ADR 0006](adr/0006-typed-array-layout-and-lazy-element-validation.md).

## Confirmed execution and implementation direction — round 4

- Literals allocate and initialize the outer descriptor before evaluating elements;
  each element evaluates once in source order and is validated before its store.
  Traps retain completed effects and allocations. Discarded literals still allocate.
- Ordinary replacement checks and retains the destination address before RHS
  evaluation, then validates and stores the new value without reading the old one.
  Numeric compounds retain their existing old-value-before-RHS semantics. Saved
  Wasm offsets survive growth; host views must be refreshed independently.
- Equality evaluates both operands, validates both complete outer descriptors/payloads,
  checks lengths, then compares pairs in increasing index order, validating left
  then right consumed values. It stops at the first mismatch and has no whole-array
  identity shortcut. Unvisited invalid elements need not trap.
- `len` validates only the outer descriptor/payload. Iteration caches the iterable
  handle/count, retains checked live descriptor access per yield, validates the
  yielded value before binding/body effects, observes alias mutation, and does not
  inspect elements skipped by transfer.
- Centralize checked sizing/addressing, element layout, typed loads/stores, consumed
  value validation, and comparison behavior behind private statically selected
  operations. Keep raw allocation, capture storage, and host marshaling distinct.
  Do not add an IR, runtime type registry, or general allocator framework.
- Measure aligned bump consumption, initial/final memory pages, module size,
  compiler timings, and guest-call timings separately on bounded workloads.
  Allocation markers must not perturb page-growth or timing runs. Memory pages are
  capacity, not live bytes or RSS; establish baselines before performance targets.
- Workloads cover construction, aliasing, indexed access/replacement, iteration,
  early/late equality exits, strings, captures, and page crossings. Reuse existing
  benchmark/test support where appropriate, without production profiling exports.

## Confirmed language interactions and compatibility — round 5

- Preserve the defaults/contracts pure whitelist. Extend existing array-reference
  reads, equality and `len` to the new types; keep literals and indexing rejected,
  including in skipped expressions. Contextual typing grants no whitelist admission.
- Pipeline inputs remain independently analyzed before stage lookup/binding. An
  unconstrained `[] |> takes_bools` is rejected as a numeric-array input; annotated
  boolean arrays work. Do not infer backward through pipeline stages. Ordinary
  written call arguments receive their mapped formal type, not written-position type.
- Unconstrained nonempty literals infer one exact supported element type from
  normally completing elements. Reject mixed types. Empty/all-Never literals retain
  numeric fallback unless a known destination supplies the element type. Expected
  types never turn Never into a stored value.
- Omitted-result functions and anonymous callables returning unconstrained `[]`
  infer `number[]`. Final rechecking must not retroactively contextually reinterpret
  inferred literals or introduce sibling-driven inference. Anonymous callable result
  annotations remain unsupported.
- Preserve helper emission, indices and exact golden bytes for old-type-only
  programs unless a concrete requirement necessitates an explicitly approved break.
  Emit new-type helpers deterministically only when required by the new types;
  do not perform general helper pruning or regenerate fixtures to hide drift.
- Each new-array Node encode/decode operation has a 64 MiB aggregate logical-copy
  budget: one byte per boolean; four bytes per string element plus UTF-8 bytes for
  every copied occurrence. Repeated occurrences count repeatedly. This is not a
  JavaScript object-overhead/RSS bound. Preserve existing string/number-array limits,
  use overflow-safe accounting and refresh views after allocations.

## Existing constraints and decisions to revisit deliberately

- Arrays alias mutable storage; every evaluated array literal allocates freshly.
- Raw allocation, typed storage operations, and host marshaling have distinct roles.
- Existing assignment checks precede RHS effects; repeated iteration descriptor
  checks, host reentry, memory growth, recursion, and traps constrain future work.
- [ADR 0001](adr/0001-private-range-representation.md) defers allocation-free range
  representations. [ADR 0004](adr/0004-local-callable-ownership-and-storage.md)
  explains why closure storage is monotonic and reclamation is deferred.
- The existing descriptor carries a pointer and length, not an element type tag.
  The current array implementation is specialized to eight-byte numeric elements.

## Confirmed delivery and acceptance — round 6

### Ordered milestones

| Milestone | Deliverable | Exit condition |
| --- | --- | --- |
| M0 — Baseline | Bounded memory workloads, allocation/page measurements, output and timing records | Reproducible evidence before implementation changes |
| M1 — Typed array foundation | Explicit element model and shared checked operations, initially exercised by `number[]` | Existing behavior, module fixtures and optimization parity preserved |
| M2 — Boolean arrays | End-to-end language, contextual literals, captures, adapters and marshaling | Full boolean acceptance/rejection and runtime contracts pass |
| M3 — String arrays | Handle elements, lazy checks, content equality, adapters and copy budgets | Full string contracts pass, including unconsumed invalid elements |
| M4 — Release and optimization handoff | Final measurements, migration docs, full validation and ranked follow-up opportunities | Implemented improvements distinguished from deferred work |

Compiler planning stays sequential. Independent tests/adapters may overlap only
once shared interfaces are fixed and exclusive file scopes assigned; retain the
[extension-plan ownership rules](was_language_extensions_plan.md#parallel-development-and-ownership).
Do not introduce empty future scaffolding merely to divide work.

### Migration policy

- Migrate Rust consumers directly from `Type::NumberArray` to the explicit array
  representation; do not retain a deprecated compatibility spelling.
- Update public typed-tree consumers, constructors, examples and downstream contract
  tests. Document the breaking Rust API change without changing `number[]` source
  syntax or existing host descriptor bytes.
- Preserve error variants/messages/spans and first-error ordering for unchanged
  behavior, except where the agreed contextual-typing extension intentionally
  changes acceptance. Document necessary exceptions explicitly.
- Preserve the portable compiler/CLI/browser ownership established in
  [ADR 0005](adr/0005-compiler-and-adapter-workspace-boundaries.md). Its API-preservation
  rule applied to the workspace refactor; this separate extension explicitly permits
  the type-model migration, not package-boundary changes.

### Required release evidence

- Frontend acceptance/rejection: syntax, inference, element types, scope,
  defaults/contracts, named calls, pipelines, captures, empty/mixed/all-Never
  literals, unreachable statements and runtime-skipped expressions.
- Independent backend rejection of forged typed nodes, annotations and storage
  types; preserve checking even where expressions do not complete.
- `wasmparser::Validator` plus independent decoded layout/instruction assertions.
  Runtime coverage checks aliasing, replacement order, lazy checks, equality exits,
  boolean bytes, iteration captures, growth, reentry and effects retained after traps.
- Determinism, selected exports, host snapshots, optimization-level parity and
  unchanged old-type golden fixtures unless a specific break is approved.
- CLI/core parity, host roundtrips, copy budgets and rebuilt-current-source browser
  parity; native checks, portable core Wasm build, architecture checks and Clippy.
- Explicit ignored runtime/browser execution after rebuilding bindings. Missing
  offline dependencies/tooling leave gates unverified, not passed. Follow the finite
  commands and ordering in [validation](validation.md); retain bounded subprocess
  support and existing ignored-test policy.
- Update grammar, supported types, ABI/runtime/browser guides, limitations and
  `AGENTS.md` as implementation lands, never presenting planned support as current.
- Record allocation bytes, capacity pages, module sizes and timings separately.
  Follow [benchmark timing rules](benchmarks.md); no speedup claim without repeated
  comparable runs, and no invented percentage gate. Record workload/toolchain/
  machine/revision details. Do not change the existing compiler timing boundaries
  when adding separate guest measurements.

## Final confirmation and explicit exclusions

No design decision remains open within this scope. Final confirmation of the
consolidated design is still pending and does not itself authorize implementation.
A separate explicit implementation request is required.

Nested arrays, growable lists, range/void arrays, slicing, chained indexing,
concatenation, first-class callables, closure result annotations, bit packing,
garbage collection, reference counting, heap rewind, allocation/check elimination,
and WP13 transformations remain excluded. Existing monotonic allocation can still
exhaust memory in long-lived instances; this effort does not claim to solve that.
Future optimization opportunities must be ranked using the new evidence and state
their required aliasing, lifetime, host-effect and trap-order proofs.
