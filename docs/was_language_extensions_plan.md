# WAS language extensions implementation plan

**Status:** WP1–WP5 implementation complete, including issue #6 adapter/interaction support; all WP5 release gates passed (exact evidence in §5). WP6–WP12 remain proposed; WP13 requires separate authorization.
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
  Function results are concrete and explicitly annotated; calls resolve names/IDs.
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
support; all WP5 release gates passed as recorded in §5. WP6–WP12 remain proposed, and
WP13 requires separate authorization. New keywords require matching host source-name
validation, not restrictions on arbitrary
nonempty host module/field names. Reserve standalone `_`
when its syntax arrives, reject its bindings, retain `_temp`, and keep `by` contextual.

## 2. File ownership and change boundaries

These paths exist today; ownership denotes responsibility, not mandatory edits for every package.

| Existing location | Responsibility for extension work |
| --- | --- |
| `src/tokens.rs`, `src/lexer.rs` | Longest-match tokens, trivia, range/decimal boundaries, original byte spans |
| `src/parser.rs`, `src/ast.rs`, `grammar.bnf` | Contextual grammar, optional tails/results/defaults, raw syntax API |
| `src/semantic.rs`, `src/semantic_tests.rs`, `src/typed_ast_tests.rs` | Scope/type/flow rules, call binding, inference and captures |
| `src/typed_ast.rs` | Concrete bindings/signatures, resolved nodes/IDs, origin information |
| `src/flow.rs` | Shared private completion sets, independent of static traversal |
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

**Proposed new files, not existing implementation:** `src/semantic/inference.rs` in WP7 and `src/semantic/captures.rs` in WP11 if those policies
outgrow `semantic.rs`; `src/codegen/closures.rs` in WP11 for direct helpers/cells.
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
5. WP11 represents callable bindings in dedicated symbol/helper metadata, not in
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

Implement WP6 and WP7 only after final design confirmation. WP8–WP12 remain future
waves; WP13 remains separately authorized. Proposed module paths below are write-scope
assignments, not existing implementation or a reason to create empty scaffolding.

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
array-slot order/encoding. Captured scalar mutation becomes testable in WP11.
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
blocks/if/loops, writes, and user/host calls, even in runtime-skipped operands. Pure
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

### WP7 — Acyclic function-result inference first (spec §9)

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
**Exit tests:** proposed `tests/result_inference.rs`; chains/diamonds/forward calls,
void, mixed returns, missing return, named tails, annotated recursion and SCC errors.
Cover Never-only value returns requiring annotations versus divergence without value-return syntax.
Forge result/body mismatches; decode signatures and execute chains. Compare inferred
versus explicit ABI/bytes at all levels and native/browser diagnostics.

**Incremental #11 development:** initial results without unresolved callee
dependencies are implemented only in `target/wp7-inference` on
`develop/wp7-initial-inference`. The focused 34-test suite and full development
checks passed; see [the isolated contract](result-inference-development.md).
Unannotated dependencies remain rejected (chain scheduling is #14). WP7 integration
and release remain separate: #13's WP6 gate passed, but the isolated WP7 tree has
not been integrated or released; no isolated validation establishes its combined gate.

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

### Issue #13 WP6 release validation

On 2026-10-12, WP6's library/native CLI/rebuilt-browser acceptance and release
checks passed on Windows with Node.js v24.12.0 and wasm-pack 0.15.0. All builds
used cached dependencies and installed tooling, without installation or fetching.
WP7 remains isolated in `target/wp7-inference`, not integrated or released.

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
