## Problem Statement

WAS developers can currently use only fixed-length numeric arrays. Boolean and string collections require unsupported representations or workarounds, and numeric element assumptions are spread across typing, allocation sizing, indexing, mutation, iteration, equality, and host integration. Adding an element type therefore requires coordinated special cases instead of using a coherent typed-array foundation.

The current monotonic allocator also makes allocation pressure important for long-running instances, but existing compiler benchmarks do not measure guest allocation consumption or execution. Without separate, reproducible measurements, extensibility improvements can be mistaken for memory savings or performance optimizations.

We need a maintainable foundation for additional array element types, consistent language and host contracts, and evidence that can guide future optimizations without prematurely introducing reclamation or changing observable behavior.

## Solution

Introduce a shared, statically selected typed-array implementation, first preserving numeric arrays and then delivering boolean arrays and string arrays end-to-end. Arrays remain fixed-length, mutable shared collections; growable lists are a separate future feature.

Retain the existing pointer/count descriptor ABI. Numeric elements occupy eight bytes, boolean elements one canonical byte, and string elements four-byte references to immutable strings. Validate complete array storage structurally, then validate element values when consumed. Preserve evaluation order, aliasing, mutation visibility, trap timing, and completed effects.

Establish bounded measurements of aligned allocation consumption, Wasm memory capacity, module size, compiler timings, and guest execution separately. Deliver the work through baseline, numeric-foundation, boolean-array, string-array, and release milestones. Use the results to rank future optimization opportunities without implementing allocation/check elimination or reclamation in this effort.

## User Stories

1. As a WAS developer, I want boolean arrays, so that I can represent collections of flags without encoding them as numbers.
2. As a WAS developer, I want string arrays, so that I can represent collections of text directly.
3. As a WAS developer, I want array literals to infer a single supported element type, so that ordinary declarations remain concise.
4. As a WAS developer, I want an explicit array destination to type an empty literal, so that I can create empty boolean and string arrays.
5. As an existing WAS developer, I want unconstrained empty literals to remain numeric arrays, so that existing source keeps its established meaning.
6. As a WAS developer, I want mixed-element arrays rejected without coercions, so that storage types remain predictable.
7. As a WAS developer, I want typed indexed reads, so that an array access produces its actual element type.
8. As a WAS developer, I want to replace boolean and string elements, so that fixed-length collections remain useful mutable data structures.
9. As a WAS developer, I want assignment and parameter passing to share array storage, so that existing aliasing semantics apply consistently to every supported element type.
10. As a WAS developer, I want every evaluated array literal to create fresh storage, so that separate constructions do not accidentally alias.
11. As a WAS developer, I want length queries for every supported array type, so that collection size is available uniformly.
12. As a WAS developer, I want array iteration to yield the element type, so that boolean and string collections can use existing control-flow constructs.
13. As a WAS developer, I want iteration bindings to remain read-only while inner shadowing bindings remain ordinary locals, so that mutability rules stay consistent.
14. As a WAS developer, I want iteration to observe mutations to later elements through aliases, so that iteration does not silently snapshot mutable storage.
15. As a WAS developer, I want array equality to compare element values, so that different allocations with equal contents compare equal.
16. As a WAS developer, I want string-array equality to compare string contents rather than handles, so that allocation identity does not change value equality.
17. As an existing WAS developer, I want numeric equality to retain IEEE-754 behavior, so that NaN, signed zero, and infinity cases do not regress.
18. As a WAS developer, I want boolean and string arrays in parameters, returns, locals, blocks, branches, matches, and value loops, so that new types compose with existing expressions.
19. As a WAS developer, I want ordinary and named call arguments to receive their mapped parameter types, so that contextual literals work without changing argument order.
20. As a WAS developer, I want pipeline inputs to retain independent typing, so that later stages do not unexpectedly reinterpret earlier expressions.
21. As a WAS developer, I want omitted-result functions and anonymous callables to infer results consistently across analysis passes, so that rechecking does not change literal meaning.
22. As a WAS developer, I want arrays captured by local callables to retain shared storage and existing lifetime behavior, so that new collections compose with closures.
23. As a WAS developer, I want array operations admitted by the existing defaults/contracts whitelist to work for new element types, so that contracts and defaults remain useful without broadening their purity rules.
24. As a WAS developer, I want destination and bounds checks before replacement RHS effects, so that invalid assignments fail in a predictable order.
25. As a WAS developer, I want ordinary replacement not to read the old element, so that overwriting an invalid element does not unnecessarily consume it.
26. As an embedding developer, I want malformed consumed boolean bytes and string handles to trap, so that invalid values do not silently enter language operations.
27. As an embedding developer, I want parameter and length validation to avoid scanning all elements, so that structural validation does not impose linear element-validation work.
28. As an embedding developer, I want equality and iteration to validate only visited elements, so that early exits have explicit, testable behavior.
29. As an embedding developer, I want boolean and string arrays in registered imports and selected exports, so that new types work across the host interface.
30. As a CLI user, I want host declarations for the new array types, so that command-line compilation exposes the same language support as the library.
31. As a browser integrator, I want current-source browser support for new array signatures and diagnostics, so that browser and native compilation remain compatible.
32. As a Node integrator, I want explicit encoding and decoding of plain arrays of booleans and strings, so that host roundtrips do not require manual descriptor construction.
33. As a Node integrator, I want encoding and decoding to copy values, so that native mutations do not accidentally mutate guest arrays.
34. As a Node integrator, I want aggregate copy limits and refreshed memory views, so that large string collections and memory growth are handled deliberately.
35. As a Rust library consumer, I want a documented migration from the numeric-only array type representation, so that the public API change is clear and actionable.
36. As a compiler maintainer, I want one checked element-operation model, so that element widths, loads, stores, and validation rules do not drift between array operations.
37. As a compiler maintainer, I want malformed public typed trees rejected independently of source analysis, so that backend safety checks survive the type-model change.
38. As an existing embedding developer, I want unchanged descriptor bytes and deterministic legacy modules where no break is required, so that adding new types does not unnecessarily invalidate integrations.
39. As a compiler maintainer, I want bounded allocation and capacity workloads, so that memory pressure is measured rather than inferred from timing or process RSS.
40. As a compiler maintainer, I want compiler timings separated from guest execution and marshaling, so that measurements identify which layer changed.
41. As a compiler maintainer, I want full ignored-runtime and rebuilt-browser validation, so that default test success is not mistaken for complete execution coverage.
42. As a project maintainer, I want a ranked follow-up optimization handoff with proof obligations, so that later performance work is authorized and evidence-driven rather than speculative.

## Implementation Decisions

### Architecture and type model

- Keep the portable compiler core and separate native CLI/browser adapters. Compiler stages remain modules; introduce no new IR, production dependency, runtime type registry, or general allocator framework.
- Replace the numeric-only public array variant with an explicit array type carrying a small, closed element-type enum for number, bool, and string. Retain a small `Copy` type model. This does not introduce source generics or recursive type representation.
- Centralize checked sizing/addressing, element layout, typed loads/stores, consumed-value validation, and element comparison behind private statically selected operations. Reuse them from construction, indexing, assignment, iteration, and equality.
- Keep raw allocation, capture storage, and host marshaling distinct. Do not conflate string UTF-8 byte width with string-array handle width.
- Retain the checked backend boundary: public typed nodes and annotations can be forged. Keep original typed-tree borrowing, ordered planning/lowering, and existing semantic identities rather than adding another independently implemented validator.

### Storage and ownership

- Keep eight-byte, eight-byte-aligned descriptors containing an unsigned memory32 payload pointer and unsigned element count. Descriptors remain untagged; interpretation comes from static or explicit host types.
- Numeric payload elements are eight-byte `f64` values with minimum eight-byte alignment. Boolean elements are one byte with minimum one-byte alignment. String elements are four-byte handles with minimum four-byte alignment.
- Preserve eight-byte raw-allocation alignment, minimum allocation policy, monotonic allocation, growth bounds, and failed-allocation bump-pointer behavior. Existing numeric descriptors and host-created noncontiguous payloads remain supported.
- Arrays remain fixed-length shared mutable references. Assignment, parameter passing, return, and captures share handles. Each evaluated literal allocates freshly, including empty and discarded literals.
- String elements reference existing immutable string values. Replacing an element changes its reference, not string contents. Introduce no reference counting.
- Raw host access remains trusted: structural bounds checks do not establish allocation provenance, enforce immutability, validate UTF-8, detect every wrong-type handle, or isolate a malicious host.

### Typing and existing language interactions

- Generalize literals, indexing, replacement, length, equality, iteration, internal signatures, locals, returns, and captures to the supported element types. Iteration introduces a read-only binding of the element type. Preserve mutable inner shadowing and existing capture-cell/environment lifetime rules.
- Numeric compounds remain numeric-only. Length continues to produce a language number and does not become a string operation.
- Known annotations, assignment destinations, mapped ordinary-call parameters, and explicitly declared named-function results provide expected array types. Propagate expectations through parentheses and value-producing block/if/match/loop paths.
- Anonymous callable results remain inferred; do not add result-annotation syntax. Calls with concrete results are checked, not specialized backward from their use.
- Unconstrained nonempty literals infer one exact supported type from normally completing elements. Reject mixed types and implicit conversions. Unconstrained empty/all-Never literals retain the numeric fallback; a known destination can supply the storage type without making a noncompleting expression complete.
- Do not infer array types from sibling branches, other returns, or callers. Probe/final checking must not retroactively reinterpret literals in omitted-result functions or anonymous callables.
- Map named arguments before selecting expected formal types, while preserving structural-error precedence, written evaluation order, hygienic defaults, and caller control-flow context.
- Pipeline inputs remain independently analyzed before stage lookup/binding. Do not type inputs backward from the first stage or propagate later-stage expectations backward through earlier calls.
- Preserve the defaults/contracts whitelist: existing array-reference reads, equality, and length generalize; array literals and indexing remain rejected, including skipped expressions. Purity does not imply nontrapping behavior.
- Continue checking unreachable and runtime-skipped children. Preserve element/storage type information independently of Never completion.

### Validation and execution order

- Array entry validation checks descriptor alignment, complete payload bounds, and type-specific payload alignment without scanning elements. Bounds arithmetic must be overflow-safe.
- Consuming a boolean byte other than zero or one traps. Preserve scalar host boolean normalization separately.
- Consuming a string-array element validates its string handle using the existing structural string contract; UTF-8 validity and string immutability remain host obligations.
- Literals reserve storage and initialize the outer descriptor before evaluating elements. Evaluate each element once, left-to-right; validate the completed value before storing it. Traps retain completed effects and allocations without heap rewind.
- Replacement evaluates/checks the target and index and saves the original destination before RHS effects. Evaluate and validate the replacement, then store at that destination. Do not read/validate the overwritten value. Preserve numeric-compound old-value-before-RHS behavior.
- Equality evaluates both operands left-to-right, then validates complete left and right outer descriptors/payloads before length-based exits. Compare element pairs in increasing index order, validating left then right consumed values, and stop at the first mismatch. No whole-array identity shortcut may bypass checks. Unvisited invalid elements need not trap.
- Equality compares boolean values and string contents; differing array types are rejected. Preserve existing numeric element equality, including NaN inequality even for the same array handle, signed-zero equality, and infinity behavior.
- Length validates the outer storage only. Iteration caches the initial handle/count, rechecks the live descriptor on each access, validates each yielded element before binding/capture promotion/body effects, observes alias mutations, and does not consume elements skipped by transfer. Reassignment of the source variable does not change the cached iterable.
- Saved Wasm offsets remain usable across growth; JavaScript views must be refreshed separately. Preserve completed effects across recursion, host reentry, and traps.

### Host integration and resource limits

- Each new type must work through registered host imports, selected exports, CLI host-signature declarations, browser host declarations, and explicit Node marshaling before release. Preserve external range restrictions and independent internal/external signature eligibility.
- Boolean-array encoding accepts plain JavaScript arrays of booleans; string-array encoding accepts plain arrays of strings. Encode/decode copy values; direct raw-memory access remains the explicit shared-storage option.
- Each new-array encode/decode operation has a 64 MiB aggregate logical-copy budget: one byte per boolean, or four bytes per string element plus UTF-8 bytes for every copied occurrence. Repeated occurrences count repeatedly. Use checked accounting and refresh views after allocations that may grow memory.
- The logical-copy limit is not a JavaScript object-overhead or RSS guarantee. Preserve existing standalone string and numeric-array limits.

### Compatibility and migration

- Breaking changes are permitted when necessary, not as a goal. Document the direct Rust migration from `Type::NumberArray` to the explicit array representation; do not keep a deprecated compatibility spelling.
- Update typed-tree consumers, constructors, examples, and downstream public-contract coverage. Preserve numeric-array source syntax and existing host descriptor bytes.
- Preserve old-type-only helper emission, signatures, ordering, indices, and golden module bytes unless a concrete requirement forces a separately explained and approved break. Add required new-type helpers deterministically; do not perform general legacy-helper pruning.
- Preserve diagnostic variants, messages, spans, and first-error ordering for unchanged behavior. Explain intentional contextual-typing changes and any necessary exceptions. Do not regenerate fixtures simply to hide drift.

### Measurements and ordered delivery

1. M0: establish reproducible bounded workloads and baseline output, allocation, capacity, and timing records before implementation changes.
2. M1: introduce the explicit element model and shared checked array operations using numeric arrays first; require preserved behavior, fixtures, and optimization-level parity.
3. M2: complete boolean arrays end-to-end, including contextual literals, captures, adapters, marshaling, and rejection/runtime coverage.
4. M3: complete string arrays end-to-end, including handle validation, equality, adapters, aggregate copy budgets, and unconsumed-invalid-element cases.
5. M4: complete release validation, migration/support documentation, comparative measurements, and ranked future optimization opportunities.

- Planning remains sequential. Independent tests/adapters may overlap only after concrete shared interfaces and exclusive write scopes are fixed; preserve ordered integration gates and one owner for shared public/parser/semantic/error definitions.
- Measure aligned bump consumption, initial/final Wasm memory pages, generated module sizes, existing compiler timings, and guest-call timings separately. Memory pages measure capacity, not live bytes or RSS.
- Keep allocation-marker runs separate from page-growth/timing runs. Reuse existing bounded support without adding production profiling exports. Preserve existing compiler timing boundaries; guest execution is a separate measurement.
- Cover construction, aliasing, indexed access/replacement, iteration, early/late equality exits, strings, captures, alignment boundaries, and page crossings. Record environment, toolchain, workload revision, and commands; repeat comparable runs before claiming speedups. No invented percentage gate.
- Update grammar, supported language, memory/host/browser contracts, limitations, and agent guidance as implementation lands. Clearly distinguish current support, new support, and deferred work.

## Testing Decisions

- Use the existing seams already confirmed in the design interview: public source analysis/compilation for language behavior, public typed-program compilation for forged-input contracts, and actual CLI/browser/host interfaces for adapter behavior. Do not create a testing-only production abstraction.
- Good tests assert externally observable results, diagnostics, rejection, effects, ABI bytes, and encoded-module contracts rather than private call choreography. Independent decoded layout/instruction assertions remain necessary because module validation alone cannot prove semantics; private refactoring is free where those contracts permit it.
- Frontend tests cover supported/malformed syntax, exact element typing, empty/mixed/all-Never literals, destination-directed typing, stable inferred results, named/default binding, pipelines, pure-whitelist boundaries, scopes, iteration mutability, and captures.
- Typed-input tests forge element annotations, places, storage types, and result metadata. Ensure independent backend rejection and checking of unreachable/skipped children, without weakening existing checked lowering.
- Pair Wasm validation with independent descriptor bytes, alignment/width/load/store assertions, deterministic output checks, helper layout contracts, and existing golden compatibility cases. Expected values must not all derive from production layout constants.
- Execute bounded generated modules to verify once-only order, allocation before literal effects, pre-RHS destination checks, replacement without consuming old values, alias mutation, strict boolean bytes, consumed/unconsumed invalid string handles, equality early exits, length without element scans, iteration transfers/captures, growth, reentry, and effects retained after traps.
- Preserve and extend numeric edge coverage: NaN, infinities, signed zero, extreme values, and unchanged numeric compounds/equality. Compare across all optimization levels without introducing transformations.
- Adapter tests check CLI/core output and diagnostics, host import/export eligibility, raw host-created layouts, explicit copy isolation, aggregate budget boundaries, refreshed views, and rebuilt-current-source browser parity.
- Prior art includes existing numeric-array memory, equality, iteration, ABI golden, closure lifetime/capture, named-call/pipeline, runtime marshaling, downstream-consumer, and deterministic benchmark-corpus suites. Extend their established ownership and bounded subprocess practices rather than creating a generic testing framework.
- Start with focused checks, then require offline native tests, Clippy, the portable core Wasm build, workspace architecture checks, rebuilt browser bindings, explicit ignored native/browser runtime tests, and browser-target Clippy. Preserve ignored-test policy; default Cargo success alone is not runtime evidence.
- Missing cached dependencies or tooling leave the corresponding gate unverified. Do not install/fetch prerequisites silently or claim partial validation as a full pass.
- Measurements are evidence, not correctness substitutes. Keep setup, correctness assertions, compilation, instantiation, marshaling, markers, and guest timing separated as appropriate; retain independent expected results and bounded workloads.

## Out of Scope

- Growable lists, capacity/resizing operations, slices, chained indexing, string concatenation, or additional source operators.
- Nested-array implementation, recursive collection equality/marshaling, generics, range arrays, or void arrays. Evaluate the chosen seams against future nested arrays without introducing speculative scaffolding.
- First-class callables, callable aliases, new closure result annotations, or pipeline feature expansion.
- Bit-packed booleans, runtime type registries, universal heap headers, a new IR, new package boundaries, or a pluggable allocator framework.
- Garbage collection, reference counting, heap rewind, reclamation, escape-based allocation removal, check elimination, scalar replacement, or other WP13 transformations.
- Automatic marshaling metadata, zero-copy behavior in explicit encode/decode, adversarial-host isolation, allocation-provenance enforcement, UTF-8 validation in guest bounds checks, transactional rollback, or guest execution fuel.
- General helper pruning, unrelated diagnostic redesign, hosted performance gates, or unmeasured speedup claims.

## Further Notes

The design was established through six rounds of explicit decisions, including approval of the testing seams and ordered acceptance gates. This issue synthesizes those decisions; publishing it does not itself execute or authorize compiler changes in the current agent session.

Respect ADR 0001's deferred allocation-free ranges, ADR 0004's capture ownership/lifetime rationale, ADR 0005's compiler/adapter separation, and ADR 0006's typed layout/lazy-validation decision. The workspace refactor's historical API-preservation requirement does not prevent the explicitly agreed Rust type-model migration in this separate extension.

This effort improves extensibility and optimization readiness, not heap reclamation. Long-lived instances can still exhaust monotonic storage. The final handoff must rank follow-up opportunities using measured evidence and identify aliasing, lifetime, host-effect, and trap-order proof obligations before any separate optimization authorization.

Specification publication and documentation checks are not evidence that new array types, runtime tests, browser rebuilds, or benchmark improvements have been implemented or validated.
