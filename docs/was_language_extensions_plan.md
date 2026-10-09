# WAS language extensions implementation plan

**Status:** Proposed work; this document does not implement or mark features complete.
**Baseline:** Extension specification v0.3.

Follow the [agent guide](../AGENTS.md), [extension specification](was_language_extensions_spec.md),
[current roadmap](compilation-roadmap.md), and authoritative [memory ABI](memory-abi.md).
The spec defines behavior; this plan defines incremental work. References use numbered spec headings.

## 1. Implemented baseline and compatibility envelope

Comments, local initializer inference, exact `%`, and export selection are implemented prerequisites.
Evidence: `src/lexer.rs`, `src/parser.rs`, `src/semantic.rs`, `src/codegen/numeric.rs`, `src/cli.rs`,
`tests/comments.rs`, `tests/inference.rs`, `tests/remainder.rs`, and `tests/emitter.rs`.

- `Type` in `src/ast.rs` is a small `Copy` enum, not an extensible callable/type tree.
  `Block` and `TypedBlock` are currently `Vec<Statement>` / `Vec<TypedStatement>`.
  Function results are concrete and explicitly annotated; calls resolve names/IDs.
- Semantic/backend blocks report guaranteed return with a boolean, insufficient for
  loop transfers/value-producing divergent branches. Extend this only when each step needs it.
- The locals visitor currently skips expressions. The temporary visitor counts only
  array literals; `BodyEmitter` allocates those scratch slots as `i32` without reuse.
- `ValidatedProgram` privately borrows the public, mutable `TypedProgram`, completes
  layout, then checks/lowers all user bodies in source order and caches encoded bodies.
  Keep that boundary; semantic analysis is not proof that a public tree is valid.
- Preserve registered import order and snapshots, user-function source order,
  signature deduplication, static-string first-discovery order, and helper ordering.
  Export selection changes exports only, not which bodies/imports are checked/emitted.
- Keep `number = f64`, canonical boolean boundaries, string/array descriptors,
  shared mutable arrays, allocation lifetime, mandatory memory exports, and the
  default export-all policy. `--export` and browser `setExports` already exist.
- Through WP12, all four optimization levels retain identical output and diagnostics.
  No new implicit imports, start function, general callable ABI, or execution engine.
- Old-source Wasm fixtures must remain unchanged unless an explicit contract decision
  justifies a difference. Never regenerate goldens to conceal incidental drift.

All extensions below are proposed. New keywords require matching host source-name validation,
not restrictions on arbitrary nonempty host module/field names. Reserve standalone `_`
when its syntax arrives, reject its bindings, retain `_temp`, and keep `by` contextual.

## 2. File ownership and change boundaries

These paths exist today; ownership denotes responsibility, not mandatory edits for every package.

| Existing location | Responsibility for extension work |
| --- | --- |
| `src/tokens.rs`, `src/lexer.rs` | Longest-match tokens, trivia, range/decimal boundaries, original byte spans |
| `src/parser.rs`, `src/ast.rs`, `grammar.bnf` | Contextual grammar, optional tails/results/defaults, raw syntax API |
| `src/semantic.rs`, `src/semantic_tests.rs`, `src/typed_ast_tests.rs` | Scope/type/flow rules, call binding, inference and captures |
| `src/typed_ast.rs` | Concrete bindings/signatures, resolved nodes/IDs, origin information |
| `src/host.rs`, `src/exports.rs`, `src/compiler.rs`, `src/lib.rs` | Registration/export restrictions, owned snapshots, public API and stage errors |
| `src/codegen/walker.rs`, `src/codegen/walker_tests.rs` | Exhaustive structural traversal and traversal-order tests |
| `src/codegen/locals.rs`, `src/codegen/temporaries.rs` | Separate declaration and typed scratch-slot planning policies |
| `src/codegen/abi.rs`, `src/codegen/indices.rs`, `src/codegen/layout.rs` | Private representation constants/signatures, typed indices, deterministic layout |
| `src/codegen/function.rs`, `src/codegen/control_flow.rs` | Scoped initialization, label/loop contexts, reachability and result-bearing regions |
| `src/codegen/expression.rs`, `src/codegen/statement.rs`, `src/codegen/values.rs`, `src/codegen/call.rs` | Checked lowering, places, value blocks, normalized direct calls |
| `src/codegen/memory.rs`, `src/codegen/numeric.rs` | Memory discovery/allocation, existing helpers, `%` / `%=` discovery |
| `src/codegen/validation.rs`, `src/codegen/mod.rs`, `src/codegen/validation_tests.rs` | Layout-before-body checked construction, cached bodies and assembly |
| `src/codegen/tests/`, `src/codegen/memory_tests/`, `src/codegen/export_tests.rs` | Forged trees, decoded instructions/layout, runtime and ABI regressions |
| `tests/abi.rs`, `tests/emitter.rs`, `tests/remainder.rs`, `tests/inference.rs`, `tests/comments.rs` | Existing compatibility and independent numerical/layout oracles |
| `src/cli.rs`, `src/cli_contract_tests.rs`, `tests/cli.rs` | Native registration, export errors, diagnostics, identical library/CLI bytes |
| `browser/src/lib.rs`, `tests/browser.rs`, `tests/browser.cjs` | Browser registration/configuration/diagnostics and emitted-module execution |
| `src/diagnostic.rs`, `tests/diagnostics.rs` | Exhaustive span extraction/delegation and shared native/browser rendering |
| `tests/support/process.rs`, `tests/runtime.rs`, `runtime/abi.cjs` | Bounded subprocesses, lifetime/view refresh, unchanged explicit marshaling |
| `src/optimization.rs` | Existing no-op boundary through correctness work; WP13 only |
| `README.md`, `docs/`, `AGENTS.md` | Supported-versus-proposed status, contracts, restrictions and validation workflow |

**Proposed new files, not existing implementation:** `src/codegen/place.rs` in WP1 if
sharing place lowering warrants it; `src/codegen/range.rs` in WP5 for exact operations;
`src/semantic/inference.rs` in WP7 and `src/semantic/captures.rs` in WP11 if those policies
outgrow `semantic.rs`; `src/codegen/closures.rs` in WP11 for direct helpers/cells.
These are cohesive modules, not another IR. Proposed integration suites appear below;
create them at their step, not as empty scaffolding now. Keep specialized decoders local.

## 3. Common gates and staged representation decisions

**Entry gate:** dependencies passed their exits; confirm normative rules, inventory public
AST consumers, and agree examples, rejections, spans and runtime order before edits.
Unresolved choices block their affected package, not a blanket foundation rewrite.

**Exit gate:** acceptance/rejection units, forged-tree checks, independent decoded Wasm
assertions plus `wasmparser::Validator`, runtime/adapter tests, docs and validation pass.
Check unreachable/skipped/unexported source. Record unavailable tooling/ignored cases;
default Cargo success alone cannot satisfy runtime gates.

Public-shape gates occur immediately before the package that needs the change:

1. WP1 adds a checked typed place/compound statement, not generalized expression callees.
2. WP2 adds minimal loop/label context and distinguishes fallthrough, return and
   loop transfer without mistaking `break` for a guaranteed function return.
3. WP3 prefers retaining statement-vector aliases with raw/typed value-block wrappers
   containing statements plus an optional boxed tail. Approve this API before coding.
   A small `Copy` result category (`Value(Type)`, `Void`, `Never`) can replace expression
   annotations; bindings, parameters and resolved function results remain concrete `Type`.
   `Never` has no source spelling/storage/ABI type. Document the public migration and
   reject forged inconsistent outcomes in checked lowering, rather than trusting them.
4. WP5 adds `Type::Range`, still `Copy`, with one canonical internal representation.
   WP6 changes raw argument/parameter metadata; WP7 makes only raw function result
   annotations optional. No unresolved inference variables reach `TypedProgram`.
5. WP11 represents callable bindings in dedicated symbol/helper metadata, not in
   `Type`; public metadata/IDs remain checked. Capture owner IDs must distinguish
   enclosing and helper functions whose variable IDs are separately scoped.

Extend the existing typed AST/emitter, not SSA or a duplicate validator. Keep the walker
structural and locals/temporary/memory/remainder policies separate. Block-exit hooks are
not unwind cleanup; restore scopes/label contexts on error explicitly.
Use existing dependencies and sequential planning; no new dependencies or threading.

## 4. Ordered work packages

Deliver WP1 → WP2 → WP3 → WP4 → WP5 in exactly this order. Continue with WP6–WP12
in the order below; WP13 is separately authorized performance work, not prerequisite
correctness infrastructure. Every package also inherits the common gates.

### WP1 — All five compound assignments (spec §4)

**Entry:** implemented binary remainder and checked assignment behavior are pinned.
Add longest-match `+= -= *= /= %=` and a statement-only compound node; accept only
number variables and numeric-array elements with existing identifier-rooted places.
Resolve/check the place, then RHS type. Share checked address computation in
`values.rs` / proposed `place.rs`; never duplicate the target in a textual rewrite.

Plan typed temporaries per syntactic occurrence: an `i32` element address and an
`f64` saved old value as needed. Preserve the old-value-before-RHS rule for both
local and indexed updates; captured local mutation becomes testable in WP11.
Generalize `temporaries.rs`/`BodyEmitter` beyond an all-`i32` count, without slot reuse
or changes to legacy array-slot order/encoding. Load old value before RHS; store to
the saved original place after computing. Delegate `%=` to the existing exact helper.
Add a remainder visitor statement hook: `%=` itself needs the helper even with no
binary `%`, inside unexported, unreachable or runtime-skipped bodies.

**Exit tests:** proposed `tests/compound_assignment.rs`, lexer/parser/semantic units,
planner units and forged invalid place/op/type/ID checks. Decode one address check,
saved-value loads/stores, correct scratch types and optional helper index. Execute
RHS array-element mutation (including the spec's 12-not-102 case), exactly-once
index/RHS effects, target-trap-before-RHS, all five operators and remainder
NaN/infinities/subnormals/extremes/signed-zero cases.
Scalar `%=` must not acquire memory or an implicit import; old goldens stay unchanged.

### WP2 — `break` / `continue` in `while` (spec §7)

**Entry:** WP1. Add statement tokens/nodes, reject value breaks and outside-loop
transfers, and maintain a per-function semantic loop stack. Extend backend label
tracking to every emitted `if`, `block`, and `loop`; compute `LabelDepth` from target
identity/current nesting, not a fixed `br 0`/`br 1`. A while continue reevaluates its
condition; break targets the outer exit. Track abrupt flow separately from return.

**Exit tests:** proposed `tests/loop_control.rs`; malformed syntax, nested scopes,
unknown/forged targets and transfers after return. Decode depths beneath multiple
ifs/blocks/nested whiles. Runtime effects prove nearest-loop exits, condition
reevaluation and skipped trailing statements; a while alone still does not satisfy
nonvoid definite return. Preserve old while instruction/golden behavior.

### WP3 — Tail blocks, `if` expressions and internal `Never` (spec §§3, 8)

**Entry:** WP2 and the WP3 public-shape gate. Add optional block tails with explicit
semicolon termination, value-context `if`/`else if`, and exact branch-result joining.
Statement `if` may omit else; value `if` requires it. Named functions still require
explicit return for nonvoid results; reject their nonvoid unreturned tails.
Extend flow summaries without requiring termination proofs; continue to resolve and
check every statement/operand, even after abrupt control or short-circuit skipping.

Change locals' current `visit_expressions = false` policy: declarations inside value
blocks in initializers, conditions, arguments and tails must receive slots. The
walker enumerates children; locals collects declarations without type-validating
expression contents. Preserve sparse IDs, initializer-before-binding, shadowing,
scoped initialization restoration and parameter/declaration/scratch slot ordering.
Memory, temporary and remainder policies traverse every new expression child.
Lower scalar/handle branch values with MVP single-result regions; never initialize
an artificial result on a divergent path or drop a nonexistent `Never` value.

**Exit tests:** proposed `tests/block_values.rs`; semicolon/adjacency/truncation,
missing else, void/mismatched arms, bool conditions, shadowing and unseen locals.
Forge false result/flow annotations and malformed skipped arms. Decode result types,
drops, scoped slots and nested transfers; execute one selected arm and divergent
branches. A return/continue/break path can join a concrete value without executing
later side effects. Annotated divergent initialization needs no reachable store.

### WP4 — Statement/value `loop` (spec §7)

**Entry:** WP3. Reuse target tracking and value regions. Statement loops allow bare
break; value loops unify reachable breaks to one nonvoid type and reject bare breaks.
Only breaks targeting that loop contribute; nested breaks do not type outer loops.
Analyze unreachable breaks for validity without using them as reachable constraints.
No reachable exit means `Never`; inferred `let x = loop {};` fails for lack of a
storage type, while `let x: number = loop {};` is accepted and later code is checked.

**Exit tests:** extend `tests/loop_control.rs` / `tests/block_values.rs`; mixed break
values, wrong loop kinds, nested targets, Never inference and forged break result
annotations. Decode typed outer block and relative branches; execute bounded loops
returning number/bool/string/array handles. Do not execute unbounded examples without
a bounded subprocess; compiler subprocess deadlines are not guest fuel.

### WP5 — Safe ranges, range/array `for`, membership (spec §§5–6)

**Entry:** WP4; approve range representation/export gates before touching `Type`.
Split this package into sequential checkpoints: construction/internal calls → exact
membership/progression → `for` and its latch → adapters/interactions. No partial
checkpoint is advertised as complete range/for support.

Choose the safest MVP: a single `i32` handle to an immutable private, aligned record
containing validated signed `i64` start/end/step and an inclusivity flag. A candidate
layout is 32 bytes (three eight-byte fields, flag and padding); freeze named private
size/offset/alignment constants in `abi.rs` and independent expected bytes in tests.
It is not the public string/array descriptor ABI. Reuse bump allocation initially;
do not materialize an array, expose range marshaling, or promise allocation-free
range locals. Scalar-record replacement belongs to WP13 after evidence/proofs.

Recognize `..=` before `..`, stop decimal scanning before range punctuation, reject
chained ranges, retain old malformed decimal behavior and contextual `by`. Evaluate
start/end/explicit step once left-to-right, cache inputs and eagerly validate at
construction even for discarded/empty ranges. Inputs must be finite integral f64
with magnitude ≤ 2^53−1; reject both +0 and −0 steps. Canonicalize endpoint −0 to +0
through exact integer representation. Use exact i64 interval/congruence arithmetic
for membership; invalid candidate numbers are false, not construction traps.
Default +1 descending ranges are empty. Each yielded integer converts exactly to
f64. Valid advancement beyond an endpoint exhausts normally: compare the exact next
integer before yielding, never trap or expose values beyond bounds. Safe operand
bounds keep sums/differences within i64; document that proof rather than using f64 `%`.
Invalid constant constructions remain runtime traps at the original evaluation
position, never new compile errors; folding later may replace them with that trap.

Internal range parameters/results are permitted as the same handle representation;
separate internal lowering from raw host-signature eligibility. Reject every range
host import (even unused) at registration and for forged host snapshots in layout.
Reject only selected user range signatures for exports; default export-all errors
with guidance to use existing explicit selection. `None`/compatible `Only` do not
prune internal range bodies. Check private record bounds/fields where consumed and
at internal parameter boundaries; do not publish unchecked private-handle APIs.

`for` evaluates iterable once; range fields or array handle/length are cached.
Array elements load each iteration so alias mutations of future elements are seen.
Add binding mutability metadata: iteration binding is read-only, ordinary let is
mutable; shadowing creates a different binding. Continue branches to a body-exit
latch block before increment/step and loop retest, including under nested value ifs.
Plan i64 induction/range scratch only here; preserve no-reuse slots and old output.
Append extension helpers deterministically without reordering `Helper::ALL` or the
existing optional remainder helper; feature-free modules acquire no new helpers.

**Exit tests:** proposed `tests/ranges.rs`, `tests/for_loops.rs`; token/precedence,
wrong iterable/types, read-only writes/compound updates and forged range/for metadata.
Pin records, internal signatures, export surfaces, integer instructions and latch
depths independently; compare bounded iteration to membership with an independent
reference. Runtime cases include ±safe limits, large steps/exhaustion, both endpoint
zeros (yield +0), ±zero steps, NaN/infinity/fractions, invalid empty/unused/constant
ranges, once-only construction effects, arrays/aliasing, break/return/continue.
CLI/browser must reject range imports/selected exports, allow internal signatures
under explicit selection, and preserve mandatory memory exports and legacy ABI.

### WP6 — Named/default parameters (spec §14)

**Entry:** WP5; raw-call metadata/API gate. Preserve a canonical full-arity typed
call/signature. Add argument names and trailing-only defaults to raw syntax/signature
metadata. Resolve defaults in declaration scope against earlier formals, never caller
locals/later formals. Establish a small explicit operator/builtin whitelist (initial
pure builtin `len`, with its existing descriptor traps), not user-function purity.

Normalize calls with hygienic typed sequencing: explicit arguments once in written
order into typed slots, omitted defaults afterward in formal order using bound values,
then direct full-arity call. Retain origin spans and check synthetic slot ownership.
Host declarations have no formal names/default metadata; reject named host arguments
unless separately specified. Raw exports still require all machine arguments.
**Exit tests:** proposed `tests/named_defaults.rs`; duplicates/unknown/missing names,
positional-after-named, nontrailing defaults, forbidden calls/later-name reads, forged
normalized calls, decoded full-arity order and runtime lexical/evaluation effects.
Test defaults per invocation, forward calls, host reentrancy and adapter raw arity.

### WP7 — Acyclic function-result inference first (spec §9)

**Entry:** WP6 and optional raw result-annotation API gate. Collect declarations,
build dependencies with standard collections, resolve acyclic functions in stable
dependency order, then analyze/output bodies and IDs in source order. Stage unresolved
signatures privately; only concrete types enter typed AST and layout. Preserve existing
first-error ordering for explicitly annotated old programs; pin inference diagnostics.
For recursive SCCs conservatively require explicit result annotations on every member
in the initial release, including anchored/self recursion. Do not add a speculative
fixed-point solver. Infer void only under the specified no-value-return rules; unify
reachable explicit returns, check all unreachable source and definite return.
**Exit tests:** proposed `tests/result_inference.rs`; chains/diamonds/forward calls,
void, mixed returns, missing return, named tails, annotated recursion and SCC errors.
Forge result/body mismatches; decode signatures and execute chains. Compare inferred
versus explicit ABI/bytes at all levels and native/browser diagnostics.

### WP8 — Whitelist contracts (spec §13)

**Entry:** WP7; reuse WP6's narrow pure-expression policy. Attach ordered typed
contracts with origin spans to named functions; bool required. Reject all user/host
calls, writes and nonwhitelisted constructs, including in skipped contract branches.
Emit checks at function entry after parameter validation/binding, before body, on
exported/internal/recursive calls alike. Caller defaults have already run. False means
a Wasm trap, not a portable custom exception or trusted proof. No check elimination.
**Exit tests:** proposed `tests/contracts.rs`; whitelist/type/forged metadata rejection,
decoded entry checks, declaration-order short circuit/traps and body-not-run effects.
Execute direct host exports and default calls; keep shared diagnostic span rendering.

### WP9 — `match` statements/values (spec §8)

**Entry:** WP8 and existing result/flow/range machinery. Reserve `_` as wildcard here
if not already reserved; reject standalone binding names across source/host APIs.
Add literals, boolean, relational and constant interval patterns; interval patterns
include fractional members and are not stepped range membership. Initially require
final wildcard for number/string and exhaustive boolean cases in either context.
Use ordered comparison chains and existing string content equality; evaluate scrutinee
once, only selected arm at runtime, all arms statically. No br_table optimization or
new warning framework; defer duplicate/unreachable warnings unless one already exists.
**Exit tests:** proposed `tests/match.rs`; coverage/constants/types/forged results,
NaN wildcard, fractional intervals, ordered overlap, strings, Never arms and transfers.
Decode cached scrutinee, result types/depths; execute branch/effect order in range for.

### WP10 — Named-only pipelines (spec §12)

**Entry:** WP9, normalized named/default calls from WP6. Add left-associative `|>`
below logical OR; stage resolves only to named declarations, never an expression
callee/value. Inject input into formal 0 before explicit stage args; reuse WP6 order
and defaults. Reject duplicate parameter-0 binding, wrong arity/types and nonfinal
void stages; final void allowed only in statement context. Do not introduce partial
application, placeholder args, callable values or higher-order collection intrinsics.
**Exit tests:** proposed `tests/pipelines.rs`; precedence/binding/forged normalized
calls, decoded direct calls/slots, once-only stage effects and default/contract order.
Include range-returning internal stages under compatible export selection.

### WP11 — Local-only shared-binding closures/direct helpers (spec §10)

**Entry:** WP10; approve dedicated callable/capture metadata and allocation policy.
Parse explicitly typed lambda parameters with expression/block bodies; infer result
using existing joins. Allow local declaration and direct lexical invocation only.
Reject callable aliases/reassignment, ordinary argument passing, return/array storage,
callable params/results/imports/exports, named-function values and general callee
chaining. No `fn` type, function table, `call_indirect`, or escaping callable runtime.

Analyze captures by resolved owner/binding ID before lowering; promote a shared cell
per captured binding and rewrite outer/inner reads/writes and compound places to it.
Captures inherit original mutability: read-only for bindings stay read-only inside
closures; shadowed mutable bindings are distinct. Fresh iteration bindings/cells
are logically distinct. Nested closures share original cells rather than copies.
A lambda creates a new function/control context: return exits that closure, never
the creator; break/continue cannot target a loop outside the lambda boundary.

Use monotonic existing bump allocation for initial cells/environments, no reclaim.
All layout/access constants stay private in `abi.rs`; do not add magic emitter
offsets or pretend allocation is bounded. Never reset the general heap pointer:
guest/host allocated arrays/strings can survive a creator return, even when callables
cannot escape. Fresh allocations isolate recursion/reentrant calls and trap reuse.
A future separate frame allocator requires recursion, host reentrancy and trap-unwind
proofs without harming persistent values; it is not a correctness prerequisite.
Generated helpers have known direct signatures and optional hidden environment i32;
plan them deterministically in source discovery order after existing runtime helpers,
without changing user IDs/import order. Extend checked layout/body lowering to helper
bodies and capture accesses; do not trust public capture types, owners or offsets.

**Exit tests:** proposed `tests/local_closures.rs`; all escape/alias restrictions,
read-only captured writes, cross-lambda transfers, forged helper/capture IDs/types,
shared cells, shadowing, array-binding reassignment, closure own returns. Extend
compound-place tests with captured scalar updates and RHS rebinding of an indexed
array target; preserve the saved old value and original element address. Decode
direct calls, hidden environments, offsets and absence of table/indirect calls.
Execute multiple/nested closures, zero captures, per-iteration cells, recursive
creators, host-reentrant invocations and repeated invocation after a trap. Specifically
return arrays and host-created strings from functions that allocate capture cells;
retain handles across further allocation/reentry/traps and verify bytes/mutations
remain valid. Check memory growth/view refresh and document monotonic space growth.

### WP12 — Placeholder sugar and closure pipeline stages (spec §§11–12)

**Entry:** WP11. Abstract each permitted placeholder region before final capture
analysis; each `_` is a distinct left-to-right parameter. Generate hygienic symbols,
infer only uniquely constrained parameter types, and use the same local callable
metadata/cells/helpers. Match wildcard is not a placeholder. Extend pipeline stages
to known local closures and syntactically immediate typed lambdas/placeholders;
retain WP10 injection/order, not a new higher-order calling convention.
**Exit tests:** extend closure/pipeline suites; `_ + _` arity, ambiguous `_ == _`,
region boundaries, shadowing/captures and every forbidden argument/return use,
including `filter(_ > 10)` without a specified intrinsic. Forge synthesized metadata;
decode direct helpers and execute mutable-capture stages, defaults/contracts and
once-only effects. No claim of general `map`/`filter` or pipeline fusion support.

### WP13 — Benchmark-gated optimization, separate authorization (spec §17)

**Entry:** WP1–WP12 correctness exits and explicit agreement on optimizer contracts.
First measure compile time, binary size, instantiation/call/steady-state time and
allocation growth with reproducible workloads. Use existing tools; new benchmark
dependencies, external wasm-opt integration and threading are not prerequisites.
Only then propose one proven transformation at a time at `optimization.rs`'s boundary:
range scalar replacement, loop/index proofs, closure inlining/cell elimination or
contract refinement. Preserve aliasing, IEEE behavior, signed zero, side effects,
allocation/trap order and checks of invalid public trees. No algebraic f64 rewrites.
Invalid constant ranges may lower to traps at their original positions, not static
errors. General collection fusion requires a separately specified intrinsic API.
**Exit:** independent differential results/effects/traps plus benchmarks justify the
change. Until approval, exact optimization-level parity remains required. Approved
passes may change optimized bytes only under a documented new parity contract;
retain semantic/diagnostic equivalence and the unoptimized compatibility baseline.

## 5. Documentation and validation workflow

For each delivered package, update `grammar.bnf`, README support/examples, the
roadmap and limitations, relevant runtime/browser guidance and `AGENTS.md` together.
Keep spec proposals distinct from implemented acceptance. Document every public AST
migration at its gate. Update memory ABI only for affected guarantees: ranges/cells
remain private layouts, not host descriptors; legacy marshaling stays unchanged.
Do not add rich runtime-trap diagnostics/debug metadata as incidental infrastructure.

Keep specification and plan consistent at every package boundary, particularly
export rejection during layout, canonical range zeros, invalid-constant runtime
traps, capture mutability/function boundaries, and monotonic cell lifetime. Any
intentional change to these contracts requires a coordinated specification update
and independent regression expectations, not just an implementation adjustment.

Run from repository root, offline, after actual feature edits. Start with relevant
existing unit/test filters, then new suites once created (names above are proposals).
Use the project bounded process helper for Node tests and preserve ignored-test policy.

```sh
cargo test --offline --lib
cargo test --offline --test remainder --test inference --test comments --test emitter --test abi
cargo test --offline --test cli --test diagnostics --test runtime
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo check --offline --target wasm32-unknown-unknown --lib
```

For core/browser work rebuild bindings before browser or full ignored tests. Required:
cached dependencies, Node.js 24+, installed wasm32 target, wasm-pack and compatible
wasm-bindgen tooling. Report missing prerequisites; never silently install/fetch.
There is no root `browser` feature. The exact browser/runtime release checks are:

```sh
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline
node tests/browser.cjs pkg
cargo test --offline -- --include-ignored
cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings
cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main
node runtime/run.cjs target/array-sum.wasm
```

The smoke result is `65`. Adapter tests must compare library/native/browser behavior,
export policy, full-arity/raw ABI, registration failures and shared diagnostics;
refresh `tests/browser.rs`'s completion-count assertion if browser test groups change.
Use `Number.isNaN` / `Object.is` for numerical runtime expectations; keep layout
expectations independent of production constants. Never substitute validation alone
for decoded layout assertions or execution of side effects/traps/lifetime cases.

Format only edited Rust files using edition 2024 and `skip_children=true`; do not
format the workspace. Finish every implementation package with:

```sh
git --no-pager diff --check
git --no-pager diff --stat
git --no-optional-locks status --short
```

Documentation-only changes require local link/path and whitespace checks; code
tests are normally unnecessary. The validation commands above are requirements for
future implementation, not claims that they have already passed.
