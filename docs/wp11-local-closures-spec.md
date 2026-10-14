## Problem Statement

WAS users can organize code into named functions but cannot declare a local helper
that reads and updates the surrounding lexical bindings. Extracting such code into
an ordinary function requires explicit state plumbing and cannot directly preserve
shared reassignment of local scalar or array bindings. Users need small local
helpers with predictable typing, mutation, evaluation order, and diagnostics,
without a general first-class function runtime or changes to the public host ABI.

WP10 has completed its release gates. WP11 remains proposed; its design interview
and final shared understanding were confirmed on 2026-10-14. This issue specifies
implementation and release acceptance, not evidence that the feature already works.

## Solution

Add explicitly typed local closure declarations with expression or tail-valued
block bodies and inferred concrete results. A local callable binding can be invoked
directly at its lexical call sites, but cannot be aliased, reassigned, passed,
returned, exported, or otherwise used as a value.

Shared-binding captures refer to the original lexical bindings. Reassignment from
the creator or closure is visible on both sides, multiple closures share the same
live binding, and shadowing never retargets an existing capture. Earlier local
callables can be invoked by later or nested closures through lexical callable
dependencies that preserve the original callable's live context.

Compile closures to deterministic direct helpers with private shared cells and
optional environments. Preserve ordinary named calls, contracts, result inference,
control-flow semantics, and external ABI rules. Document eager monotonic allocation
and its possible traps and memory growth rather than promising bounded storage.

## User Stories

1. As a WAS author, I want to declare a helper inside a lexical scope, so that helper logic can remain close to the state it uses.
2. As a WAS author, I want explicitly typed closure parameters, so that local helpers retain predictable static typing.
3. As a WAS author, I want zero-argument closures, so that stateful local actions do not require artificial parameters.
4. As a WAS author, I want expression-bodied closures, so that small computations need little boilerplate.
5. As a WAS author, I want block-bodied closures with statements and a tail expression, so that multi-step computations can return a result naturally.
6. As a WAS author, I want reachable explicit returns to join with a completing tail, so that early returns and normal completion have one exact result type.
7. As a WAS author, I want Void local helpers, so that side-effect-only actions can be invoked directly.
8. As a WAS author, I want incompatible returns and missing value returns rejected, so that local helpers cannot silently coerce or discard results.
9. As a WAS author, I want clear rejection when a closure result cannot be inferred, so that I am not advised to use unsupported annotation syntax.
10. As a WAS author, I want direct positional invocation of a local callable binding, so that I can use its inferred result in ordinary value contexts.
11. As a WAS author, I want parenthesized lambda initializers to remain transparent, so that grouping does not change declaration eligibility.
12. As a WAS author, I want closures to read captured number, bool, string, range, and numeric-array bindings, so that all existing concrete value types can participate.
13. As a WAS author, I want outer reassignment visible inside closures, so that captures are live bindings rather than stale snapshots.
14. As a WAS author, I want closure reassignment visible in outer code, so that a local helper can update shared state.
15. As a WAS author, I want multiple closures to share the same original binding, so that they coordinate through one live state.
16. As a WAS author, I want nested closures to retain original binding identity, so that additional nesting does not copy shared state.
17. As a WAS author, I want shadowed bindings to remain distinct, so that an existing closure never switches to a later same-named variable.
18. As a WAS author, I want captured array-binding reassignment visible to later reads, so that closures observe the current array reference.
19. As a WAS author, I want array element aliasing and mutation preserved, so that capture support does not change existing array semantics.
20. As a WAS author, I want captured compound assignment to preserve the old value and original checked target, so that RHS mutations cannot redirect a write.
21. As a WAS author, I want captured iteration bindings to remain read-only, so that closures cannot bypass for-loop write restrictions.
22. As a WAS author, I want an inner shadowing let to remain mutable, so that read-only iteration provenance does not contaminate distinct bindings.
23. As a WAS author, I want each yielded iteration to have a fresh captured binding instance, so that later iterations cannot reuse a still-relevant cell.
24. As a WAS author, I want a closure to invoke an earlier visible local callable, so that local helpers can compose without callable values.
25. As a WAS author, I want nested helpers to forward lexical callable dependencies, so that the original callable's live context is retained through intermediate closures.
26. As a WAS author, I want ordinary declaration-order visibility, so that local callable names follow the same lexical expectations as lets.
27. As a WAS author, I want local values and callables to shadow named functions consistently, so that name resolution stays predictable.
28. As a WAS author, I want callable aliases, reassignment, and escape attempts rejected clearly, so that the restricted local-callable model is understandable.
29. As a WAS author, I want a closure return to exit only that closure, so that it cannot unexpectedly return from its creator.
30. As a WAS author, I want closure break and continue restricted to its own loops, so that lexical capture does not cross function-control contexts.
31. As a WAS author, I want closures to call ordinary named functions with existing named/default arguments and contracts, so that local helpers compose with established features.
32. As a WAS author, I want annotated named recursion through a closure to remain legal, so that local helper extraction does not arbitrarily forbid a typed recursive creator.
33. As a WAS author, I want inferred named-function cycles through closure bodies rejected, so that closures do not introduce circular result-equation solving.
34. As a WAS author, I want unreachable and runtime-skipped closure code checked, so that invalid code is not hidden by control flow or export selection.
35. As a host integrator, I want recursive and reentrant creator invocations to use distinct storage, so that one invocation cannot overwrite another's captures.
36. As a host integrator, I want later invocations to remain correct after a trap, so that capture storage is not unsafely recycled by failed calls.
37. As a host integrator, I want returned or host-retained arrays and strings to survive creator return and further allocations, so that local closure storage does not invalidate persistent handles.
38. As a host integrator, I want explicit memory-growth and view-refresh guidance, so that closure allocations do not leave JavaScript views stale.
39. As a host integrator, I want no public callable ABI or generated-helper exports, so that host integration remains explicit and stable.
40. As a compiler-library user, I want malformed public typed callable metadata rejected, so that checked lowering does not assume every caller used semantic analysis.
41. As a compiler-library user, I want the raw and typed AST migration documented, so that direct AST producers can update deliberately.
42. As a compiler maintainer, I want source-ordered direct-helper planning, so that inference scheduling and hash iteration cannot change emitted order.
43. As a compiler maintainer, I want closure-local planning separated from creator planning, so that locals, temporaries, and transfer targets retain correct ownership.
44. As a compiler maintainer, I want operations inside closure bodies included in module-wide helper discovery, so that closure-only strings, ranges, and remainder operations are supported.
45. As a CLI user, I want native diagnostics and output to match the core compiler, so that closure support does not create adapter-specific semantics.
46. As a browser user, I want rebuilt-browser output and diagnostics to match current native source, so that stale generated artifacts do not masquerade as compatibility evidence.
47. As a compiler maintainer, I want acceptance evidence at all optimization levels, so that nominally identical optimization options retain behavior and diagnostic parity.
48. As a project maintainer, I want explicit release gates and retained runtime-test policy, so that a passing default suite is not mistaken for complete WP11 validation.

## Implementation Decisions

1. **Feature scope.** Support local declarations and direct lexical positional calls only. Parameters require explicit existing concrete value types. No named/default closure parameters, closure contracts, result annotations, immediate invocation, or closure pipeline stages are introduced. Calls from closures to ordinary named functions retain existing behavior.

2. **Lexical resolution.** Analyze a closure initializer before introducing its new callable binding. Capture and call resolution uses already-visible original identities, never later same-named declarations. The new declaration has no special self visibility or forward/mutual closure declaration support. An existing outer binding or named function with the same spelling can still resolve during initialization. Local values/callables shadow named functions; calling a shadowing noncallable remains invalid.

3. **Raw syntax interface.** Represent lambdas as raw expressions with explicitly typed parameters, expression-or-value-block bodies, lambda spans, and body spans. Recognizing syntax does not authorize general lambda values. Transparent outer parentheses are allowed for local initializers; a value-type annotation on a callable declaration is invalid. Raw identifier-led calls retain call spans because parser syntax cannot distinguish ordinary from lexical callable resolution. Preserve existing ordinary-call diagnostic locations.

4. **Typed ownership interface.** Introduce dedicated typed closure declarations owning their name, callable identity, parameters, concrete result, body, resolved captures, and lexical callable dependencies. Introduce dedicated typed lexical calls referencing callable identity with ordered positional arguments and a call span. There is no general typed lambda-value expression. Backend helper planning borrows declaration-owned bodies rather than cloning them or adding public user functions.

5. **Semantic identity.** Callable IDs are program-unique and assigned in source-discovery order. Binding owners explicitly distinguish user functions from closures. Original captured bindings use owner-qualified per-owner variable IDs. Parameters, locals, and loop IDs remain local to their owning function or closure. Keep semantic identities distinct from private helper indices, signatures, and offsets.

6. **Result inference.** Join reachable explicit returns with normally completing expression or block-tail results using exact types and existing Never compatibility. Bare returns contribute Void. No-tail fallthrough is allowed only for Void; reachable value returns with possible no-tail completion are rejected. Unreachable children remain checked against the final result but do not select inference. Reject expression/tail closures with only Never and value-return syntax without a reachable concrete result. Preserve the existing Void convention for no-tail block bodies without value-return syntax, including a wholly divergent statement-loop body. No Never callable signature is added; calls use concrete results rather than inheriting callee-body divergence.

7. **Named recursion and dependency scheduling.** Include named calls inside lambda bodies in structural recursion discovery, including skipped/unreachable syntax. Every recursive named-function member still requires an explicit result. Annotated named-function recursion through closures is allowed; inferred named-function cycles are rejected. Infer closures using concrete named signatures and earlier lexical callable signatures, without solving circular result equations. Closure returns never contribute to creator inference, and declaration never executes the body.

8. **Shared-binding captures.** Capture the original number, bool, string, range, or numeric-array binding by reference to its shared cell. Outer and inner reads/writes observe that same live binding, including array-reference reassignment. Multiple and nested closures share original cells, not value snapshots. Preserve original mutability: captured iteration bindings remain read-only, while distinct shadowing lets remain mutable.

9. **Lexical callable dependencies.** Distinguish invocation dependencies on earlier local callables from captured value bindings. Preserve the original callable identity and live invocation context transitively through nested closures, including intermediate closures without direct value reads. Dependencies do not permit callable aliases or values. Forward an earlier callable's original environment address rather than copying or reconstructing it.

10. **Control-flow ownership.** Each lambda establishes a new function return and loop-control context. Returns exit that lambda. Break/continue can target only loops within it. Reconstruct result and transfer checks independently during lowering, including forged typed annotations and unreachable children.

11. **Allocation timing.** Eagerly promote structurally captured bindings. Allocate captured parameter cells in formal order after all explicit parameter validation and ordered contracts, before the body. Contracts read incoming parameters before cell promotion. Allocate captured let cells after normally completing initializer effects. Allocate fresh captured iteration cells only after a successful yield, including checked array element access, before the iteration body. Allocate environments when closure declarations execute, without executing their bodies. A noncompleting initializer or empty/exhausted iteration allocates no corresponding cell. Captured bindings can allocate even if a later capturing declaration is skipped.

12. **Lifetime and failure.** Use the existing monotonic bump allocator without reclamation, lazy promotion, branch-sensitive allocation elision, or heap rewind. Distinct recursive/reentrant invocations and yielded iterations receive fresh storage. Allocation can trap or grow memory even for a never-invoked closure. Prior effects/allocations are not rolled back. Never reset the general heap pointer: arrays and strings may outlive their creator or remain retained by a host. A separate reclaimable frame allocator requires its own lifetime, recursion, reentrancy, and trap proofs.

13. **Private storage layout.** Use eight-byte-aligned eight-byte cells with f64 number storage or i32 bool/handle storage. Environments are eight-byte aligned with eight-byte slots holding i32 addresses. Add no environment header or universal callable descriptor. Keep alignment, slot widths, signatures, offsets, and checked size arithmetic centralized in the private ABI module. Accept the space trade-off over packed four-byte pointers; this layout is not a host ABI.

14. **Canonical environment ordering.** Value-capture slots come first, ordered by original owner's source-discovery order then parameter/declaration order within that owner. Callable-dependency slots follow, ordered by referenced callable discovery order. Each original binding or context-requiring callable dependency appears at most once. Context-free dependencies require no stored pointer. Reject duplicate/noncanonical public metadata instead of silently repairing it.

15. **Direct signatures.** A context-requiring helper receives a hidden first i32 environment parameter. A helper requiring no context omits the parameter and environment allocation. Absence of direct value captures alone does not prove absence of context requirements. No table, indirect call, public callable type, or generated-helper export is added.

16. **Module discovery and lowering.** Discover lambdas in source-ordered user functions and lexical depth-first preorder, with enclosing lambdas before nested ones, including unreachable/skipped syntax. Append generated helpers after existing memory/remainder/range helpers. Preserve user IDs, import order, and deterministic output independently of inference scheduling. Discover strings, memory, ranges, and remainder operations inside helper bodies, but plan each helper's locals, temporaries, and loop targets separately. Context-free closures do not introduce memory solely for callable support; capture storage activates existing memory helpers, exports, and collision rules.

17. **Checked compilation interface.** Extend the existing validated-program lowering path: complete layout and check/cache creator and helper bodies before assembly. Verify unique identities, owner/body relationships, original capture type/mutability, lexical availability/initialization, earlier-callable dependency ownership/context, complete transitive metadata, direct call arity/types, and result/control annotations. Do not trust public capture types, owners, or offsets; do not add another IR or independently implemented validator. Existing checked places retain old values and original indexed addresses across RHS effects and array-binding rebinding.

18. **Runtime trust model.** Private environment pointers are internally generated, not source values or public helper arguments. Do not add tagged runtime environment descriptors or a second validation protocol. Retain ordinary explicit string/array/range parameter entry checks and Wasm memory bounds behavior. These checks do not isolate malicious hosts able to corrupt guest memory or validate UTF-8 beyond the established host contract.

19. **Semantic diagnostic additions.** Add InvalidClosure with a stable reason and span, and CannotInferClosureResult displaying "cannot infer a concrete closure result" without an annotation suggestion. InvalidClosure reasons are "a closure must initialize a local callable binding", "a local callable binding cannot be used as a value; invoke it directly or use a named function", "a local callable binding cannot be reassigned", and "a local callable binding cannot have a value type annotation".

20. **Backend diagnostics and reuse.** Add backend InvalidClosure categories "invalid callable identity", "invalid closure ownership", "invalid capture metadata", "invalid callable dependency", "invalid callable call", and "invalid closure result". Reuse existing read-only-write, loop-transfer, incompatible-return, missing-return, named-argument rejection, and positional-argument errors where accurate. Use the stable display label "<closure>" when an existing diagnostic needs a function name. Backend callable-call checks incompatible with FunctionRef-only errors use the callable-call category.

21. **Diagnostic staging and spans.** Malformed lambdas, declaration defaults/contracts, and unsupported immediate invocation remain parser errors without general postfix chaining. Recognized lambdas in forbidden expression positions fail semantically; retain existing default/contract whitelist precedence. Unresolved self/forward references retain ordinary undefined-name errors without lookahead. Lambda spans run from parameter-list opening through body end, excluding semicolon and outer wrapping parentheses. Body spans cover the expression or complete braced body; declaration-name spans cover the identifier; lexical-call spans run from callee through closing parenthesis. Inference/join/missing-return errors use the body; placement errors use the lambda. Bare callable-value errors may use existing statement/argument origins. Forged-metadata errors use relevant declaration/call origins without inventing source detail. All offsets remain UTF-8 bytes, with existing renderer behavior.

22. **First-error ordering.** Preserve named-function declaration/recursion staging. Validate closure parameters before their body. Reject unsupported placement/value-type annotations before body analysis. Check creator statements in existing order and helper bodies in source-discovery order. No hash-order diagnostics, later-declaration lookahead, or unrelated diagnostic cleanup.

23. **Affected modules and portability.** Extend tokens/lexing, parsing/untyped syntax, semantic scope/call/inference analysis, typed declarations, structural traversal, ABI/layout, local/temporary/memory planning, checked places/function/call lowering, and stage-error extraction where required. Keep module-wide discovery distinct from owner-local planning. The core remains portable to native and compiler-in-Wasm targets; adapters continue using the shared pipeline. No speculative framework, dependency, concurrency, or optimizer is introduced.

24. **Public migration and documentation.** Document changes to raw calls/lambdas and typed declaration/call/owner/capture interfaces. Reconcile the extension specification and plan with this accepted contract. Update grammar, supported-language documentation, limitations, memory/lifetime guidance, runtime/browser guidance, and agent guidance as affected. Respect the accepted callable ownership/storage ADR and established normalized-call/pipeline ownership decisions. Distinguish proposed, implemented, and release-validated status throughout.

25. **Integration gates.** Freeze concrete interfaces and diagnostic expectations first, integrate frontend/capture/result dependency analysis second, integrate storage/places/direct-helper checked lowering third, and finish runtime/adapters/docs/release evidence fourth. Compiler integration and planning remain sequential. Independent test authors may work in parallel only after their interfaces are frozen, with exclusive write scopes and one owner per shared file. WP12 and optimization remain separately scoped.

## Testing Decisions

1. **Confirmed seams.** The design interview confirmed the existing core compilation interface as the highest primary test seam: source analysis and compilation through Compiler entry points. Use public typed-program compilation to test forged-input contracts, and existing CLI/browser adapters to test their observable contracts. No new public testing interface, alternate compiler, or generic test framework is needed.

2. **Good tests.** Assert externally meaningful language behavior, public errors/spans, emitted-module contracts, and runtime effects. Do not couple tests to private Rust function decomposition or derive every expected offset/order from production constants. Independent decoding of the deliberately specified private Wasm layout and direct-call contracts is necessary evidence, not a substitute for observable runtime tests. Module validity alone does not prove semantics.

3. **Frontend module coverage.** Use a dedicated closure frontend suite for syntax, transparent parentheses, explicit parameters, lexical visibility/shadowing, exact result inference, divergence distinctions, named dependency cycles, capture types/mutability, and own return/loop contexts. Cover all forbidden alias/escape/value/annotation/default/contract/immediate/pipeline forms. Include malformed input and source with Unicode, comments, tabs, CRLF, and EOF locations. Assert diagnostic variants, stable reasons, and independently calculated byte spans.

4. **Checked lowering coverage.** Use a dedicated closure lowering suite for forged callable IDs/owners/bodies, capture/dependency IDs/types/order/completeness, unavailable/uninitialized bindings, read-only writes, bad arity/results/control targets, and helper bodies in unreachable or unexported code. Validate emitted modules with wasmparser and independently decode helper order/signatures, hidden first parameters, cell/environment offsets, call targets, and absence of tables/indirect calls/helper exports. Verify helper-only memory/range/remainder/string discovery, creator/helper local separation, and context-free memory omission.

5. **Observable runtime coverage.** Use a dedicated ignored-by-default Node runtime suite for outer/inner shared reassignment, multiple/nested closures, shadowing, transitive callable dependencies, all captured concrete types, array-reference changes/alias mutations, once-only left-to-right effects, captured scalar compounds, and indexed compounds with RHS array rebinding. Cover zero-context helpers, read-only iteration capture, fresh iteration cells, closure-local returns and nested control flow. Check allocation-effect and trap ordering where observable, including entry validation/contracts before captured-parameter allocation and checked iteration reads before cell allocation.

6. **Lifetime and host integration coverage.** Execute bounded recursive creators, host-reentrant creator calls, and repeated invocation after a trap. Return arrays and host-created strings from creators that allocate capture storage; retain handles across later allocations, reentry, and traps and verify bytes/mutations remain correct. Exercise memory growth with refreshed JavaScript views. Do not execute unbounded recursion/divergence examples or claim the process deadline supplies guest fuel.

7. **Adapter coverage.** Use a dedicated closure adapter suite for CLI/library byte parity, WAT, diagnostics, exit codes, unchanged output destinations on failure, and temporary cleanup. Rebuilt-browser tests compare current-source native/browser bytes, WAT, execution, and rendered diagnostics. Preserve external signature eligibility, host registration snapshots, import ordering, explicit export selection, and mandatory memory export behavior.

8. **Determinism and options.** Assert repeatable bytes, source-order helper discovery independent of inference readiness, analysis/compilation host-snapshot behavior, and output/diagnostic/runtime parity at None, Basic, Standard, and Aggressive optimization levels. Export selection must not prune body checks or shift user/import indices.

9. **Prior art.** Follow existing named-call lowering tests for mutable typed-tree ownership checks; pipeline frontend/runtime/adapter tests for result contexts, options parity, exact diagnostics, and current-source browser compatibility; contracts tests for parameter validation before ordered preconditions; for-loop tests for read-only provenance, induction/control flow, and fresh iteration semantics; and place/compound tests for preserving the original target and old value across RHS effects. Use the existing bounded subprocess support that drains pipes and kills/reaps its direct child, without implying process-tree control or rollback.

10. **Release gates.** Start with focused closure suites, then run offline default Cargo tests and all-targets clippy with warnings denied. For core/browser changes, rebuild current browser bindings offline without tooling installation before browser tests and the full include-ignored suite; run browser-target clippy. Keep Node/browser tests ignored by default. Required prerequisites include Node.js 24+, the Wasm target, wasm-pack, compatible wasm-bindgen tooling, and cached dependencies. Missing tooling/dependencies block the affected release claim; never silently fetch/install. Record actual commands/results, missing coverage, and unrelated failures. Format only edited Rust files and finish with whitespace/diff review. Preserve independent compatibility fixtures unless an intentional contract change is explicitly justified.

## Out of Scope

- General first-class functions or source function types.
- Callable aliases, reassignment, arguments, returned values, array storage, imports, or exports; named-function values.
- Self/forward/mutually recursive local closure bindings or special declaration hoisting.
- Immediate lambda invocation and general callee/postfix chaining.
- Closure pipeline stages and placeholder sugar, which belong to WP12.
- Named/default closure parameters, closure contracts, closure result annotations, parameter inference, new value types, coercions, or Never callable signatures.
- New nested named-function declaration/capture semantics.
- Function tables, indirect dispatch, general higher-order map/filter facilities, or a public callable/environment ABI.
- Lazy promotion, branch-sensitive allocation elimination, packed capture slots, reclamation, frame arenas, garbage collection, or heap rewind.
- Inlining, constant folding, fusion, scalar replacement, or other optimization work reserved for separately authorized WP13.
- New dependencies, speculative frameworks, another IR/validator, concurrent compiler planning, or a new public testing framework.
- Repository-wide expression-span migration, diagnostic recovery, unrelated cleanup, and automatic fixture regeneration.
- Host isolation, execution fuel, cancellation, process-tree termination, transactional rollback, or bounded-memory promises.

## Further Notes

The 2026-10-14 interview accepted all substantive decisions and final shared
understanding. This spec is the synthesis of that agreement, not a new interview.
The tests use the already-confirmed compilation, typed-input, and adapter seams.

Eager capture promotion can allocate even when a later closure declaration is
skipped, and a never-invoked closure can still allocate an environment. These are
intentional initial-release semantics with memory-growth and allocation-trap
consequences. Non-escaping callables do not imply reclaimable or bounded storage.

Entirely divergent expression/tail bodies cannot infer a callable value result;
a no-tail block without value-return syntax follows the existing Void convention.
Calls remain concrete-typed even when their bodies diverge. Document and test this
distinction rather than accidentally introducing interprocedural Never propagation.

The public raw/typed AST migration is intentional. Keep unchanged language/ABI
behavior and diagnostic compatibility intact; tests must also cover ordinary
named/default calls, contracts, pipelines, ranges, arrays, and export selection.

Completion means implementation, reconciled documentation, and recorded release
evidence meeting every applicable gate. Publishing this issue or passing default
Cargo tests alone does not establish WP11 release readiness.
