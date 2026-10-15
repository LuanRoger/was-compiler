# WP11 local closures: design interview

**Workspace relocation:** Test paths below reflect the original interview.
Compiler suites now live in `crates/compiler/tests/`; adapter suites are split
between `crates/cli/tests/` and `crates/browser/tests/`. See
[current validation](validation.md).

This historical design interview records the accepted decisions confirmed on
2026-10-14; it did not itself authorize implementation or establish release evidence.
WP11 is now implemented with [#35 release gates passed on 2026-10-15](local-callable-interface.md#issue-35-wp11-release-validation-2026-10-15).
The design interview is complete; WP12 and WP13 remain separately scoped.

References: [extension plan](was_language_extensions_plan.md#wp11--local-only-shared-binding-closuresdirect-helpers-spec-10),
[closure specification](was_language_extensions_spec.md#10-feature-specification-anonymous-functions-and-local-closures),
and [glossary](../GLOSSARY.md).

## Accepted decisions

### Q1: WP11 scope

Support explicitly typed positional closure parameters, expression/block bodies,
local closure declarations, and direct lexical invocation. Defer immediate lambda
invocation, closure pipeline stages, placeholder sugar, named/default closure
parameters, and closure contracts. Calls from closures to ordinary named functions
retain existing named/default/contract behavior. No first-class callable values,
callable aliasing/reassignment, callable arguments/results/storage, or source `fn`
type are introduced.

### Q2: Declaration-order visibility

Analyze a closure initializer before introducing its local callable binding, as
with ordinary lets. Resolve captures and lexical calls to already-visible original
bindings. Later shadowing does not retarget them. Earlier visible closures can be
called from another or nested closure. The new declaration has no special self
visibility, forward references, or mutual-recursion support; an existing outer
binding with the same spelling still resolves normally. Local value/callable
bindings shadow named functions; calling a shadowing noncallable remains an error.
Recursive named creator functions are distinct from recursive closure bindings.

### Q3: Dedicated callable/capture identity

Use dedicated callable and binding-owner identities, not ordinary concrete-typed
value bindings or unqualified per-function variable IDs. Captures identify resolved
original bindings. Checked lowering verifies ownership, types, mutability, lexical
availability, and initialization. Cell/environment layout is backend-private and
derived, not trusted public offsets. Preserve existing normalized-call ownership
checks rather than disguising closures as ordinary lets or user functions.

### Q4: Closure result inference

Join reachable explicit returns with the normally completing expression/body-tail
result using exact types and existing Never compatibility. Bare returns contribute
Void. No-tail fallthrough is allowed only for a Void result; value returns with
possible no-tail fallthrough remain invalid. Check unreachable children against
final results without using them to select inference.

Reject expression/tail closures whose only result is Never, and value-return syntax
without a reachable concrete inferred result. Preserve the named-function Void
convention for no-tail block bodies without value-return syntax: `() => { loop {} }`
can have a Void signature, whereas `() => loop {}` has no inferable value result.
No closure result annotations or Never signatures are added. Calls use concrete
signatures and do not inherit body divergence.

### Q5: Named recursion through closures

Allow creator/closure execution cycles when every recursive named-function member
has an explicit result annotation. Include calls inside lambda bodies in structural
named recursion discovery, including unreachable/skipped syntax. Reject inferred
named-function cycles; do not solve circular result equations. Infer closures using
concrete named signatures and earlier lexical callable signatures. Closure returns
do not contribute to creator return inference; declaration does not execute a body.

### Q6: Transitive lexical callable dependencies

Distinguish original value-binding captures from dependencies on earlier visible
lexical callables. Preserve each callable's resolved identity and live invocation
context transitively through nested closures, including intermediate closures that
do not directly read the underlying captured values. These dependencies do not
permit callable aliases, argument passing, or any other callable-value use.

### Q7: Typed declaration ownership

Dedicated typed closure declarations own their parameters, inferred concrete result,
body, callable identity, resolved captures, and lexical callable dependencies.
Direct calls reference callable identity. Backend helper planning borrows these
bodies instead of cloning them or adding public user functions. Preserve lexical
initialization/scope requirements, existing user IDs/exports, and the existing
checked lowering path without another IR or independently implemented validator.

### Q8: Allocation timing

Eagerly promote structurally captured bindings: allocate captured parameter cells
in formal order after all explicit parameter validation and ordered contracts;
allocate captured let cells after completing initializers; allocate fresh captured
for cells after obtaining a successful iteration value and before the body.
Allocate closure environments when their declarations execute, without executing
closure bodies. Noncompleting initializers and empty/exhausted iterations allocate
no corresponding cell. Captured bindings may allocate even when a later capturing
declaration is skipped. Allocation can trap/grow memory; earlier effects and
allocations are not rolled back. No lazy promotion, branch-sensitive allocation
elision, reclamation, or allocator rewind is introduced.

### Q9: Private cell/environment layout

Use eight-byte-aligned, eight-byte cells with f64 number storage and i32 bool/handle
storage. Use eight-byte-aligned environments with eight-byte slots containing i32
addresses. Value-capture slots reference original shared cells; callable-dependency
slots preserve earlier callables' environment addresses, not copied environments
or binding values. Order slots deterministically by resolved declaration order.
Helpers requiring context receive a hidden first i32 parameter. Helpers requiring
no context omit that parameter and environment allocation; absence of direct value
captures alone does not imply absence of context requirements. Keep layout constants
and checked size arithmetic private in abi.rs; this is not a host ABI. Document the
space trade-off against packed four-byte pointer slots.

### Q10: Discovery and planning order

Discover helpers by source-ordered user functions and lexical depth-first preorder
lambdas, assigning enclosing lambdas before nested ones. Include unreachable/skipped
syntax and do not use inference scheduling as emitted-order authority. Helpers
follow existing memory/remainder/range helpers. Discover module-wide strings,
memory, ranges, and remainder operations inside closure bodies, but plan each
closure's locals, temporaries, and loop targets separately from its creator.
Context-free closures introduce no memory solely for callable support; capture
storage activates existing memory helpers and required exports. Preserve user IDs,
import ordering, and deterministic output.

### Q11: Canonical environment slot groups

Place value-capture slots first, ordered by original owner's source-discovery order
then parameter/declaration order within that owner. Place callable-dependency slots
second, ordered by referenced callable source-discovery order. Include each original
binding/context-requiring dependency at most once per environment. Dependencies on
context-free callables require no pointer slot. Add no environment header or runtime
callable descriptor. Derive offsets privately and reject duplicate/noncanonical
public metadata rather than silently repairing it.

### Q12: Checked-input guarantees

Checked lowering verifies unique callable/owner identities, correctly owned bodies,
existing and lexically available/initialized captures, original type/mutability,
valid earlier-callable dependencies/context, complete transitive metadata, call
arity/types/results, and independent closure control contexts. Check every helper
body, including unexported/unreachable/skipped declarations. Private environments
are generated internally, not accepted as source values or public helper arguments.
Do not expose helpers or add runtime environment headers/descriptor validation.
Retain explicit string/array/range parameter entry checks. Neither these checks nor
ordinary Wasm bounds traps isolate hosts able to corrupt guest memory.

### Q13: Diagnostic staging and span policy

Add semantic InvalidClosure with stable reason/span for forbidden callable-value
uses and recognized lambdas in unsupported placements; add CannotInferClosureResult
without suggesting unavailable annotation syntax. Add backend InvalidClosure for
forged callable/capture/dependency metadata. Reuse existing read-only-write, loop
transfer, incompatible-return, missing-return, and positional-argument errors where
their meaning remains accurate. Use the stable display label `<closure>` when an
existing error needs a function name.

Malformed lambda syntax, lambda defaults/contracts, and unsupported immediate
invocation remain parser errors without general postfix chaining. Successfully
parsed lambdas in forbidden expression positions fail semantically. Preserve
existing default/contract whitelist precedence. Unresolved self/forward references
retain ordinary undefined-name errors without lookahead. Add focused lambda, body,
declaration-name, and lexical-call spans; bare callable-value errors may use existing
enclosing statement/argument origins. Do not migrate all expressions to new spans.
Q16 freezes individual reason strings, exact span rules, and new-error ordering.

### Q14: Integration and validation gates

Freeze concrete AST/typed metadata interfaces and diagnostics, integrate frontend
analysis/dependencies/captures, integrate storage/place/direct-helper checked
lowering, then complete runtime/adapters/docs/release evidence. Compiler integration
and planning remain sequential. Parallel independent test authors require frozen
interfaces and exclusive write scopes.

Use tests/closures_frontend.rs, tests/closures_lowering.rs,
tests/closures_runtime.rs, and tests/closures_adapters.rs. Runtime/browser cases
retain the ignored-by-default policy. Required coverage includes forged metadata,
independent decoded layouts/direct calls/no tables, all optimization levels, shared
mutation/indexed compound RHS rebinding, fresh iteration storage, bounded recursive
and host-reentrant creators, post-trap invocation, and persistent returned/retained
arrays and strings. Never execute unbounded recursion examples.

Use offline Cargo tests/clippy; rebuild browser bindings before browser tests and
full ignored coverage. Missing prerequisites prevent a release claim; default test
success does not establish ignored coverage. WP12 and optimization remain excluded.

### Q15: Concrete raw/typed interface

Raw lambda expressions retain explicitly typed parameters, expression-or-value-block
bodies, lambda spans, and body spans without authorizing general lambda-value use.
Typed closure declarations are dedicated statements owning name, callable ID,
parameters, concrete result, body, captures, and callable dependencies. There is no
general typed lambda-value expression. Dedicated typed lexical calls reference a
callable ID and own ordered positional arguments and their call span.

Callable IDs are program-unique and assigned in source-discovery order. Binding
owners explicitly distinguish user-function IDs from closure IDs; original captures
use owner plus per-owner variable ID. Parameter/local/loop IDs remain owner-local.
Transparent parentheses around lambda initializers are accepted; value-type
annotations on callable declarations are rejected.

Raw identifier-led calls retain call spans because parser syntax cannot determine
whether a name resolves to a named function or lexical callable. Semantic analysis
uses these for typed lexical calls without changing existing ordinary-call error
locations. Document the public raw/typed AST migration explicitly.

### Q16: Stable diagnostic contract

Semantic InvalidClosure reasons:

| Case | Reason |
| --- | --- |
| Unsupported lambda placement | `a closure must initialize a local callable binding` |
| Callable value use/aliasing | `a local callable binding cannot be used as a value; invoke it directly or use a named function` |
| Callable reassignment | `a local callable binding cannot be reassigned` |
| Value-type annotation | `a local callable binding cannot have a value type annotation` |

CannotInferClosureResult displays `cannot infer a concrete closure result` without
suggesting unavailable result-annotation syntax.

Backend InvalidClosure uses stable categories: `invalid callable identity`,
`invalid closure ownership`, `invalid capture metadata`, `invalid callable dependency`,
`invalid callable call`, and `invalid closure result`. Reuse more specific existing
errors where their meaning remains accurate, including actual writes to captured
read-only iteration bindings. Backend call checks incompatible with an existing
FunctionRef-only error use InvalidClosure's callable-call category.

Span endpoints are UTF-8 byte offsets:

- Lambda: parameter-list opening through body end, excluding the declaration
  semicolon and outer wrapping parentheses.
- Body: expression body or complete braced body.
- Declaration name: identifier token.
- Lexical call: callee identifier through closing parenthesis.
- Inference/join/missing-return errors: body span.
- Unsupported-placement errors: lambda span.
- Bare callable-value errors: existing enclosing statement/argument origin.
- Forged metadata: relevant typed declaration/call origin, without inventing source
  detail absent from public typed input.

Preserve named-function declaration/recursion diagnostic staging. Validate closure
parameters before their body. Reject unsupported placement/value-type annotations
before analyzing that lambda body. Check creator statements in existing order and
helper bodies in source-discovery order. Do not introduce lookahead, hash-order
error selection, or unrelated diagnostic cleanup.

### Q17: Architectural decision record

Record the approved dedicated-identity/shared-cell/direct-helper architecture and
its alternatives in [ADR 0004](adr/0004-local-callable-ownership-and-storage.md).
Keep compiler mechanics out of the glossary.

## Existing constraints retained

- Shared captures observe original binding reassignment, preserve type/mutability,
  and share cells across closures; nested closures do not snapshot original values.
- Lambda return and loop-control contexts are separate from their creator's.
- Fresh iteration and recursive/reentrant invocation storage remain distinct.
- Use monotonic bump allocation without reclamation or general heap-pointer rewind.
- Use direct generated helpers after existing runtime helpers, without function
  tables, indirect calls, changing user IDs, or reordering imports.
- Validate public mutable typed input through the existing checked lowering path.
- WP12 and WP13 remain separately scoped; no incidental optimizer is authorized.

## Final confirmation

All substantive interview questions Q1–Q17 are accepted. Q18 confirmed final shared
understanding on 2026-10-14, closing the design interview. No compiler implementation
has been authorized by this interview, and no implementation or release validation
is claimed.

Reconcile the normative specification and implementation plan with this contract
as part of the separately authorized implementation workflow.
Maintain the proposed-versus-implemented distinction until release gates pass.
