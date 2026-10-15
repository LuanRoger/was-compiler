# Local callable interface — issues #29–#33

## Frozen public interface and implementation ownership

Issue #29 implements capture-free local callable declarations and positional
lexical invocation. Issue #30 adds shared captures of immediate-creator parameters
and ordinary lets of every concrete type: `number`, `bool`, `string`, `number[]`,
and `range`. Captures refer to original shared bindings, never snapshots.
Issue #31 adds all five numeric compound operators on captured places, including
creator-side promoted bindings. Issue #32 implements nested original-binding
capture forwarding and earlier context-requiring callable dependencies in the
frontend/backend; [validation passed](#issue-32-validation-2026-10-14).
The parent [issue #28](https://github.com/LuanRoger/was-compiler/issues/28)
defines the full contract. #33 adds [read-only iteration captures](#implemented-read-only-iteration-captures-33).
Combined nested/transitive iteration-capture convergence and full WP11 release remain deferred.
[#30 subset validation passed](#issue-30-validation-2026-10-14); that record predates
#31 and does not validate captured compounds. The historical #29 evidence below
predates shared capture storage.

The confirmed test seams are `Compiler::analyze`, `Compiler::compile_source`,
`Compiler::compile` on mutable public typed input, and the existing CLI adapter.
Runtime and rebuilt-browser tests use the existing bounded subprocess support
and remain ignored by default. Independent Wasm decoding supplements validation and runtime effects.

Public AST consumers must migrate as follows:

- Raw `Expr::Call` adds its identifier-through-closing-parenthesis `span`.
- Raw `StatementKind::Let` adds the identifier-only `name_span`.
- Raw `Expr::Lambda(Lambda)` owns explicit parameters, `LambdaBody::Expression`
  or `Block`, and lambda/body byte spans. Parentheses are transparent.
- `CallableId` is program-wide in source depth-first preorder, independently of
  named result-inference scheduling. `BindingOwner::{Function, Closure}` qualifies
  original bindings; variable and loop identities remain owner-local.
- `TypedStatementKind::Closure(TypedClosure)` owns its parameters, concrete result,
  `TypedClosureBody`, declaration/lambda/body spans, captures and dependencies.
  `owner` identifies the enclosing owner; the body owner is `Closure(id)`.
- `TypedExprKind::LexicalCall { callable, args, span }` is a dedicated positional
  invocation. No typed lambda value, callable `Type`, or extra public user function
  is introduced.
- `TypedCapture` identifies an original owner/variable/type/mutability.
  `CaptureMutability` distinguishes mutable locals/parameters and read-only for
  bindings; #30 accepts mutable immediate-creator parameters/ordinary lets,
  #32 forwards original ancestor parameter/let bindings through every intermediate
  helper, and #33 records `ReadOnly` for captured iteration bindings.
  Captures are unique and canonical by original owner/declaration order, with
  parameters before locals. Checked lowering derives declaration order independently
  of numeric `VariableId` values, since public typed input may use sparse IDs.
- #30 adds `TypedExprKind::Captured { owner: BindingOwner, id: VariableId,
  name: String }`. Migrate exhaustive matches and distinguish captured reads,
  assignment targets and indexed array bases from owner-local `Identifier` nodes.
  Creator accesses remain `Identifier`, even when their storage is promoted.
  Matching local IDs in different owners do not identify the same binding.
- `dependencies: Vec<CallableId>` records complete, unique earlier visible callable
  identities in source-preorder ID order, including transitive requirements of
  nested helpers. #32 supports context-requiring dependencies without changing
  public shapes; context-free dependencies remain lexical permissions only.

Backend planning borrows declaration-owned bodies, emits direct helpers after all
existing runtime helpers, and validates all owners, identities, lexical availability,
results, calls and transfers through existing checked lowering. Context-free helpers
have only explicit parameters and allocate no callable environment. Ordinary body
operations and handle signatures may still require memory.

## Implemented shared-capture subset (#30)

Outer and helper reads/writes observe the same binding; ordinary indexed array
mutation and array-binding reassignment retain existing alias/type rules. Capture
resolution preserves declaration-time initialization and shadowing. Helpers keep
independent parameter/local/return/loop contexts. #30 initially supported only
immediate-creator captures and earlier context-free callable composition; #32
extends these limits as described below.

#33 removes the former `capturing for bindings is not supported in this subset`
rejection; see [the read-only iteration-capture contract](#implemented-read-only-iteration-captures-33).
Ordinary read-only-write checks now apply to those captures.

#32 removes the former `nested capture forwarding is not supported in this subset`
and `context-requiring callable dependencies are not supported in this subset`
rejections; ordinary scope, type and mutability checks remain.

Captured parameter cells allocate eagerly in formal order after all parameter
validation and ordered contracts; captured let cells allocate only after completing
their initializers. Structural promotion can allocate a cell even when a later
capturing declaration is skipped. A nonempty environment allocates only
when its declaration executes; context-free declarations allocate no environment
and add no hidden parameter. Helpers requiring captures or callable contexts receive
a hidden first `i32` environment argument. No declaration executes its helper body.

See [the private layout and lifetime contract](memory-abi.md#private-closure-storage-30)
for uniform eight-byte cells/slots, allocation traps, monotonic growth and host view
refresh obligations. None of this adds a public callable ABI or changes explicit
string/array marshaling or external range restrictions.

## Implemented captured numeric compounds (#31)

`+=`, `-=`, `*=`, `/=`, and `%=` support captured `number` bindings and elements
of captured `number[]` bindings, as well as creator-side promoted bindings before
or after the capturing declaration. The former semantic captured-compound guards
are removed; ordinary numeric type, mutability and checked-backend rules remain.
Whole-array compounds and compounds on nonnumeric bindings remain invalid.

Lowering uses the existing checked place path in `src/codegen/place.rs`. Resolve
the target once and read its old value before evaluating the RHS. A promoted
scalar retains its original cell address and old `f64` on the operand stack.
An indexed compound saves the original checked element address and old value in
the existing `i32`/`f64` scratch slots. RHS mutation, array-binding rebinding, calls
or memory growth do not retarget the write or replace the saved old value. Bounds
and descriptor failures trap before RHS effects. Ordinary indexed assignment also
preserves its checked pre-RHS element address, without a compound old-value read.
`%=` uses the existing exact floating remainder helper.

No new AST form, scratch policy, cell/environment layout or public ABI is added.
#32 extends parameter/let forwarding and #33 adds read-only iteration captures as
described below. Full WP11 release remains deferred. [#31 validation passed](#issue-31-validation-2026-10-14).
The #29/#30 records below retain their historical scope.

## Implemented nested captures and callable contexts (#32)

An ancestor parameter/ordinary-let capture keeps its original `(BindingOwner,
VariableId)`, concrete type and mutability through every intermediate helper,
even when only a deeper descendant reads or writes it. The owning helper uses
`Identifier` for its own binding; descendants use `Captured`. Forwarding reuses the
original shared cell, never snapshots its value or allocates a replacement cell.
Initialization and lexical shadowing retain their declaration-time rules.

Earlier visible context-requiring callable dependencies forward the earlier
callable's original live environment, including through intermediate helpers with
no direct value captures. Requirements stop at the creator that owns the relevant
binding or callable declaration. Declaration-before-use and lexical visibility
remain mandatory; this adds no self/forward hoisting or callable values.

Environments place canonical original owner/declaration-ordered captures first,
then context-requiring dependency slots in source-preorder callable-ID order.
Context requirements propagate through earlier dependencies. Context-free
dependencies have no slots and introduce no hidden argument or allocation of
their own. A helper with either captures or contextful dependencies receives a
hidden first `i32`; a truly context-free helper retains only explicit parameters.
All slots use the existing private eight-byte layout and allocation timing.

Checked lowering independently verifies complete, unique, canonical metadata,
including requirements used only by descendants. Missing/extra entries, wrong
owners/types/mutability, uninitialized or invisible bindings, invalid dependency
visibility/order and forged sparse variable IDs retain checked diagnostics.
Helpers remain borrowed, direct and unexported; no public typed shape changes.
#33 adds the separate iteration-capture subset below; combined nested/transitive
iteration-capture convergence and full WP11 release remain deferred.

## Implemented read-only iteration captures (#33)

Closures may read the numeric iteration binding of a `range` or `number[]` `for`.
The frontend records `CaptureMutability::ReadOnly`. Before lowering helpers, the
backend independently derives mutability from the original `For` binding's
`BindingKind::Iteration` provenance and checks capture metadata against it; public
metadata cannot grant write permission. Ordinary assignment and all five numeric
compound writes (`+=`, `-=`, `*=`, `/=`, `%=`) to captured iteration bindings are
rejected, including skipped and unreachable writes. Distinct shadowing lets retain
their own mutability.

The existing promotion path runs for each successfully yielded captured binding,
before the body: after the range endpoint test and conversion/local set, or after
the checked array element load/local set. It allocates a fresh uniform eight-byte
cell for every yielded iteration, rather than reusing a previous iteration's cell.
Promotion is eager and structural even if the capturing declaration is skipped.
Empty/exhausted loops and failed yields allocate no iteration cell. Nonempty
environments still allocate only when their declarations execute.

Cells remain monotonic: they are neither reused nor reclaimed. Allocation may grow
memory or trap without rollback. Endpoint-before-conversion range behavior, checked
array loads, continue/break/return targets, array alias visibility and all existing
loop/control semantics remain intact. There is no AST or ABI shape change; see
[the private storage contract](memory-abi.md#private-closure-storage-30).

Combined nested/transitive iteration-capture convergence and full WP11 release
remain deferred. [#33 validation passed](#issue-33-validation-2026-10-15) for the
read-only iteration-capture subset, not the full WP11 contract.

## Issue #33 validation (2026-10-15)

Validation passed for #33's read-only range/array iteration-capture subset, as
reported by the integration owner and runtime-suite agent. Node.js v24.12.0 and
wasm-pack 0.15.0 were available; no tools were installed. The browser build used
`--mode no-install` and cached wasm-bindgen tooling even though its output included
`Installing wasm-bindgen`. Current-source browser bindings were rebuilt before
browser validation and the full include-ignored run.

| Exact command (repository root) | Result |
| --- | --- |
| `cargo check --offline` | Passed during implementation slices |
| `cargo check --offline --all-targets` | Passed; passed again after the review-driven assertion change |
| `cargo test --offline --test closures_frontend --test closures_lowering --test closures_adapters` | Passed: 34 frontend, 47 lowering, 2 default adapter tests; 2 adapter tests ignored by default |
| `cargo test --offline --test closures_runtime -- --ignored` | Runtime-suite agent reported 23 passed, exercising all four optimization levels |
| `cargo test --offline --test closures_frontend --test closures_lowering --test closures_runtime --test closures_adapters -- --include-ignored` | Passed final focused run: 34 frontend, 47 lowering, 23 runtime and 4 adapter tests |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current-source browser package |
| `node tests/browser.cjs pkg` | Passed |
| `cargo test --offline` | Passed complete default suite |
| `cargo test --offline -- --include-ignored` | Passed complete suite including ignored runtime/browser cases |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `git --no-pager diff --check` | Passed |

Independent review found no behavioral defect or standards violation, only optional
code smells. One weak branch-depth assertion was strengthened to require the exact
nested-region sequence `[BrIf1, Br3, Br5, Br0, Br0]`, plus an explicit continue latch
followed by induction add/set/backedge. The subsequent `closures_lowering` rerun
passed all 47 tests, and the all-target check passed again. This was a test-strength
improvement, not a compiler behavior fix.

The final focused include-ignored run and a subsequent full repository
include-ignored rerun passed after the strengthened lowering assertions and
all-five-compound forged-write checks. Runtime/browser tests retain their
ignored-by-default policy.

This validates #33's read-only iteration-capture subset. Combined nested/transitive
iteration-capture convergence and full WP11 release remain deferred. Earlier issue
records below retain their historical scope.

## Issue #32 validation (2026-10-14)

Final integrated validation passed, as reported by the parent/integration owner.
Node.js v24.12.0, wasm-pack 0.15.0, the Wasm target, compatible tooling and cached
dependencies were available; the agent installed no dependencies or tooling.
Browser bindings were rebuilt from current source before the final browser,
focused closure-suite and full include-ignored runs.

| Exact command (repository root) | Final result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline --test closures_frontend --test closures_lowering --test closures_runtime --test closures_adapters -- --include-ignored` | Passed: 33 frontend, 44 lowering, 17 runtime and 4 adapter tests |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current-source browser package before final browser/focused/include-ignored coverage |
| `node tests/browser.cjs pkg` | Passed: 25 cases |
| `cargo test --offline` | Passed complete default suite |
| `cargo test --offline -- --include-ignored` | Passed complete suite including ignored runtime/browser cases |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `git --no-pager diff --check` | Passed |

Frontend and forged-input/decoded-lowering coverage checks original-binding
forwarding, complete canonical metadata and capture-before-dependency slot layout.
New runtime cases verify original shared cells through shadowing helper parameters
and locals, plus dependency-only intermediate helpers, at all four optimization
levels. The adapter fixture exercises nested captures and live callable dependencies
through native/browser byte parity and the CLI. Runtime/browser cases retain their
ignored-by-default policy; the explicit runs above establish execution coverage.

The initial adapter run failed because two obsolete rejection fixtures used syntax
now allowed by #32. They were replaced with still-deferred captured-`for` rejection
cases; the final reruns passed.

Two-axis review compared the working diff against
`28ecae41224ff31ca299c2d9e568f53d38b99b32`. Spec reported zero findings; Standards
reported zero breaches or bugs and one nonblocking judgment call about possible
primitive obsession in the capture-order tuple. The small internal tuple was
retained without adding an abstraction.

This validates #32's nested original-binding capture and live callable-context
forwarding subset, not parent #28 or full WP11 release. Captured `for` bindings and
full WP11 release remain deferred. Historical #29–#31 evidence below retains its
original scope.

## Issue #31 validation (2026-10-14)

The integrated captured-place subset passed the following offline checks. Node.js
v24.12.0, wasm-pack 0.15.0, the Wasm target and cached dependencies were available;
no dependencies or tooling were installed. Browser bindings were rebuilt from
current source before the final browser and include-ignored runs.

| Exact command (repository root) | Result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline --test closures_frontend --test closures_lowering --test closures_runtime --test closures_adapters -- --include-ignored` | Passed: 29 frontend, 40 lowering, 15 runtime and 4 adapter tests |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current-source browser package |
| `node tests/browser.cjs pkg` | Passed: 25 cases |
| `cargo test --offline` | Passed complete default suite |
| `cargo test --offline -- --include-ignored` | Passed complete suite including ignored runtime/browser cases |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `git --no-pager diff --check` | Passed |

Red-first frontend and forged-input tests demonstrated the former subset rejection
before implementation. Coverage now includes all five captured scalar/indexed
compounds, creator-side promotion before declarations, saved places and old values
through RHS mutation/rebinding, aliases, descriptor/bounds traps before RHS events,
independent IEEE-754 expectations, and abrupt RHS transfers. Decoded instruction
and scratch assertions supplement `wasmparser::Validator`; bounded execution checks
observable effects at all four optimization levels. CLI bytes/WAT match the core.
Runtime/browser cases retain their ignored-by-default policy.

The first integrated run exposed two test-fixture errors, not compiler defects:
a CLI expectation omitted a later scalar reset, and a transfer case used an
out-of-bounds array index. Both were corrected using independent source arithmetic
and intended event order before the successful final runs.

Both Standards and Spec reviews compared the uncommitted implementation with
starting HEAD `c7af8b16bd4a6cbc2172501c95626745e5833e27` and reported no actionable
findings. This validates #31's captured-place slice, not the deferred parent #28
or full WP11 release.

## Issue #30 validation (2026-10-14)

Final validation on the current integrated code passed after review fixes, as
reported by the integration owner. Node.js v24.12.0 and wasm-pack 0.15.0 were
available with the Wasm target, compatible bindings tooling and cached dependencies;
no tooling or dependencies were installed. Browser bindings were rebuilt from
current source before the final browser and include-ignored coverage.

| Exact command (repository root) | Final result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline --test closures_frontend --test closures_lowering --test closures_runtime --test closures_adapters -- --include-ignored` | Passed: 27 frontend, 34 lowering, 10 runtime and 3 adapter tests, including rebuilt-browser coverage |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current-source browser package |
| `node tests/browser.cjs pkg` | Passed: 25 cases |
| `cargo test --offline` | Passed complete default suite |
| `cargo test --offline -- --include-ignored` | Passed complete suite including ignored runtime/browser cases |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `git --no-pager diff --check` | Passed |

Runtime coverage includes independent allocation counts, parameter validation and
ordered contracts before parameter-cell promotion, and noncompleting initializers
allocating no corresponding let cell. Cases cover shared mutation across all
concrete types, lexical shadowing, array-binding rebinding, recursive and host-
reentrant creators, and post-trap retained handles remaining valid across memory
growth. These explicit runtime/browser runs, not default Cargo success alone,
establish the subset's execution coverage. The ignored-by-default policy remains
unchanged.

Review compared the uncommitted implementation against starting HEAD
`b797d480689f166366ab0360b576c56f89e4ea64`. The Spec review found no findings.
The Standards review found a linear capture-access scan and duplicate compound-
place bookkeeping across repeated inference/body checks. Owner-qualified lookup
and ordered deduplication fixed those hot paths, with regression tests preserving
first-error order and repeated analysis. Both fixes were re-reviewed with no
remaining findings; final body checks remain intact without AST cloning.

This validates only #30's immediate-creator parameter/ordinary-let capture subset.
At that checkpoint, nested forwarding, context-requiring callable dependencies,
captured compound places and captured `for` bindings remained deferred, as did
full WP11 release. #31 subsequently implements captured numeric compounds; this
record does not validate that extension.
No parent #28 release or broader WP11 acceptance claim follows from these results.
The historical #29 evidence below retains its original scope and counts.

## Historical #29 implementation ownership

Exclusive implementation write scopes for #29:

- Frontend integration: tokens/lexer/parser/grammar, `src/semantic.rs` and
  `src/semantic/`, frontend constructor migrations, `tests/closures_frontend.rs`.
- Backend integration: `src/codegen/`, backend constructor migrations,
  `tests/closures_lowering.rs`.
- Integration owner: public `src/ast.rs` and `src/typed_ast.rs`, runtime/adapter tests,
  documentation and final validation. Public shapes above are frozen before work
  is dispatched; changes require coordination with the integration owner.

## Historical #29 subset validation — 2026-10-14

The #29 subset is implemented and validated, not full shared-capture WP11.
Frontend/lowering suites each pass 22 tests. The seven Node runtime tests and
three adapter tests (including rebuilt-browser parity) pass when explicitly run.
Default runs keep the seven Node and one browser cases ignored.

Actual successful commands from the repository root:

```text
cargo check --offline
cargo test --offline --test closures_frontend --test closures_lowering
cargo test --offline --test closures_runtime --test closures_adapters -- --include-ignored
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline
node tests/browser.cjs pkg
cargo test --offline
cargo test --offline -- --include-ignored
cargo clippy --offline --all-targets -- -D warnings
cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings
git --no-pager diff --check
```

Node v24.12.0, wasm-pack 0.15.0 and the installed Wasm target/cached dependencies
were available. The browser package was rebuilt after the final compiler changes;
its existing browser smoke suite passed 25 cases, and the dedicated closure adapter
compared current-source native/browser bytes, WAT, bounded execution and rendered
diagnostics at all four optimization levels and three export selections.

Review used the starting commit `9b4d172636acf9d7ab386d32ea86f1f916c0f509` against
the uncommitted implementation (the requested review precedes commit). The spec
axis found no actionable deviations. The standards axis identified exponential
nested inference and quadratic retained callable-permission snapshots; both were
fixed and re-reviewed. Concrete closure-result memoization retains final body
checks, while fixed-size lexical predecessor links preserve declaration-time
permissions and forged missing-dependency diagnostics. Regression coverage includes
depth-16 declarations and 2,048 independent sibling helpers. An earlier 120-second
default-suite attempt timed out; the final bounded broader runs above succeeded.

These runs covered #29 before shared capture storage was implemented. They do not
validate #30's cells/environments or establish full WP11 parent acceptance gates.
See the separate [#30 validation record](#issue-30-validation-2026-10-14) for the
#30 shared-capture subset, before #31 captured compounds.
