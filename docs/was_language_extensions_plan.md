# WAS language extensions implementation plan

**Status:** WP1–WP9 implemented; WP5, WP6 (#13), WP7 (#16), and WP8 release gates passed (exact evidence in §5). WP9 frontend/backend are integrated and [release gates passed](#wp9-release-validation). WP10 named-only pipelines, including positional/named/default binding and final-Void discard support (#27), are implemented with [release gates passed](#wp10-release-validation). WP11's local callable (#29), immediate-creator shared parameter/let capture (#30), captured numeric compound (#31), and nested capture/live callable-context forwarding (#32) subsets are implemented in the frontend/backend; [#32 validation passed](local-callable-interface.md#issue-32-validation-2026-10-14); [#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14); [#30 subset validation passed](local-callable-interface.md#issue-30-validation-2026-10-14). #33 implements [read-only iteration captures](local-callable-interface.md#implemented-read-only-iteration-captures-33) ([validation passed](local-callable-interface.md#issue-33-validation-2026-10-15)); combined nested/transitive iteration-capture convergence, full WP11 release and WP12 remain deferred. WP13 requires separate authorization.
**Baseline:** Extension specification v0.3.

Follow the [agent guide](../AGENTS.md), [extension specification](was_language_extensions_spec.md),
[current roadmap](compilation-roadmap.md), and authoritative [memory ABI](memory-abi.md).
The spec defines behavior; this plan defines incremental work. References use numbered spec headings.

## 1. Implemented baseline and compatibility envelope

Comments, local initializer inference, exact `%`, and export selection are implemented prerequisites.
WP1 adds numeric compound assignments; WP2 adds bare while transfers and flow/label
tracking; WP3 adds block/if values and internal `Never`; WP4 adds statement/value
loops with reachable break-result constraints and explicit divergence. The baseline
implementation notes below reflect those completed packages and WP5:
range construction, immutable private records, internal calls, exact numeric
membership, and range/array statement for. Frontend, checked-lowering, deterministic
layout and bounded runtime coverage are implemented in the for suites below;
Node execution retains the ignored-test policy.

Evidence: `src/lexer.rs`, `src/parser.rs`, `src/semantic.rs`, `src/codegen/numeric.rs`, `src/cli.rs`,
`tests/comments.rs`, `tests/inference.rs`, `tests/remainder.rs`, `tests/emitter.rs`,
`tests/compound_assignment.rs`, `tests/loop_control.rs`, `tests/block_values.rs`,
`tests/value_loops.rs`, `tests/range_frontend.rs`, `tests/ranges.rs`,
`tests/for_frontend.rs`, `tests/for_lowering.rs`, `tests/for_loops.rs`, and
`src/codegen/range_tests.rs`.

- `Type` in `src/ast.rs` is a small `Copy` enum, not an extensible callable/type tree.
  `Block` and `TypedBlock` are currently `Vec<Statement>` / `Vec<TypedStatement>`.
  Raw function results are optional `Option<Type>` annotations; typed results are
  concrete after acyclic inference. Every recursive component member requires an
  explicit result in the initial release. Calls resolve lexical names/IDs.
- Expressions carry `ExprResult::{Value(Type), Void, Never}`; bindings/signatures
  retain concrete `Type`. Raw/typed value-block wrappers add optional boxed tails
  without replacing statement-vector block aliases. Individual expressions still
  have no independent spans.
- Semantic expressions propagate transient `(TypedExpr, Flow)`; backend lowering
  reconstructs completion while emitting, without public flow annotations or a
  second validator. Every statement/child still receives checking/emission.
  A while consumes its own transfers; activate its context before the condition.
  Its body alone is not a guaranteed return, but an unconditional explicit return
  in its condition is. Do not infer Never from arbitrary calls or possible traps.
- The locals visitor traverses expressions to collect nested declarations without
  validating their expression contents. Reserve an outer declaration slot before
  initializer declarations, preserving declaration preorder. The temporary visitor plans
  ordered array `i32` slots and indexed-compound `i32` address / `f64` old-value
  slots without reuse; local compounds keep the old value on the operand stack.
  For adds a numeric iteration-binding slot and exact `i64` current/end/step plus
  `i32` inclusivity scratch, keeping planner policies separate.
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

WP1–WP5 are implemented, including array iteration and issue #6 adapter/interaction
support; all WP5 release gates passed as recorded in §5. WP6 named/default parameters
and WP7 acyclic result inference are implemented with release gates passed (#13/#16).
WP8 whitelist contracts are implemented, with
[release gates passed](#wp8-release-validation). WP9 match statements/values are
implemented with frontend/backend integrated and [release gates passed](#wp9-release-validation). WP10 named-only pipelines are implemented with [release gates passed](#wp10-release-validation). WP11's local callable (#29), immediate-creator shared parameter/let capture (#30), captured numeric compound (#31), and nested capture/live callable-context forwarding (#32) subsets are implemented in the frontend/backend; [#32 validation passed](local-callable-interface.md#issue-32-validation-2026-10-14); [#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14); [#30 subset validation passed](local-callable-interface.md#issue-30-validation-2026-10-14). #33 implements [read-only iteration captures](local-callable-interface.md#implemented-read-only-iteration-captures-33) ([validation passed](local-callable-interface.md#issue-33-validation-2026-10-15)); combined nested/transitive iteration-capture convergence, full WP11 release and WP12 remain deferred, and WP13 requires separate authorization.
`requires`, `match`, and standalone `_` are reserved for source/host-language names,
not arbitrary nonempty host module/field names. `_temp` remains valid; `by` stays
contextual. WP9 does not implement ignored bindings or placeholder expressions.

## 2. File ownership and change boundaries

These paths exist today; ownership denotes responsibility, not mandatory edits for every package.

| Existing location | Responsibility for extension work |
| --- | --- |
| `src/tokens.rs`, `src/lexer.rs` | Longest-match tokens, trivia, range/decimal boundaries, original byte spans |
| `src/parser.rs`, `src/ast.rs`, `grammar.bnf` | Contextual grammar, optional tails/results/defaults, raw syntax API |
| `src/semantic.rs`, `src/semantic_tests.rs`, `src/typed_ast_tests.rs` | Scope/type/flow rules, call binding, inference and captures |
| `src/typed_ast.rs` | Concrete bindings/signatures, resolved nodes/IDs, origin information |
| `src/flow.rs` | Shared private completion sets, independent of static traversal |
| `src/match_patterns.rs` | Shared semantic/backend pattern-domain, numeric-constant and exhaustiveness checks |
| `src/codegen/match_.rs` | Private checked match lowering, cached scrutinee, scoped arms and ordered comparison regions |
| `tests/match_frontend.rs`, `tests/match_lowering.rs` | WP9 frontend/inference and forged-tree/decoded lowering contracts |
| `src/host.rs`, `src/exports.rs`, `src/compiler.rs`, `src/lib.rs` | Registration/export restrictions, owned snapshots, public API and stage errors |
| `src/codegen/walker.rs`, `src/codegen/walker_tests.rs` | Exhaustive structural traversal and traversal-order tests |
| `src/codegen/locals.rs`, `src/codegen/temporaries.rs` | Separate declaration and typed scratch-slot planning policies |
| `src/codegen/abi.rs`, `src/codegen/indices.rs`, `src/codegen/layout.rs` | Private representation constants/signatures, typed indices, deterministic layout |
| `src/codegen/function.rs`, `src/codegen/control_flow.rs` | Scoped initialization, label/loop contexts, reachability and result-bearing regions |
| `src/codegen/expression.rs`, `src/codegen/statement.rs`, `src/codegen/values.rs`, `src/codegen/call.rs`, `src/codegen/place.rs` | Checked lowering, shared places, value blocks, normalized direct calls |
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

**Implemented WP5 module:** `src/codegen/range.rs` owns range discovery,
construction, private-record validation, and exact numeric membership helpers.
Range advancement and array index iteration are inline checked control-flow
lowering, not new helpers. Array iteration reuses checked `ArrayAddress` and
preserves live alias observation without changing the descriptor ABI.

**Private policy modules:** `src/semantic/inference.rs` implements #11 result
inference and #14 acyclic dependency scheduling; `src/semantic/closures.rs` implements
local callable inference, original-binding capture forwarding and earlier lexical
callable dependencies; `src/codegen/closures.rs` handles direct helper/cell planning.
These are cohesive modules, not another IR. Proposed integration suites appear below;
create them at their step, not as empty scaffolding now. Keep specialized decoders local.

## 3. Common gates and staged representation decisions

**Entry gate:** for integration/release, dependencies passed their exits; confirm normative
rules, inventory public AST consumers, and agree examples, rejections, spans and runtime
order before edits. Development may overlap under the coordination rules in §4 once
its shared interfaces are agreed. Unresolved choices block their affected package,
not a blanket foundation rewrite.

**Exit gate:** acceptance/rejection units, forged-tree checks, independent decoded Wasm
assertions plus `wasmparser::Validator`, runtime/adapter tests, docs and validation pass.
Check unreachable/skipped/unexported source. Record unavailable tooling/ignored cases;
default Cargo success alone cannot satisfy runtime gates.

Public-shape gates occur immediately before the package that needs the change:

1. WP1 adds a checked typed place/compound statement, not generalized expression callees.
2. WP2 adds typed `LoopId`, `While.id`, and resolved `Break` / `Continue` targets,
   requiring public typed consumers to update constructors/matches. Private
   completion sets and loop/label contexts distinguish transfers from returns.
3. WP3's approved and implemented API retains statement-vector aliases and adds
   `ValueBlock` / `TypedValueBlock` with statements, optional boxed tail, and span.
   `TypedExpr.ty` now uses the small `Copy` `ExprResult::{Value(Type), Void, Never}`;
   migrate old expression constructors with `ty.into()` and extract concrete types
   with `value_type()`. Bindings/parameters/function results remain `Type`.
   `Type::Void.into()` is `Void`; `Value(Type::Void)` is malformed. Never has no
   source spelling/storage/ABI type, and public equality is identity, not Never
   compatibility. Checked lowering rejects inconsistent outcomes.
4. WP5 adds `Type::Range`, still `Copy`, with one canonical internal representation.
   Issue #4 adds typed `For { id, variable, name, iterable: Box<TypedExpr>, body }`.
   Derive read-only restrictions from For provenance, without public mutable-binding
   metadata; update exhaustive public statement matches/constructors.
   WP6 changes raw argument/parameter metadata; WP7 makes only raw function result
   annotations optional. No unresolved inference variables reach `TypedProgram`.
5. WP8 adds raw `Function.contracts: Vec<ContractExpression>` and typed
   `TypedFunction.contracts: Vec<TypedContract>`. Existing constructors supply
   `vec![]`; wrappers contain `value: Expr` / `TypedExpr` and expression-only
   UTF-8 byte `span`, excluding the keyword and surrounding trivia. Expressions
   themselves still have no independent spans. Contracts retain declaration order
   and resolved callee parameter IDs; checked lowering validates public metadata.
6. WP11 represents callable bindings in dedicated symbol/helper metadata, not in
   `Type`; public metadata/IDs remain checked. Capture owner IDs must distinguish
   enclosing and helper functions whose variable IDs are separately scoped.

Extend the existing typed AST/emitter, not SSA or a duplicate validator. Keep the walker
structural and locals/temporary/memory/remainder policies separate. Block-exit hooks are
not unwind cleanup; restore scopes/label contexts on error explicitly.
Use existing dependencies and sequential planning; no new dependencies or threading.

## 4. Ordered work packages

Deliver WP1 → WP2 → WP3 → WP4 → WP5 in exactly this order. WP6–WP12 retain ordered
integration and release gates, but development may overlap after the relevant shared
interface gate. WP13 is separately authorized performance work, not prerequisite
correctness infrastructure. Every package also inherits the common gates.

### Parallel development and ownership

Approved coordination policy: parallel development does not waive predecessor exits
or authorize parallel execution inside the compiler. A single integration owner controls
shared public shapes, parser/grammar, semantic orchestration and stage-error definitions.
Assign each shared file one writer; other agents work through agreed interfaces and
exclusive file scopes. Do not introduce speculative modules merely to divide work.

| Development wave | Parallel scopes | Integration/release constraint |
| --- | --- | --- |
| WP6 + WP7 | WP6 binding/default policy; WP7 dependency/result policy; checked normalization lowering; separately owned integration suites | Agree raw metadata, declaration signatures, normalized-sequence ownership and inference interfaces first. Release WP6 before WP7. |
| WP8 + WP9 + WP10 | Contracts, match, and named-pipeline policies/tests; separate lowering scopes where practical | Share WP6's approved whitelist and normalization. Coordinate AST/parser/semantic edits through the integration owner; release WP8, then WP9, then WP10. |
| WP11 | Capture analysis, cells/direct-helper lowering, independent lifetime/escape tests | Approve callable/capture metadata and allocation policy; integrate after WP10's exit. |
| WP12 | Placeholder abstraction and closure-pipeline integration/tests | Requires WP11's callable/capture representation and WP10's pipeline rules. |
| WP13 | Measurement planning only before separate authorization | All correctness exits and an explicit optimizer/parity agreement precede transformations. |

Acceptance examples, rejection matrices and representation research for later waves
may proceed early. Do not reserve future keywords, change public shapes, or add empty
scaffolding before the affected interface gate. One owner coordinates shared adapter
harnesses and documentation. The WP6/WP7 policies and first-wave scope below are
approved. Obtain final confirmation of the shared design before compiler implementation;
then freeze concrete interfaces and exclusive write scopes before parallel dispatch.

### WP6/WP7 shared interface decisions

- Raw call arguments carry an optional name, an expression and an argument span.
  Raw parameters gain an optional default with its declaration-site span; raw function
  result annotations become `Option<Type>`. Typed signatures remain concrete/full-arity.
  Do not add independent spans to every expression. Preserve legacy diagnostic locations.
- Calls needing reordering/default evaluation use a dedicated typed normalized-call
  sequence with ordered invocation-local bindings, formal-to-binding mapping, origin
  spans and a final full-arity direct call. Checked lowering verifies scope, types,
  ordering and mapping consistency. Keep ordinary full-positional calls on the existing
  direct path when normalization is unnecessary, preserving legacy Wasm bytes.
- Resolve default references against declaration-side formal identities, then remap
  them to invocation-local bindings. Never substitute by name or copy callee-local
  numeric IDs into the caller. Preserve caller control-transfer behavior in explicit
  argument expressions; normalization adds no function or loop context.
- Recursive-component detection includes every lexically resolved user-function call,
  including unreachable calls and runtime-skipped operands. Respect lexical shadowing;
  text-only function-name matching is insufficient. Exclude host/builtin calls. A smaller
  result-scheduling dependency set must not weaken the recursive annotation rule.
- Preserve exact legacy first-error order for old fully annotated programs. For programs
  using new features, stage declaration/default errors in source order, dependency/SCC
  errors with source-order tie-breaking, inference errors in stable dependency order,
  then final body checks in source order. Function IDs, typed-function output and Wasm
  function order remain source-ordered regardless of inference scheduling.

- Validate every declared default during semantic analysis, including numeric-literal
  syntax/overflow, even when unused or always explicitly supplied. Report new default
  errors at their declaration origins. Preserve ordinary body-literal analysis/codegen
  behavior, allow underflow and add no arithmetic constant folding.
- Named/default call binding resolves the target and lexical shadowing first, rejects
  named host/builtin arguments, scans structural errors in written order, checks missing
  required parameters, then analyzes explicit expressions in written order and instantiates
  omitted defaults in formal order. Preserve legacy ordinary-call error ordering.

These decisions extend the existing typed tree, not another IR or a second whole-program
validator. Private inference shares the expression checker and completion rules; it must
not introduce public unresolved types or public flow annotations.

### First-wave execution handoff

This historical first-wave handoff required final design confirmation before WP6
and WP7 implementation. Those packages and their release gates are now complete;
WP8 is implemented with release gates passed; WP9 frontend/backend are now
integrated with [release gates passed](#wp9-release-validation). WP10 is implemented
with [release gates passed](#wp10-release-validation). WP11's #29–#33 subsets are
implemented in the frontend/backend; #31 adds captured numeric compounds and #32
adds nested capture/live callable-context forwarding ([validation passed](local-callable-interface.md#issue-32-validation-2026-10-14)); [#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14); #33 implements [read-only iteration captures](local-callable-interface.md#implemented-read-only-iteration-captures-33) ([validation passed](local-callable-interface.md#issue-33-validation-2026-10-15)); combined nested/transitive iteration-capture convergence, full WP11 release and WP12 remain deferred, and WP13
remains separately authorized. The module paths below
record original write-scope assignments, not current module inventory or a reason
to create empty scaffolding.

| Owner | Exclusive write scope | Handoff |
| --- | --- | --- |
| Integrator | `src/ast.rs`, `src/typed_ast.rs`, `src/parser.rs`, `grammar.bnf`, shared `src/semantic.rs` orchestration/errors and frontend/typed unit-test migrations; shared public/module wiring, adapter harnesses and docs | Freeze concrete raw/typed records, declaration/default interfaces, return-summary interface, diagnostic variants and synthetic identity rules before dispatch. Own backend stage-error/assembly wiring in `src/codegen/mod.rs`. |
| WP6 policy agent | Proposed `src/semantic/defaults.rs`, `src/semantic/call_binding.rs`, and `tests/named_defaults.rs` | Check defaults in declaration scope, produce binding/normalization policy through the frozen interface, and supply acceptance/rejection/evaluation-order tests. |
| WP7 inference agent | Proposed `src/semantic/inference.rs` and `tests/result_inference.rs` | Resolve lexical dependencies/SCCs and concrete results through the shared checker/summary interface; test deterministic diagnostics, recursion restrictions and explicit/inferred parity. |
| Backend agent | `src/codegen/` implementation/tests except integrator-owned `mod.rs` | Check/lower normalized sequences; update exhaustive structural traversal and relevant policies; cover forged mappings/IDs/types, decoded locals/calls and bounded runtime effects. |

The integrator coordinates any shared numeric-literal helper needed by semantic default
checking and existing backend checking; preserve ordinary-body errors and avoid two
independent literal-rule implementations. Agents request shared-file changes from the
owner instead of editing outside their scopes. Record any additional touched path and
its owner before edits.

Complete the WP6 gate before accepting WP7's integration/release. Run focused checks,
then the full offline/native/wasm32/browser/runtime gates in §5; rebuild browser bindings
before browser or full ignored execution. Check rejection, forged trees, decoded Wasm,
observable effects, optimization-level parity and old fixtures. Record actual evidence
and unavailable prerequisites; preserve the default ignored-test policy.

### WP1 — All five compound assignments (spec §4): implemented

**Completed scope:** Longest-match `+= -= *= /= %=` tokens and statement-only
`CompoundAssignment { target, op: BinaryOp, value }` variants in both raw and typed
ASTs. Targets are number variables or identifier-rooted numeric-array elements;
semantic and backend checks reject non-arithmetic operators in forged trees.
Public AST consumers must handle these new statement variants. Checked places
in `place.rs` share ordinary-assignment lowering and `values.rs` address checks.

Typed temporaries reserve an `i32` element address and `f64` old value per indexed
occurrence before traversing children. Local updates keep the old value on the
operand stack. Target/bounds checks and old-value reads precede RHS effects;
writes use the original place. There is no scratch reuse or change to legacy
array-slot order/encoding. WP11's #30 subset adds ordinary captured assignment;
#31 adds all five captured numeric compounds through this checked place path,
including creator-side promoted bindings. Scalar cell address/old `f64` remain on
the stack; indexed address/old value use existing scratch. Ordinary indexed
assignment retains its pre-RHS checked element address. [#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14).
`%=` delegates to the unchanged exact helper and triggers its statement discovery
hook even without binary `%`, in hidden, unreachable or runtime-skipped bodies.

**Implemented coverage:** `tests/compound_assignment.rs`, lexer/parser/semantic units,
planner units and forged invalid place/op/type/ID checks. Decode one address check,
saved-value loads/stores, correct scratch types and optional helper index. Execute
RHS array-element mutation (including the spec's 12-not-102 case), exactly-once
index/RHS effects, target-trap-before-RHS, all five operators and remainder
NaN/infinities/subnormals/extremes/signed-zero cases.
Scalar `%=` must not acquire memory or an implicit import; old goldens stay unchanged.

### WP2 — `break` / `continue` in `while` (spec §7): implemented

**Completed scope:** Reserved statement keywords, raw unit transfers, typed
per-function `LoopId`s on while nodes and resolved transfer targets. Semantic loop
stacks select the nearest enclosing while and restore on error; value/label forms
are parse errors, outside-loop forms are semantic errors. Public typed consumers
must handle `While.id`, `Break { target }`, and `Continue { target }`.

`BodyEmitter` tracks unique label identities for every emitted `if`, `block`, and
`loop`, including short-circuit expression ifs. Relative `LabelDepth` is computed
from active nesting rather than fixed `br 0`/`br 1`. Continue reevaluates the
condition; break targets the exit block. Checked lowering rejects duplicate loop
IDs and unknown, inactive, non-innermost, or cross-function transfer targets.
Private `Flow` completion sets separate transfers from guaranteed return without
pruning unreachable checks/emission. Scope/label/loop restoration is error-safe.

**Implemented coverage:** `tests/loop_control.rs`; malformed syntax, nested scopes,
unknown/forged targets and transfers after return. Decode depths beneath multiple
ifs/blocks/nested whiles. Runtime effects prove nearest-loop exits, condition
reevaluation and skipped trailing statements; a while alone still does not satisfy
nonvoid definite return. Preserve old while instruction/golden behavior.

### WP3 — Tail blocks, `if` expressions and internal `Never` (spec §§3, 8): implemented

**Completed scope:** Optional block tails, value-context `if`/`else if`, exact
branch-result joining with Never, and expression completion propagation. Value if
requires else; statement if may omit else and discards branch tails. Named functions
retain explicit returns; function/while ordinary expression tails are rejected.
Standalone block expression statements require semicolons. In a value block a
final bare if is a tail; a nonfinal bare if is a statement. Bare statement-if ends
at its last arm before `-1;` / `--1;`, independent of trivia. Parenthesize the whole
if for arithmetic continuation in these positions. Legacy shapes/spans/bytes stay
unchanged. Semicolons discard values, Void is not a local value, and inferred Never
initialization reports `CannotInferNeverType`; annotated Never is accepted.

The approved public migration is recorded in §3 above. Semantic expression analysis
returns transient flow alongside the typed node; checked lowering reconstructs it.
No public flow annotation, second validator, or subtree reanalysis is introduced.
Unreachable statements, unselected functions, and skipped operands remain checked.
A divergent logical RHS alone retains the short-circuit normal-completion path.
While context covers the condition, so its blocks can transfer to that loop;
an unconditional condition return guarantees return before the body, unlike a
literal-true body alone.

The walker visits value-block statements then tail within block hooks. Locals now
visit expressions for nested declarations, reserving the outer let before its
initializer declarations. Sparse IDs, scoped initialization, declaration order,
separate memory/scratch/remainder policies and no scratch reuse are preserved.
MVP single-result regions use f64 for numbers and i32 for bool/handles, empty for
Void/Never. Divergent paths produce no artificial result/store/drop; lowering
restores stack polymorphism after a region with no normal completion.

**Implemented coverage:** `tests/block_values.rs`, focused frontend/planner/backend
units, diagnostics and CLI/browser suites. Rejections cover missing else, category
mismatches, bool conditions, unseen locals, adjacency/truncation and malformed
hidden/skipped typed input. Independent decoded region/drop/local assertions and
Validator checks accompany Node execution for selected arms, shadowing,
short-circuiting, returns/transfers, original compound places and trap order.
Import snapshots, exports and optimization parity remain tested. Independent
legacy unary-minus byte expectations are retained. WP4 extends these regions.

### WP4 — Statement/value `loop` (spec §7): implemented

**Completed scope:** Reserved `loop`, statement/value loops and `break value;`.
Raw/typed expression and statement enums add `Loop { body }`, with a per-function
LoopId in typed nodes. Existing bare Break variants remain; raw `BreakValue(Expr)`
and typed `BreakValue { target, value }` are separate. Loop bodies retain strict
statement vectors, not tails. Update public exhaustive matches; no ABI/type
representation changes are introduced.

Prefix bare loops in function/statement bodies, including `loop {};`, are
statements. In value blocks, final bare loops without semicolons are value tails;
nonfinal or semicolon-terminated bare loops are statements. Parenthesized loops
are expressions, including discarded values. No newline heuristics or arithmetic
continuation across bare statement-loop boundaries. Reserve loop in host source
names, never module/field names.

Statement/while loops allow only bare breaks; value loops require values. All
reachable breaks targeting a value loop unify to one concrete nonvoid type.
Nested-loop constraints are consumed independently. Private Flow carries reachable
constraints through sequence/branch composition; there is no constraint collector
that ignores reachability, subtree rewalk, or public flow annotation. Unreachable
breaks do not constrain results but remain contextually/type checked. Void values
are invalid; a Never RHS performs no actual break. Kind/void failures have located
`MissingBreakValue`, `BreakValueNotAllowed`, and `InvalidBreakValue` errors; reachable
result conflicts use TypeMismatch at the enclosing loop's diagnostic span.

No reachable exit yields Never. Inferred `let x = loop {};` fails; annotated
initialization is accepted with no reachable store, and later source is checked.
Nonvoid functions allow returns or explicit divergence, never normal fallthrough.
Ordinary while remains conservative even with literal true; a noncompleting while
condition consumes its own break/continue outcomes without inventing completion.

Checked lowering uses a declared single-result outer block and empty backedge loop,
then verifies the annotation against reconstructed flow. Unreachable breaks with
incompatible raw Wasm payloads emit Unreachable rather than an invalid typed branch;
their constraints still fail if reachable. Keep dynamic label identities, scoped
cleanup, live stack prefixes, separate planners, helper ordering and no result local.
The walker traverses loop bodies within block hooks and value-break RHSs as expressions.

**Implemented coverage:** `tests/value_loops.rs`, focused lexer/parser/semantic,
walker/backend/cleanup tests, diagnostics and CLI/browser suites. Validator plus
independent region/branch/local/helper assertions accompany bounded Node execution
of all result types, effects, transfers, aliases and saved compound places. Forged
IDs, loop kinds/results and hidden/unreachable children are checked. Legacy fixtures,
optimization parity, exports and import snapshots are retained. Unbounded loops
are compile-only tests; subprocess deadlines are not guest fuel. WP5's first
checkpoint follows below.

### WP5 — Safe ranges, range/array `for`, membership (spec §§5–6)

**Entry:** WP4. Range representation/export gates are approved as specified below;
construction/internal calls, numeric membership, range/array statement for, and
issue #6 adapter/interaction support are implemented. All release gates passed;
exact commands/results are recorded in [§5](#5-documentation-and-validation-workflow).
See [ADR-0001](adr/0001-private-range-representation.md)
for the representation and external-boundary rationale.
Split this package into sequential checkpoints: construction/internal calls → exact
numeric membership → range-only statement for with exact advancement/latch → array
iteration → adapters/interactions. All five are implemented and the release gates
passed, with validation evidence recorded separately from test presence.

**Implemented first checkpoint:** `Type::Range` and raw/typed range expressions
carry start/end, optional step, and inclusivity. `..` / `..=` bind below arithmetic
and above comparisons, non-associatively; `by` remains contextual with default
`+1`. Numeric operands use existing Never compatibility. Locals/assignment,
internal parameters/results, blocks/if/value loops and exact/Never joins work.
Range equality is rejected. Construction and parameter validation follow the
record/boundary rules below; helpers append `Construct`, then `Validate` after
memory and optional remainder, leaving feature-free bytes unchanged.
Coverage lives in `tests/range_frontend.rs`, `tests/ranges.rs`, and
`src/codegen/range_tests.rs`: frontend rejection, forged trees, Validator plus
independent record/instruction/layout assertions, export/import gates,
optimization parity, and ignored bounded Node validation/effect cases.
Exact numeric membership and range/array statement for are implemented as described
below; all release gates passed as recorded in §5.

The approved MVP uses a single `i32` handle to an immutable private, eight-byte-aligned
32-byte record. Validated signed `i64` start/end/step occupy byte offsets 0/8/16;
a `u32` inclusivity flag at offset 24 is exactly 0 or 1, and four bytes of reserved
padding at offset 28 are initialized to zero. Copies share the immutable record.
Freeze named private size/offset/alignment constants in `abi.rs` and independent
expected bytes in tests.
It is not the public string/array descriptor ABI. Reuse bump allocation initially;
do not materialize an array, expose range marshaling, or promise allocation-free
range locals. Scalar-record replacement belongs to WP13 after evidence/proofs.

Recognize `..=` before `..`, stop decimal scanning before range punctuation, reject
chained ranges, retain old malformed decimal behavior and contextual `by`. Evaluate
start/end/explicit step once left-to-right, all before any validation even if an
earlier operand is invalid; cache inputs and eagerly validate at
construction even for discarded/empty ranges. Inputs must be finite integral f64
with magnitude ≤ 2^53−1; reject both +0 and −0 steps. Canonicalize endpoint −0 to +0
through exact integer representation. Invalid constant constructions remain
runtime traps at the original evaluation position, never new compile errors.


Range `==` and `!=` remain static errors, including for forged typed input;
membership introduces no handle, record, or sequence equality contract.

Internal range parameters/results are permitted as the same handle representation;
separate internal lowering from raw host-signature eligibility. Reject every range
host import (even unused) at registration and for forged host snapshots in layout.
Reject only selected user range signatures for exports; default export-all errors
with guidance to use existing explicit selection. `None`/compatible `Only` do not
prune internal range bodies. Check private record bounds/fields where consumed and
at internal parameter boundaries; do not publish unchecked private-handle APIs.

**Implemented numeric membership checkpoint:** `number in range` returns `bool`
at comparison precedence with existing left associativity. Reserve `in` for source
and host-language names, leaving host module/field names unrestricted. Require
exact asymmetric operand types with Never compatibility and check all children,
including unreachable/skipped code. Evaluate both operands once, left-to-right.
Validate the consumed private record before the candidate guard, so malformed
records trap even for invalid candidates. Nonfinite, fractional, or out-of-safe-domain
candidates return false; both signed zeros are accepted. Use signed `i64` interval
checks and `(candidate - start) rem_s step == 0` via `I64RemS`, not f64 `%`.
The [memory ABI](memory-abi.md#private-range-record) and spec §5.4 prove that
`|candidate - start| <= 2*(2^53−1) < 2^54` and a validated nonzero safe-domain step
cannot cause integer overflow or a remainder trap. Append optional `Contains`
after `Construct`/`Validate` only when membership occurs anywhere in the typed AST;
construction-only bytes remain unchanged, and no floating remainder helper is needed.
Frontend and decoded-lowering coverage lives in `tests/range_frontend.rs` and
`tests/ranges.rs`.

**Implemented range-only statement for checkpoint (issue #4):** Reserve `for` for
source and host-language names, not host module/field names. Accept
`for name in range_expr { body }` with exact range/Never compatibility; issue #5
below extends the same syntax/node to numeric arrays.
Raw statements add `For { name, iterable, body }`; typed statements add
`For { id, variable, name, iterable: Box<TypedExpr>, body }`, retaining strict
statement-vector bodies and existing per-function loop/binding ID checks. For is
a void statement, never a value expression. Bare breaks are permitted; value
breaks targeting for are rejected even when unreachable.

Evaluate the iterable once in the enclosing context, before activating the new
loop or introducing its iteration binding. Break/continue during evaluation targets
an existing outer nearest loop under its usual rules, or is invalid without one;
unlike while conditions, the iterable cannot target the new loop. Preserve iterable
return/divergence without inventing fallthrough, and check the body regardless.
A normally completing range may be empty, so its body alone cannot guarantee return.
The numeric iteration binding is read-only for ordinary and compound writes, even
when unreachable; a distinct inner shadowing let remains mutable. Reconstruct
checked-lowering restrictions from For provenance, with no new public mutable-binding
metadata or duplicate validator.

Validate the consumed record, then cache exact signed `i64` current/end/step and
`i32` inclusivity scratch. Default +1 descending ranges are empty. Test signed
endpoints before exact `f64` conversion/yield; zero yields canonical +0. Continue
branches to a body-exit latch block, then `I64Add` advances and the loop retests,
including under nested value ifs/blocks; break/return skip advancement. Resolve
label depths by identity, not fixed relative depths. The structural walker visits
iterable then body within body block hooks. Preserve separate local/memory/
temporary/remainder policies, no-reuse scratch slots, and old-source output.
For adds no range helpers: reuse `Validate`, keep `Construct`/`Validate` and optional
membership-only `Contains` order unchanged, and do not add a floating remainder need.

**Advancement bounds:** With `M = 2^53−1`, every yielded current and validated
nonzero step have magnitude ≤ M; `|next| = |current + step| <= 2M < 2^54` safely
fits i64. Next may leave the safe domain, but the signed endpoint check happens
before conversion/yield and exits on its first failure. An out-of-domain next is
past the safe end in the direction of advancement, so it exhausts normally;
no subsequent addition uses it. See the
[full proof](memory-abi.md#private-range-record). Folding remains WP13 work and
must preserve invalid-constant construction traps at their original effect position.

**Implemented range-for coverage:**

- `tests/for_frontend.rs`: syntax/reserved names, iterable typing, statement versus
  value contexts, read-only ordinary/compound updates and mutable inner shadowing,
  body checking after divergent iterables, iterable transfers to outer loops and
  rejection without one, binding scope, unreachable value-break rejection.
- `tests/for_lowering.rs`: forged nodes/types/results/IDs/targets and read-only writes,
  exact error spans, initialization/scope restoration, planner slot types/preorder,
  Validator plus independent decoded cache/conversion/latch ordering, and rejection
  parity across export selections and all optimization levels without mutating input.
- `tests/for_loops.rs`: default checks for deterministic output, owned import
  snapshots, export indices/body preservation, helper counts and no-reuse scratch.
  Ignored bounded Node cases use an independent BigInt progression/membership
  reference for ±safe limits, large steps and normal exhaustion, canonical +0
  (`Object.is`), invalid construction/effect order, once-only iterable effects,
  nested break/return/continue, consumed-record validation even for empty ranges,
  and cached fields before body effects. All optimization levels are covered.

These suites check unexported, unreachable, and runtime-skipped children. Node
execution retains the ignored-test policy and bounded subprocess helper; default
Cargo success alone does not establish runtime coverage.

**Implemented numeric-array iteration checkpoint (issue #5):** Accept `number[]`
or `range` with Never compatibility; reject other iterable types. Reuse the parser
and raw/typed For shapes. Evaluate once in the enclosing context before the new
binding/loop is active; preserve iterable transfers, return/divergence, body checks,
read-only `number` bindings and mutable inner shadowing. Empty arrays preserve the
same conservative return analysis as empty ranges.

After full `ValidateArray`, including for an empty array, cache its handle and
unsigned `u32` length once. Use unsigned index tests and existing checked
`ArrayAddress` followed by `f64` load each iteration, without snapshotting elements
or caching a raw payload pointer. Alias mutations to unvisited elements are visible;
source-variable reassignment does not replace the captured handle. Continue exits
the body latch to increment/retest; break/return skip advancement. ABI and helper
order are unchanged; array-only iteration needs no range helpers. Array coverage
lives in `tests/array_for_loops.rs`, alongside shared frontend/forged-lowering
contracts in `tests/for_frontend.rs` and `tests/for_lowering.rs`. This records test
locations, not a claim that runtime validation or release gates passed.

**Implemented adapter/interaction checkpoint (issue #6):** CLI/browser adapters
compile both iterable types through the shared core. The CLI maps `range` host
declarations to core unsupported-ABI rejection with CLI guidance; the browser
reports an actionable private-type host-registration error. `tests/cli.rs` and
`tests/browser.cjs` add coverage across optimization levels for internal range
signatures under explicit selection, range import/selected-export rejection,
mandatory memory exports, diagnostics, and legacy ABI preservation. Cross-feature
cases compose ranges/iteration with block/if/value loops, aliases, compounds,
abrupt flow, and observable trap/effect order. `tests/wp5_interactions.rs` adds
import-snapshot, independent decoded layout/helper and export-independence checks,
skipped/unreachable/unexported children and forged-metadata rejection, plus bounded
Node effects/aliases/transfers/traps at all four optimization levels.

**Release acceptance:** Implementation and all release gates are complete. Exact
commands/results in [§5](#5-documentation-and-validation-workflow) include the
offline/no-install browser rebuild, 22 browser groups, and the full ignored-test
run with no remaining ignored cases. Ordinary default Cargo tests retain the
ignored policy. Rebuild bindings after any subsequent core/browser changes before
adapter validation. External range signatures, range equality, and allocation-free
range optimization are out of scope.

### WP6 — Named/default parameters (spec §14): implemented

**Entry:** WP5; raw-call metadata/API gate. Preserve a canonical full-arity typed
call/signature. Add argument names and trailing-only defaults to raw syntax/signature
metadata. Resolve defaults in declaration scope against earlier formals, never caller
locals/later formals. Use the shared initial whitelist specified in spec §14.2:
scalar literals, permitted parameter reads, existing type-valid unary/arithmetic/
comparison/equality/logical operations, and positional builtin `len`. Existing string
and array equality are allowed. Reject indexing, array/range construction, membership,
blocks/if/loops/match, writes, and user/host calls, even in runtime-skipped operands. Pure
means no writes or runtime allocation, not absence of descriptor traps. Reuse this
policy for WP8 with all formals readable there; no user-function purity analysis.

Normalize calls with hygienic typed sequencing: explicit arguments once in written
order into typed slots, omitted defaults afterward in formal order using bound values,
then direct full-arity call. Retain origin spans and check synthetic slot ownership.
Named arguments initially target user declarations only; reject named host and builtin
arguments because neither has a formal-name contract. Raw exports still require all
machine arguments.
**Exit tests:** proposed `tests/named_defaults.rs`; duplicates/unknown/missing names,
positional-after-named, nontrailing defaults, forbidden calls/later-name reads, forged
normalized calls, decoded full-arity order and runtime lexical/evaluation effects.
Test defaults per invocation, forward calls, host reentrancy and adapter raw arity.

**Release status:** issues #10/#12 implement named calls and trailing pure defaults;
#13 completes native/browser integration coverage and the WP6 release gate.
Dedicated coverage is in `tests/named_defaults.rs`, `tests/default_lowering.rs`,
`tests/default_runtime.rs`, the named-call suites, and `tests/cli.rs`.
See [the release evidence](#issue-13-wp6-release-validation). Earlier independent
worktree development checks remain historical in [the handoff](wp6-wp7-development.md#validation).

### WP7 — Acyclic function-result inference first (spec §9): implemented

**Entry:** WP6 and optional raw result-annotation API gate. Collect declarations,
build dependencies with standard collections, resolve acyclic functions in stable
dependency order, then analyze/output bodies and IDs in source order. Stage unresolved
signatures privately; only concrete types enter typed AST and layout. Preserve existing
first-error ordering for explicitly annotated old programs; pin inference diagnostics.
For recursive SCCs conservatively require explicit result annotations on every member
in the initial release, including anchored/self recursion. Do not add a speculative
fixed-point solver. Infer void only under the specified no-value-return rules; unify
reachable explicit returns, check all unreachable source and definite return. If value-return
syntax exists but no reachable value-return supplies a concrete type, require an explicit
result annotation rather than guessing void. Thus `fn spin() { return loop {}; }` needs
an annotation, while `fn spin() { loop {} }` can infer void. Never is not a function ABI type.
**Exit tests:** `tests/result_inference.rs` and `tests/result_inference_chains.rs`;
chains/diamonds/forward calls,
void, mixed returns, missing return, named tails, annotated recursion and SCC errors.
Cover Never-only value returns requiring annotations versus divergence without value-return syntax.
Forge result/body mismatches; decode signatures and execute chains. Compare inferred
versus explicit ABI/bytes at all levels and native/browser diagnostics.

**Incremental #11 integration:** initial results without unresolved callee
dependencies are combined with released WP6 on `develop/wp7-initial-inference`,
after #13's gate. The original 34-test suite, dedicated named/default interaction
and adapter tests, and combined offline/runtime/browser checks passed; see
[the incremental contract and evidence](result-inference-development.md).
At that historical #11 checkpoint, unannotated dependencies remained rejected.
That evidence completes only #11's incremental scope, not the full WP7
integration/release gate.

**Issue #14 implementation complete:** acyclic chains/diamonds now resolve in
stable dependency order with source-order ties among currently ready functions.
Recursion uses the full lexical call graph separately from scheduling; annotated
barriers do not hide recursive omitted results. Owned concrete signatures, source
IDs, typed-function order and Wasm layout remain unchanged. Diagnostics stage
source-order declarations/defaults, source-order recursive annotation errors,
stable dependency-order inference, then source-order final body checks. See
[the #14 contract and test coverage](result-inference-development.md#issue-14-acyclic-chains-and-diamonds).
**Issue #15 combined acceptance complete:** named/default calls through inferred
forward chains/diamonds have dedicated source, checked-input, decoded ABI, and
bounded effect/alias/trap/transfer coverage. Extended CLI/rebuilt-browser cases
use the same inferred named/default diamond. See [the interaction contract and
validation evidence](result-inference-development.md#issue-15-combined-nameddefault-and-inference-acceptance).
**Issue #16 release acceptance complete:** native CLI and current-source browser
coverage pins concrete/full-arity ABI, bool normalization, string/array descriptors,
private range export/host restrictions and import snapshots/order. Cross-adapter
rejections cover conservative recursion, skipped/unreachable calls, lexical shadowing,
incompatible returns, incomplete paths, implicit tails, Never-only value returns and
invalid unused defaults. Existing forged-result/normalization, decoded instruction,
legacy fixture and combined effect suites pass. See [exact release evidence](#issue-16-wp7-release-validation).
#11's historical commands remain evidence only for that incremental checkpoint.

<a id="wp8--whitelist-contracts-spec-13"></a>

### WP8 — Whitelist contracts (spec §13): implemented

**Implementation and release validation complete.** Native, freshly rebuilt browser,
full ignored runtime and Clippy gates passed. See [WP8's separate release evidence](#wp8-release-validation);
historical WP5–WP7 evidence retains its original scope.

**Frozen contracts:**

- Named declarations accept ordered `requires` expressions after an optional result
  annotation and before the body. `requires` is reserved in source and host-language
  names, not arbitrary nonempty host module/field names.
- Raw `Function.contracts: Vec<ContractExpression>` and typed
  `TypedFunction.contracts: Vec<TypedContract>` contain `value` and expression-only
  UTF-8 byte `span`. Existing constructors supply `vec![]`. No new IR or per-expression
  span field is introduced.
- Require exact `bool`; reuse WP6's narrow default whitelist with all callee formals
  readable. Reject body/caller-local reads, user/host calls, writes, indexing,
  array/range construction, membership, blocks/if/loops/match, and normalized calls,
  including in runtime-skipped children. Pure expressions may read shared arrays
  or trap; they perform no writes or runtime allocation.
- Source-order declaration checking handles each function's parameter types/defaults
  then contracts before inference/body checking. Contract number literals use the
  same syntax/finite-representability rules as defaults; malformed/overflowing
  literals fail semantically at the declaration origin, underflow is allowed.
  Ordinary body-literal stages remain unchanged. False conditions do not affect
  inferred results, body checking, or missing-return rules.
- Invocation order is explicit arguments in written order, omitted defaults in formal
  order, all callee parameter validation, contracts sequentially, then body. Validate
  even parameters not read by contracts, before the first condition. Apply this entry
  path to internal, recursive, and raw export calls; raw exports remain full-arity
  with no host-generated defaults.
- False executes standard Wasm `unreachable`. No custom failure metadata, trusted
  refinement facts, constant folding or check elimination is implemented. Exact
  optimization-level output/diagnostic parity remains required. Feature-free bytes,
  import snapshots/order, export selection and helper order remain compatible.
- The shared structural walker visits contracts in declaration order before the body.
  Local, temporary, memory, range and remainder policies remain separate; the private
  checked boundary validates forged contracts and lowers every function, even unexported.

**Actual suites:** `tests/contracts_frontend.rs` covers syntax, expression-only spans,
reservation, scope, whitelist/types, declaration staging, literals and inference/return
interactions. `tests/contracts_lowering.rs` covers forged metadata/reads/annotations,
`wasmparser::Validator`, independent decoded entry order, helper/layout discovery,
feature-free compatibility and optimization parity. `src/codegen/contracts_tests.rs`
covers ordered structural traversal, early exit and independent planner discovery.
`tests/contracts_runtime.rs` covers bounded Node execution, written/default/entry/body
order, sequential and logical short-circuit traps, all-parameter validation, shared
arrays, raw exports and recursion at every level. `tests/contracts_adapters.rs` covers
CLI bytes/WAT, diagnostics/origins, host-name reservation, output preservation and
recovery, plus rebuilt-browser equivalence/raw calls. Node/browser cases retain their
ignored-test policy; runtime/browser release coverage is established by the explicit
rebuilt-browser and include-ignored runs recorded in §5, not default Cargo success.

**Future only:** local refinement, constant-check simplification, optional debug
metadata and proof-based check elimination require separate design/authorization;
none is part of WP8. WP9 match is implemented with [release gates passed](#wp9-release-validation);
WP10 named-only pipelines are implemented with [release gates passed](#wp10-release-validation);
WP11's #29–#33 local callable/shared parameter/let capture, captured numeric
compound and nested capture/live callable-context forwarding subsets are implemented
in the frontend/backend; [#32 validation passed](local-callable-interface.md#issue-32-validation-2026-10-14); [#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14);
#33 implements [read-only iteration captures](local-callable-interface.md#implemented-read-only-iteration-captures-33) ([validation passed](local-callable-interface.md#issue-33-validation-2026-10-15)); combined nested/transitive iteration-capture convergence, full WP11 release and WP12 placeholders remain deferred.

### WP9 — `match` statements/values (spec §8): implemented

**Entry:** WP8 and existing result/flow/range machinery. Reserve `_` as wildcard here
if not already reserved; reject standalone binding names across source/host APIs.
Add literals, boolean, relational and constant interval patterns; interval patterns
include fractional members and are not stepped range membership. Initially require
final wildcard for number/string and exhaustive boolean cases in either context.
Use ordered comparison chains and existing string content equality; evaluate scrutinee
once, only selected arm at runtime, all arms statically. No br_table optimization or
new warning framework; defer duplicate/unreachable warnings unless one already exists.
**Implementation and release validation complete.** Frontend/backend, bounded
runtime, native adapters and current-source rebuilt-browser gates passed on
2026-10-13; see [the parent-run evidence](#wp9-release-validation). The agreed
design interview details below remain frozen.

**Agreed implementation scope (design interview; implemented):**
- Restrict patterns to wildcard, exact number/string/bool literals, numeric
  comparisons, and constant numeric intervals. No bindings, guards, destructuring,
  or-patterns, optimizations, or new duplicate/unreachable-arm warnings.
- Normally completing scrutinees must be `number`, `bool`, or `string`; reject
  Void and retain Never compatibility with unconditional arm checking, as detailed
  below.
- Numeric pattern constants are decimal literals with optional unary minus, not
  arithmetic, calls, or names. Preserve finite-literal representability checks;
  interval endpoints may be fractional and need not be safe integers.
- Bare match follows bare-if statement/tail classification: statement contexts
  discard arm results; a final bare match in a value block supplies its tail;
  nonfinal or semicolon-terminated matches are statements. Parenthesize the whole
  match for arithmetic continuation in these positions. No newline heuristics or
  implicit function returns. Arm syntax and result joining are detailed below.
- Match remains outside the shared defaults/contracts pure-expression whitelist,
  including frontend and forged-typed backend checks.

**Agreed arm and coverage rules (design interview; implemented):**
- Require at least one arm. Each arm is `pattern => expression`; commas separate
  all arms, including block-expression arms, with an optional trailing comma.
  Each arm has its own lexical scope; declarations do not leak to later arms or
  beyond the match. Return/break/continue statements require an arm block, not a
  direct statement RHS; transfers still target the enclosing function/nearest loop.
- Interval `a..b` tests `a <= value && value < b`; `a..=b` includes both bounds.
  Reversed intervals and equal-endpoint exclusive intervals are accepted and never
  match. Reject `by`. Numeric equality identifies both signed zeros; NaN falls
  through numeric patterns to the wildcard.
- A wildcard may occur at most once, only last. Number/string matches require it.
  Boolean coverage accepts both literals, either literal followed by wildcard,
  or wildcard alone. Duplicate non-wildcard patterns and overlapping intervals
  remain accepted with first-match-wins behavior.
- When the scrutinee can complete, conservatively include every arm in result
  joins, return inference, and loop-transfer summaries. Do not prune using constant
  scrutinees, duplicate patterns, overlap, or earlier pattern coverage. Pattern
  coverage checking remains separate from flow analysis.
- Value matches join exactly matching concrete categories or all Void, with Never
  compatibility; all-Never arms produce Never. Statement matches discard arm
  results and permit heterogeneous tails while independently checking every arm.

**Agreed divergence and compatibility rules (design interview; implemented):**
- A Never scrutinee does not excuse invalid patterns, missing coverage, or structural
  value-arm result mismatches. Non-wildcard patterns establish a single domain
  (number, bool, or string); reject mixed domains and apply ordinary coverage rules.
  Wildcard-only matches need no concrete scrutinee domain. Check every arm, but
  compose flow after the noncompleting scrutinee: no arm contributes reachable flow
  and the overall match is Never.
- Reserve exact `match` and `_` for source binding/function names and registered
  host source-language names. Preserve `_temp` and arbitrary nonempty host module/
  field names. This deliberately breaks existing uses of these source identifiers;
  WP9 adds neither ignored bindings nor placeholder expressions.
- Preserve pattern-only spans and arm spans (pattern through RHS, excluding comma)
  in raw and typed records. Pattern errors use pattern spans; incompatible arm
  results use the offending arm span. Overall coverage errors retain the enclosing
  expression/statement location. Do not introduce general expression spans or alter
  existing diagnostics and first-error ordering for unchanged syntax.
- Release requires frontend/inference and forged-typed checks, Wasm validation plus
  independent decoded caching/result-region/branch-depth assertions, bounded runtime
  effects/overlap/string/fractional/NaN/transfer tests, optimization-level parity,
  and native/browser compatibility. Rebuild browser bindings before browser checks.
  Preserve ignored-runtime defaults, but execute runtime cases for release evidence.
  Missing prerequisites leave gates incomplete; do not install tooling or introduce
  dependencies without authorization.

**Implementation ownership:** `src/match_patterns.rs` shares pattern constants,
domains and coverage checks between semantic analysis and checked lowering.
`src/codegen/match_.rs` privately owns ordered numeric/string comparison chains,
scoped arm lowering and reconstructed results/flow. The structural walker visits
the scrutinee then every arm within separate block hooks; pattern-string discovery
and typed scrutinee scratch planning retain separate policies. No new ABI, helper
kind, IR, optimizer or warning framework is introduced.

**Actual test ownership:** `tests/match_frontend.rs` owns syntax/spans,
reservations, domains/coverage, scopes, structural joins and inference/transfer
checks. `tests/match_lowering.rs` owns forged trees, Wasm validation and independent
decoded scrutinee-cache/result-region/branch-depth/planner assertions.
`tests/match_runtime.rs` owns bounded effects, overlaps, strings, fractional/NaN
cases and transfers. `tests/match_adapters.rs` owns CLI bytes/diagnostics,
host-name reservations and current-source browser compatibility;
`tests/browser.cjs` includes the WP9 browser case. Runtime/browser cases retain
the default ignored policy. Optimization-level parity remains required; actual
release evidence is recorded in [§5](#wp9-release-validation), not inferred from
default Cargo success.

### WP10 — Named-only pipelines (spec §12)

**Implemented; #27 release gates passed:** left-associative named-user pipelines
below OR inject input into formal 0 and reuse ordinary positional-prefix/named/default
binding for remaining parameters. Injection occupies formal 0 even when defaulted;
naming it is a duplicate. Bare/empty stages are equivalent and use remaining defaults.
Final declared-Void stages require discarded results, propagated through discarded
block tails, if branches and match arms with transparent parentheses. Outer discard
does not authorize operands, call arguments, pipeline inputs or value-producing
contexts. Nonfinal declared Void is rejected even after Never. Expression if/match
retain structural joins; ordinary named/default calls remain unchanged.
Every concrete value type, including private internal ranges under compatible export
selection, is supported. Raw stages participate in inference/recursion discovery and
whitelist rejection; typed stages nest checked normalized direct calls with
`PipelineInput` origins. Discard permission remains source-semantic, without a typed
wrapper/public flag; independent backend injection/type/ownership/flow checks remain.
Public migration is in [the call interface](named-call-interface.md#wp10-pipeline-migration-issue-25).
The four `tests/pipelines_{frontend,lowering,runtime,adapters}.rs` suites cover the full
contract at the approved public seams. Node/browser cases remain ignored by default;
[release evidence](#wp10-release-validation) includes explicit bounded runtime and
current-source rebuilt-browser runs. #25/#26 records below remain historical subset
evidence, not the #27 full-release gate.

#### Issue #25 subset validation (2026-10-14)

The first source-normalization test failed at the `|>` token before implementation;
five bounded runtime tests likewise failed at pipeline syntax before integration.
The approved source/public typed compilation and CLI/current-source browser seams
are covered by 36 focused tests: 14 frontend, 12 lowering, 5 runtime, 5 adapters.
Six Node/browser cases remain ignored by default and were run explicitly. Decoded
assertions establish direct calls, once-only slots, argument/helper order, no
indirect calls/tables, export eligibility, and owned host snapshots after registry
mutation. No golden fixture was regenerated or dependency added.

Passed from the repository root with offline builds and installed tooling:

- `cargo check --offline --all-targets`
- `cargo test --offline --test pipelines_frontend --test pipelines_lowering --test pipelines_runtime --test pipelines_adapters -- --include-ignored`
- `cargo test --offline`
- `cargo clippy --offline --all-targets -- -D warnings`
- `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline`
- `node tests/browser.cjs pkg`
- `cargo test --offline -- --include-ignored` (after the current-source rebuild)
- `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings`
- Scoped Rust formatting and `git --no-pager diff --check`

Two-axis worktree review found zero hard standards breaches or implementation
defects. A pipeline snapshot/helper-order coverage gap was filled and follow-up
spec review found no remaining findings. A low-priority normalized-call finalizer
duplication observation remains deliberately unchanged. This validates #25's
subset, not the deferred named/default/final-Void slices or full WP10 release.

#### Issue #26 subset validation (2026-10-14)

Named/mixed-prefix/default stages reuse the ordinary semantic binder with formal 0
preoccupied. Source acceptance failed with `MissingArgument` and
`NamedArgumentsNotSupported` before integration; existing checked lowering required
no production changes. The four focused suites now contain 53 tests: 18 frontend,
20 lowering, 10 bounded runtime, and 5 adapters. Eleven Node/browser cases remain
ignored by default and passed explicitly against freshly rebuilt browser bindings.
No ABI, golden fixture, dependency, or final-Void discard policy changed.

Passed from the repository root with cached dependencies and installed tooling:

- `cargo check --offline --all-targets`
- The four focused `pipelines_*` suites, including explicit `--include-ignored` runs
- `cargo test --offline`
- `cargo clippy --offline --all-targets -- -D warnings`
- `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline`
- `node tests/browser.cjs pkg` — 25 current-source browser cases
- `cargo test --offline -- --include-ignored`
- `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings`
- Scoped Rust formatting and `git --no-pager diff --check`

The first adapter attempt used stale bindings and failed; it passed after the
current-source rebuild. A combined default-plus-ignored command reached its
120-second deadline during the ignored run; the standalone include-ignored retry
completed successfully within the same bound. Standards/spec review found zero
hard breaches or spec defects; its low-priority string-based test-category smell
was resolved with a local enum. This validates #26's binding subset, not deferred
final-Void support or full WP10 release completion.

**Entry:** WP9, normalized named/default calls from WP6. Add left-associative `|>`
below logical OR; stage resolves only to named declarations, never an expression
callee/value. Inject input into formal 0 before explicit stage args; reuse WP6 order
and defaults. Reject duplicate parameter-0 binding, wrong arity/types and nonfinal
void stages; final void allowed only in statement context. Do not introduce partial
application, placeholder args, callable values or higher-order collection intrinsics.
**Exit tests:** proposed `tests/pipelines.rs`; precedence/binding/forged normalized
calls, decoded direct calls/slots, once-only stage effects and default/contract order.
Include range-returning internal stages under compatible export selection.

**Confirmed design interview decisions (implemented by #25–#27):**

- Initial stages resolve only to user-defined named functions, not registered hosts
  or builtin `len`. Preserve ordinary lexical shadowing and `NotCallable` behavior;
  do not fall back to a function when a local binding shadows its name.
- Bare `input |> f` and `input |> f()` are equivalent: injection occupies formal 0
  even when it has a default, and remaining omitted formals use ordinary default
  rules. Required remaining formals must be supplied; explicitly naming formal 0
  is a duplicate binding. Zero-parameter functions cannot be stages.
- A final Void-producing stage is allowed only when its result is discarded.
  Propagate discard context through discarded block tails, if branches, and
  statement-match arms; parentheses do not change it. Value-producing contexts,
  operands, and call arguments receive no discard permission from an outer
  statement. Reject nonfinal declared-Void stages even after a divergent input.
- Preserve a dedicated raw pipeline expression with its input and ordered named
  stage records until semantic analysis. Stages retain their target, written
  arguments, and narrow origin spans; inference discovery visits stage dependencies,
  and the shared default/contract whitelist rejects pipeline syntax before erasure.
  Stage syntax is only a name or a named call, not an expression callee.
- Normalize typed stages into nested existing `TypedNormalizedCall` sequences.
  Add `CallBindingOrigin::PipelineInput` instead of treating injection as a written
  explicit argument. Checked lowering requires this origin to be unique, first,
  and mapped to formal 0; written explicit indices retain their zero-based meaning.
  Preserve existing ownership, type, flow, and earlier-formal-read checks. See
  [the representation decision](adr/0003-normalized-pipeline-injection.md).
- Preserve bare if/match/loop statement/tail classification. Require parentheses
  where piping the whole construct would otherwise cross that boundary; add no
  general postfix chaining or newline-sensitive parsing.
- Final-Void discard permission is source-semantic validation, not a guarantee of
  complete source-context reconstruction from forged typed trees. Checked lowering
  still validates injection, ownership, mapping, concrete types, flow, and user
  targets; Void cannot serve as an injected value for another stage. Do not add a
  typed wrapper or public discard-permission flag solely to retain source context.
- Spans are original UTF-8 byte offsets. Input spans include its whole written
  expression and parentheses; stage spans cover the name through any closing call
  parenthesis, excluding `|>` and surrounding trivia. Written arguments keep WP6
  spans; injected bindings use the preceding pipeline prefix; defaults retain
  declaration-expression spans. Stage errors use stage spans, explicit argument
  errors use argument spans, and injected type errors use prefix spans.
- After existing declaration checking and inference scheduling, check a pipeline's
  input first, then each stage in written order: resolve target/eligibility, perform
  WP6 structural binding checks with formal 0 occupied, analyze written arguments,
  instantiate defaults, and check result/context restrictions. Preserve ordinary
  call diagnostics. Never does not skip static checks; an earlier static error
  still terminates analysis.
- Use `tests/pipelines_frontend.rs`, `tests/pipelines_lowering.rs`,
  `tests/pipelines_runtime.rs`, and `tests/pipelines_adapters.rs` for separate
  acceptance/inference/spans/whitelist, forged-tree/decoded-Wasm, bounded effects,
  and CLI/current-source browser contracts. Runtime/browser cases remain ignored
  by default. Common offline release gates, rebuilt browser bindings, explicit
  runtime coverage, optimization-level parity, and range-returning internal stages
  under compatible export selection remain required.
- Freeze the public raw shape as `Expr::Pipeline { input: Box<Expr>,
  input_span: Span, stages: Vec<PipelineStage> }` with
  `PipelineStage { name: String, args: Vec<CallArgument>, span: Span }`.
  Parser-produced pipelines have at least one stage; bare and empty-parenthesis
  stages both have empty arguments. Derive injected-prefix spans from input start
  through the preceding stage end. The typed change is only the unit origin
  `CallBindingOrigin::PipelineInput`, without public stage indices or flow/discard
  annotations. Update exhaustive raw consumers and public migration documentation;
  retain existing normalized-call walker/planner traversal.
- Add `SemanticError::InvalidPipeline { message: &'static str, span: Span }`
  for host/builtin ineligibility, zero-parameter stages, nonfinal declared-Void
  stages, and final Void stages outside discard context. Reuse existing target,
  argument-binding, and type errors; malformed syntax stays a parser error, and
  malformed typed injection uses `CodegenError::InvalidNormalizedCall`. Add
  exhaustive span extraction and diagnostic message tests without a new error stage.

**Implementation ownership:** diagnostics and coverage form one combined scope,
including pipeline errors/messages/spans and all four focused test suites. This
ownership grouping does not merge the test files or weaken their acceptance gates.
The integration owner retains exclusive ownership of shared AST/parser/semantic
and stage-error files; the combined scope coordinates shared-file edits through
that owner rather than introducing overlapping writers.

All interview decisions above are confirmed and implemented. Issues #25/#26
supplied the value-producing binding subset; #27 completes final-Void discard support
and [combined WP10 release validation](#wp10-release-validation). Interview records
and historical subset runs are not substitutes for that release evidence.

### WP11 — Local-only shared-binding closures/direct helpers (spec §10)

The [WP11 design interview](wp11-local-closures-development.md) records accepted
scope, inference, identity, storage, diagnostic, and validation decisions. Final
shared understanding was confirmed on 2026-10-14. The interview itself does not
authorize implementation or establish release evidence. #29–#33 implement the
subsets below; the remaining full WP11 contract and release remain deferred.

**Implemented #29 subset:** Capture-free local declarations and lexical positional
calls use dedicated owned typed declarations, program-wide callable IDs and
owner-qualified capture interfaces. Context-free helpers are checked/cached through
`ValidatedProgram`, emitted in source depth-first preorder after existing runtime
helpers, with independent locals/temporaries/control contexts and no environment
or callable-only memory. Earlier context-free callable dependencies are supported.
#30 extends this baseline with shared captures. See [public migration and scopes](local-callable-interface.md)
and the separate [#30 validation record](local-callable-interface.md#issue-30-validation-2026-10-14);
the historical #29 results do not cover shared storage.

**Implemented #30 subset ([validation passed](local-callable-interface.md#issue-30-validation-2026-10-14)):** Capture immediate-
creator parameters and ordinary lets of all concrete types (`number`, `bool`,
`string`, `number[]`, `range`). Helper reads/assignment targets/indexed array bases
use `TypedExprKind::Captured { owner, id, name }`; creator accesses remain
`Identifier`. Metadata is unique and canonical by original owner/declaration order;
checked lowering must derive that order independently of forged sparse IDs. Preserve
initialization, shadowing, original type and shared ordinary assignment/indexed
mutation without snapshotting or cloning bodies. Cached concrete closure-result
inference and independent final body checks remain intact.

Eager parameter cells allocate in formal order after all validation/contracts,
eager let cells after completing initialization. Structural promotion can allocate
cells even when a later capturing declaration is skipped. Nonempty environments
allocate only at executed declarations; context-requiring helpers receive hidden
first `i32` environments (captures and, since #32, callable-context dependencies). Context-free helpers retain no callable-only allocation
or hidden parameter. The [private storage contract](memory-abi.md#private-closure-storage-30)
uses uniform eight-byte cells/slots and monotonic allocation, with possible memory
growth/traps and no rollback/reclamation/heap rewind. Hosts refresh views after
possible growth; persistent returned/retained handles keep existing ownership.

**Implemented #31 subset ([validation passed](local-callable-interface.md#issue-31-validation-2026-10-14)):** All five numeric compound operators
support captured scalar/indexed places and creator-side promoted bindings, including
before the capturing declaration. Reuse the existing checked place path: preserve
the original scalar cell address and old `f64` on the stack, or the checked element
address and old value in existing scratch, before RHS effects. Ordinary indexed
assignments also retain their pre-RHS checked address. Preserve numeric/mutability
checks, trap order and exact `%=` behavior; no new storage layout or public ABI.

**Implemented #32 frontend/backend subset ([validation passed](local-callable-interface.md#issue-32-validation-2026-10-14)):**
Forward original ancestor parameter/ordinary-let captures through every intermediate
helper, retaining owner-qualified identity, type, mutability and the original cell.
Forward original live environments for earlier visible context-requiring callable
dependencies, including helpers with no direct captures. Environments contain
canonical owner/declaration-ordered captures, then contextful dependencies in
source-preorder callable-ID order. Context-free dependencies occupy no slots and
add no hidden argument; either kind of context requires hidden first `i32`.
Independently check complete/unique/canonical capture and dependency metadata,
initialization, lexical availability and forged sparse IDs, including descendant-only
requirements. No public typed shape changes. #33 extends iteration capture as
specified below.
Full WP11 release remains deferred. [#30 subset validation passed](local-callable-interface.md#issue-30-validation-2026-10-14)
after review fixes, including current-source rebuilt-browser and full ignored
runtime coverage. That record includes exact commands/results and the re-reviewed
owner-qualified lookup/ordered bookkeeping fixes; it predates #31 and is not
captured-compound validation or a parent WP11 release claim.

**Implemented #33 read-only iteration-capture subset ([validation passed](local-callable-interface.md#issue-33-validation-2026-10-15)):**
Allow reads of range/array iteration bindings, recording `CaptureMutability::ReadOnly`.
Backend independently derives mutability from original `For` / `BindingKind::Iteration`
provenance and checks metadata before helper lowering. Reject ordinary and all
numeric compound writes even when skipped/unreachable. Existing promotion runs
after successful range endpoint-before-conversion yield or checked array element
load/local set, before the body: a fresh uniform eight-byte cell per yielded
captured binding, eager even for skipped declarations, none on empty/exhausted/failed
yields. Cells remain monotonic, never reused/reclaimed; environments allocate only
at executed declarations. Loop/control/alias semantics and AST/ABI shapes are
unchanged. See [the contract and validation record](local-callable-interface.md#implemented-read-only-iteration-captures-33).
Combined nested/transitive iteration-capture convergence and full WP11 release remain deferred.

The remaining paragraphs describe the **full WP11 target and exit**, including
features deliberately outside #29–#33; they are not claims of current support.

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

**Full WP11 exit tests (not satisfied by #29–#33):** extend
`tests/closures_frontend.rs`, `tests/closures_lowering.rs`,
`tests/closures_runtime.rs` and `tests/closures_adapters.rs`; all escape/alias restrictions,
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

### WP10 release validation

On 2026-10-14, issue #27 completed final-Void discard support and the combined
Diagnostics and Coverage acceptance gate for named-only pipelines. Validation ran
on Windows with Rust/Cargo 1.98.0, Node.js v24.12.0, wasm-pack 0.15.0, cached offline
dependencies and installed `wasm32-unknown-unknown`/wasm-bindgen tooling. The starting
commit was `b1e7920ff5752b357b641f78b00580ce3d9190cd`. No dependencies were added;
`pkg/` was rebuilt from current integrated source before browser and full ignored runs.

| Exact command (repository root) | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed after integration and the expanded frontend tests |
| `cargo test --offline --test pipelines_frontend --test pipelines_lowering --test pipelines_adapters` | Passed after semantic integration: 19 frontend, 25 lowering, 4 adapter tests; 2 adapters ignored |
| `cargo test --offline --test pipelines_frontend` | Passed after additional boundary/diagnostic/inference coverage: 23 tests |
| `cargo test --offline --test pipelines_runtime -- --include-ignored` | Passed: all 14 bounded Node tests at every optimization level |
| `cargo test --offline --test pipelines_adapters -- --include-ignored` | Passed: all 6 tests, including rebuilt-browser/native parity and bounded CLI execution |
| `cargo test --offline` | Passed complete default suite; Node/browser cases retain ignored-by-default policy |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current-source browser bindings without tooling installation |
| `node tests/browser.cjs pkg` | Passed: 25 browser cases against rebuilt bindings |
| `cargo test --offline -- --include-ignored` | Passed complete default-plus-ignored suite; no aggregate suite count asserted |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |

TDD captured the old final-Void rejection before semantic integration at the source,
decoded-lowering, bounded-runtime and adapter seams. A frontend assertion initially
used `ExprResult` instead of `Type` for `SemanticError::TypeMismatch.found`; correcting
the test restored typechecking and focused success. No fixtures were regenerated.
Scoped Rust formatting and `git --no-pager diff --check` passed. The two-axis review
found no documented standards breaches or spec defects; its low-priority duplicated
adapter-assertion heuristic was resolved with a local shared JavaScript assertion
function. The adapter include-ignored command (6 tests) and
`cargo clippy --offline --test pipelines_adapters -- -D warnings` passed again after
that test-only extraction; follow-up standards review found no remaining findings.

The four focused suites audit the full parent matrix: syntax/precedence/classification,
user-only eligibility/shadowing, binding/default origins, inference/recursion, purity,
Void/Never, exact UTF-8 spans and first-error ordering, forged metadata, independent
Wasm validation/decoding, deterministic layout/helper/import snapshots, export
selection/internal ranges and optimization-level parity. Runtime cases establish
once-only effects, shared mutation, default/parameter/contract/body and trap order,
caller transfers and suppression of later effects without suppressing static checks.
Adapters verify native/library bytes and diagnostics, current-source browser behavior
and unchanged raw full-arity ABI. Source discard permission is not claimed to be
wholly reconstructible from forged typed trees. No public discard flag, indirect call,
new ABI, optimizer or closure/placeholder support is introduced.

### WP9 release validation

On 2026-10-13, the parent-run results established that WP9's frontend/backend,
bounded runtime, native adapters and current-source rebuilt-browser release
gates passed. The parent reported Node.js v24.12.0, wasm-pack 0.15.0 and an
installed `wasm32-unknown-unknown` target. Builds used offline cached dependencies
and installed tooling; `pkg/` was rebuilt from current integrated source before
browser and full ignored execution. The commands/results below are supplied
parent-run evidence, not additional runs performed by the documentation owner.

| Exact command (repository root) | Actual result |
| --- | --- |
| `cargo test --offline` | Passed; Node/browser cases retain their default ignored policy |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed; rerun after integration fixes also passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed with wasm-pack 0.15.0 and the installed Wasm target; rebuilt current source |
| `node tests/browser.cjs pkg` | Passed: 25 browser cases against rebuilt bindings |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `cargo test --offline -- --include-ignored` | Passed after correcting the browser summary harness expectation described below; no aggregate full-suite count is asserted |
| `cargo test --offline --test match_frontend --test match_lowering --test match_runtime --test match_adapters -- --include-ignored` | Passed: 45 tests (17 frontend, 18 lowering, 6 runtime, 4 adapters) |
| `git diff --check` | Passed |

The initial full ignored run failed because `tests/browser.rs` still expected
24 browser cases after the new WP9 case raised the summary to 25. Updating that
harness expectation from 24 to 25 was followed by a successful full ignored rerun.
This records the resolved validation failure, not a failing WP9 release gate.

Frontend/lowering ownership and frozen semantics remain in [WP9 §4](#wp9--match-statementsvalues-spec-8-implemented).
`tests/match_runtime.rs` supplies bounded observable effects, first-match overlap,
string/fractional/NaN cases and transfers; `tests/match_adapters.rs` and
`tests/browser.cjs` establish native/current-source browser compatibility.
The focused include-ignored run and rebuilt-browser/full ignored gates supply
runtime evidence; ordinary default Cargo success alone does not. Ignored defaults,
optimization-level parity and sequential compiler planning remain unchanged.
WP10 is now implemented with [release gates passed](#wp10-release-validation);
WP11's #29–#33 subsets are implemented in the frontend/backend, including captured
numeric compounds and nested capture/live callable-context forwarding;
[#32 validation passed](local-callable-interface.md#issue-32-validation-2026-10-14).
[#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14). #33 implements [read-only iteration captures](local-callable-interface.md#implemented-read-only-iteration-captures-33) ([validation passed](local-callable-interface.md#issue-33-validation-2026-10-15)); combined nested/transitive iteration-capture convergence, full WP11 release and WP12 remain deferred; WP13 optimization requires separate authorization.
Earlier WP5–WP9 release records retain their historical scope and counts; see
[the separate #30 evidence](local-callable-interface.md#issue-30-validation-2026-10-14)
for shared-storage validation before #31, not captured compounds.

### WP8 release validation

On 2026-10-13, WP8's library, native CLI, checked lowering, runtime and rebuilt-browser
release gates passed on Windows with Rust/Cargo 1.98.0, Node.js v24.12.0 and wasm-pack
0.15.0. Builds used offline cached dependencies and available tooling; no dependencies
were added. `pkg/` was rebuilt from the WP8 source before browser or full ignored execution.

| Exact command (repository root) | Actual result |
| --- | --- |
| `cargo test --offline --test contracts_frontend --test contracts_lowering --test contracts_runtime --test contracts_adapters -- --include-ignored` | Passed: 27 frontend, 11 lowering, 7 runtime and 4 adapter tests, including rebuilt-browser parity |
| `cargo test --offline` | Passed; Node/browser cases retain their default ignored policy |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current WP8 source |
| `node tests/browser.cjs pkg` | Passed: 24 browser cases against rebuilt bindings |
| `cargo test --offline -- --include-ignored` | Passed; all ignored Node/browser cases executed |

Frontend coverage checks clause syntax/origins, reservation, whitelist/type/literal
errors, declaration precedence and inference isolation. Lowering coverage validates
Wasm and independently decodes parameter-validation/contract/body order, contract-only
helper/data discovery, forged metadata, sparse formal identities and feature-free bytes.
Two backend unit tests cover structural traversal and separate planner discovery.
Bounded runtime cases exercise named/default/recursive/raw calls, caller transfers,
shared-array mutation, IEEE edge cases, descriptor checks, ordered traps and body
suppression at all optimization levels. CLI/browser tests compare native bytes,
diagnostics, full arity, export selection, host-name reservation and recovery.

No check elimination, rich runtime metadata, ABI change, or WP9+ feature is included.
Historical fixtures were not regenerated. Issue tracker state was not changed, and
no changes were staged or committed.

### Issue #23 WP8 integrated release verification

On 2026-10-13, the integrated scalar (#20), named/default/inference (#21), and
collection (#22) slices passed the release gates again on Windows. The starting
commit was `54539b1251824df8396bc1cf29851341fed89cdb`. Available tooling included
Rust 1.98.0, Node.js v24.12.0, and wasm-pack 0.15.0; all Cargo/build commands used
cached dependencies offline. Browser bindings were rebuilt before browser and
include-ignored execution, using `--mode no-install` and `--no-opt`. No new
prerequisites, dependencies, compiler behavior, or generated-fixture updates were
required.

| Exact command (repository root) | Actual result |
| --- | --- |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current source before browser execution |
| `cargo test --offline --test contracts_frontend --test contracts_lowering --test contracts_runtime --test contracts_adapters -- --include-ignored` | Passed: 27 frontend, 14 lowering, 8 runtime, and 4 adapter tests |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline` | Passed; default Node/browser ignored policy retained |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `node tests/browser.cjs pkg` | Passed: 24 cases against rebuilt bindings |
| `cargo test --offline -- --include-ignored` | Passed; full suite including ignored Node/browser cases |

Adapter diagnostic fixtures now distinguish UTF-8 byte spans from character
columns and caret widths using same-line Unicode, tabs, and CRLF. They also cover
a contract-bearing function whose ordinary body literal fails checked lowering,
retaining the backend variant, statement origin, CLI destination preservation,
and native/browser byte/WAT recovery. This body diagnostic does not introduce
expression-level body spans or expose forged typed input through browser APIs.
The [constructor migration guide](named-call-interface.md#wp8-contract-constructor-migration)
now explicitly lists both contract collections and invocation order.

Runtime subprocesses retain the bounded harness and ignored policy. Deadlines are
not guest fuel or process-tree cancellation. Raw exports remain full-arity;
preconditions use ordinary traps without rich metadata or check elimination.
At this WP8 checkpoint, WP9–WP12 remained proposed; WP9's current integrated
status and completed release gates are recorded in §4 and §5. WP13 still requires separate
authorization. Earlier release evidence above is historical and unchanged; this
record does not assert that the current work was committed or merged, or close
parent issue #19.

### Issue #13 WP6 release validation

On 2026-10-12, WP6's library/native CLI/rebuilt-browser acceptance and release
checks passed on Windows with Node.js v24.12.0 and wasm-pack 0.15.0. All builds
used cached dependencies and installed tooling, without installation or fetching.
At this WP6 checkpoint, WP7 remained isolated and unreleased. Subsequent #11
incremental integration is recorded above; it does not complete the full WP7 gate.

| Exact command (repository root) | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed before and after integration-test edits |
| `cargo test --offline --test named_defaults --test default_lowering --test named_arguments --test named_call_lowering` | Passed: 33 default, 3 initial lowering, 9 named frontend, 11 named lowering tests |
| `cargo test --offline --test default_lowering` | Passed: 4 tests after adding unreachable/skipped/unexported forged-default coverage |
| `cargo test --offline --test cli named_defaults -- --include-ignored` | Passed: 3 new adapter tests |
| `cargo test --offline --test cli -- --include-ignored` | Passed: 51 tests, including bounded Node execution |
| `cargo clippy --offline --test cli -- -D warnings` | Passed |
| `cargo test --offline --test default_runtime --test named_call_runtime -- --include-ignored` | Passed: 4 default and 5 named-call tests; its early existing-package browser result is not release evidence, superseded by the current-source rebuild/full ignored run below |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current source before browser/full ignored execution |
| `node tests/browser.cjs pkg` | Passed against freshly rebuilt bindings |
| `cargo test --offline -- --include-ignored` | Passed; all ignored Node/browser cases executed, including the expanded 4-test default runtime suite and 51-test CLI suite |
| `cargo test --offline` | Passed; Node/browser cases retain their default ignored policy |
| `cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main` | Passed |
| `node runtime/run.cjs target/array-sum.wasm` | Passed; returned `65` |

Acceptance tests exposed no production defect, so no adapter/compiler patch or
fixture regeneration was needed. New CLI checks compare exact library bytes/WAT,
full-arity decoded signatures, host import order, selected/no exports and required
memory exports. Diagnostics cover original CRLF/Unicode/tab byte origins and
unchanged destinations on failure. Rebuilt-browser tests cover forward reordered
named calls, dependent defaults, raw full arity, binding errors and recovery at all
levels. Forged-default tests add hidden/unreachable/short-circuited checks with
exact declaration origins. Existing suites retain caller transfers, nested regions,
recursion/reentrancy, scalar capture, array aliases, descriptor traps, special
numbers, legacy diagnostics/fixtures and optimization parity.

Standards and Spec reviews of the uncommitted diff against pinned base
`5b60b446e6dd732628dc4dcb4f29554c5bd182b4` each reported zero outstanding
actionable findings. Review independently reran the 4 lowering, 9 default/named
runtime and 3 CLI adapter tests and checked 147 local Markdown links/anchors.
Scoped `rustfmt --edition 2024 --config skip_children=true tests/cli.rs` and
`rustfmt --edition 2024 --config skip_children=true tests/default_lowering.rs tests/default_runtime.rs`
completed successfully. `git --no-pager diff --check`, diff/stat and final status
review passed. At that validation checkpoint, nothing was staged or committed;
parent #9 and the isolated WP7 worktree remained unchanged.

### Issue #16 WP7 release validation

On 2026-10-13, WP7's core/native CLI/current-source browser acceptance and release
checks passed on Windows with Rust 1.98.0, Node.js v24.12.0, wasm-pack 0.15.0,
cached Cargo dependencies and the installed `wasm32-unknown-unknown` target.
The standalone `wasm-bindgen` command was absent from `PATH`, but the prescribed
`--mode no-install --offline` wasm-pack build succeeded with cached compatible
tooling. No missing prerequisite prevented coverage. WP6 had already passed #13.

Acceptance adds `tests/result_inference_release.rs` (seven tests, one normally
ignored Node case), two browser harness groups and a runnable inferred named/default
diamond. Existing inference/default/combined, forged-tree, independent decoded ABI
and legacy fixture suites were preserved. See [the coverage inventory](result-inference-development.md#issue-16-nativebrowser-release-acceptance).
Production, optimization, dependencies and golden fixtures are unchanged.

| Exact command (repository root) | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed during integration and after the browser-wrapper update. |
| `cargo test --offline --test result_inference_release` | Passed: six active tests; one Node case ignored by default. |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current-source bindings before browser/full ignored execution. |
| `node tests/browser.cjs pkg` | Passed: 24 groups, including two new WP7 groups at levels 0–3. An initial explicit-void fixture was corrected to the existing statement-call contract. |
| `cargo test --offline --test result_inference --test result_inference_defaults --test result_inference_chains --test result_inference_combined --test result_inference_combined_checks --test result_inference_adapters --test result_inference_release -- --include-ignored` | Passed: all seven suites, including current-source browser cases and bounded Node runtime. |
| `cargo test --offline` | Passed; Node/browser cases retained their normal ignored policy. |
| `cargo test --offline --test browser -- --include-ignored` | Passed after updating the wrapper's required completion summary from 22 to 24 groups. |
| `cargo test --offline -- --include-ignored` | Passed: complete suite including all normally ignored runtime/browser tests. The initial run caught the stale wrapper summary; the full rerun passed after its update. |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed, including after the wrapper update. |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed. |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed. |
| `cargo run --offline -- examples/inferred-defaults.was -o target/inferred-defaults.wasm --export main` | Passed. |
| `node runtime/run.cjs target/inferred-defaults.wasm` | Passed; returned `584` (`236 + 348`). |
| `rustfmt --edition 2024 --config skip_children=true tests/browser.rs tests/result_inference_release.rs` | Passed; only edited Rust files formatted. |
| `node --check tests/browser.cjs` | Passed. |
| Read-only inline Python local Markdown path/anchor check | Passed: 11 changed Markdown files, 179 local link occurrences (86 anchors), zero errors. |
| `git --no-pager diff --check` and final diff/status/index review | Passed; only assigned tests/docs/grammar/example changes, no staged files. |

Independent Standards review found zero documented violations and one optional
low-priority test-local artifact-assertion duplication; it was retained to avoid
incidental helper churn. Spec review found a stale WP7-isolated sentence in §14;
that was corrected to integrated support and linked release evidence. Follow-up
Spec review found zero outstanding findings. No goldens were regenerated.

WP7 is implemented and its release gate is complete; every recursive component
member still needs an explicit result in the initial release. WP8–WP12 remain
proposed, and WP13 requires separate authorization. Parent #9 and issue tracker
state remain untouched; no changes are staged or committed.

### Historical WP5 release validation

**Issue #6 release validation (2026-10-12):** WP5 implementation and all release
checks below passed on Windows with Node.js v24.12.0, wasm-pack 0.15.0, cached Rust
dependencies, and the installed `wasm32-unknown-unknown` target. Issues #2–#5 were
confirmed closed with all acceptance gates checked; their frontend, forged-tree,
decoded-layout, compatibility, and runtime suites passed again in the full runs.
WP6–WP13 remain proposals, not part of this release.

| Exact command (repository root) | Result |
| --- | --- |
| `cargo test --offline --test cli` | 42 passed; 6 Node cases ignored by default |
| `cargo test --offline --bin was-compiler cli::` | 13 passed |
| `cargo test --offline --test wp5_interactions -- --include-ignored` | Runtime and rejection cases passed in the focused run; the initial layout assertion incorrectly expected a string-validation call in an array-only body. Corrected expectation; all 3 cases passed in the full ignored run below. |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current bindings before browser/full ignored validation, without enabling tooling installation or fetching dependencies |
| `node tests/browser.cjs pkg` | 22 groups passed, including adapter diagnostics and cross-feature execution at all optimization levels |
| `cargo test --offline` | Passed; Node/browser-dependent cases retained their default ignored policy |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `cargo test --offline -- --include-ignored` | Passed; all previously ignored Node/browser cases executed, including the rebuilt browser harness and all WP5 runtime suites; no ignored cases remain in this run |
| `cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main` | Passed |
| `node runtime/run.cjs target/array-sum.wasm` | Passed; returned `65` |
| `git --no-pager diff --check` | Passed |

A standalone `wasm-bindgen --version` probe was unavailable on `PATH`; this did
not block wasm-pack's successful offline/no-install use of compatible cached
tooling. No release prerequisite or runtime/browser coverage remains missing.
Earlier red adapter tests confirmed the old unknown-type errors; the final rebuilt
package validates the new private-range guidance. The two-axis review found
browser export remedies still naming only Rust APIs and range host parsing
bypassing core registration precedence. Both findings were fixed test-first:
missing JavaScript remedies and mixed-invalid registration regressions failed
before fixes, then the rebuilt 22-group browser suite passed. Follow-up standards
and spec reviews reported no outstanding actionable findings. Final rebuild and
full checks were repeated after these fixes. An initial interaction-test
compile also caught a section-range conversion and statement-variant naming error;
both were corrected before the successful suites. No golden fixture was regenerated.
Local Markdown links/anchors were checked across the updated documents.

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
unit/test filters and implemented suites, then add proposed suites at their package
boundary.
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
tests are normally unnecessary. The commands above are repeatable validation gates
for implementation changes, not a per-run validation log.
