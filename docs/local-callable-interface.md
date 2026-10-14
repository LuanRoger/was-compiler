# Local callable interface — issues #29–#30

## Frozen public interface and implementation ownership

Issue #29 implements capture-free local callable declarations and positional
lexical invocation. Issue #30 adds shared captures of immediate-creator parameters
and ordinary lets of every concrete type: `number`, `bool`, `string`, `number[]`,
and `range`. Captures refer to original shared bindings, never snapshots.
The parent [issue #28](https://github.com/LuanRoger/was-compiler/issues/28)
defines the full contract. Nested forwarding, context-requiring callable
dependencies, captured compound places, captured `for` bindings and full WP11
release remain deferred. [#30 subset validation passed](#issue-30-validation-2026-10-14);
the historical #29 evidence below predates this capture subset.

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
  bindings; #30 accepts only mutable immediate-creator parameters/ordinary lets.
  Captures are unique and canonical by original owner/declaration order, with
  parameters before locals. Checked lowering derives declaration order independently
  of numeric `VariableId` values, since public typed input may use sparse IDs.
- #30 adds `TypedExprKind::Captured { owner: BindingOwner, id: VariableId,
  name: String }`. Migrate exhaustive matches and distinguish captured reads,
  assignment targets and indexed array bases from owner-local `Identifier` nodes.
  Creator accesses remain `Identifier`, even when their storage is promoted.
  Matching local IDs in different owners do not identify the same binding.
- `dependencies: Vec<CallableId>` records unique earlier context-free callable
  identities in discovery order. Context-requiring dependencies remain deferred.

Backend planning borrows declaration-owned bodies, emits direct helpers after all
existing runtime helpers, and validates all owners, identities, lexical availability,
results, calls and transfers through existing checked lowering. Context-free helpers
have only explicit parameters and allocate no callable environment. Ordinary body
operations and handle signatures may still require memory.

## Implemented shared-capture subset (#30)

Outer and helper reads/writes observe the same binding; ordinary indexed array
mutation and array-binding reassignment retain existing alias/type rules. Capture
resolution preserves declaration-time initialization and shadowing. Helpers keep
independent parameter/local/return/loop contexts. A nested helper may capture its
immediate creator's own parameters/lets and be called by that creator; it may not
forward an ancestor binding through an intervening helper. Creator calls to its
own capturing declarations are supported. Earlier context-free callable
composition remains supported.

The staged semantic `InvalidClosure` reasons are stable:

| Deferred case | Reason |
| --- | --- |
| Capture owned by an ancestor other than the immediate creator | `nested capture forwarding is not supported in this subset` |
| A helper depends on a context-requiring callable owned elsewhere | `context-requiring callable dependencies are not supported in this subset` |
| Compound assignment to a captured variable or indexed captured array base | `compound assignment to captured bindings is not supported in this subset` |
| Capture of a `for` iteration binding | `capturing for bindings is not supported in this subset` |

Compound rejection also covers creator-side promoted bindings, including compounds
before the capturing declaration. Use ordinary assignment, such as `x = x + 1;`,
within this subset. Ordinary unrelated compounds remain supported.

Captured parameter cells allocate eagerly in formal order after all parameter
validation and ordered contracts; captured let cells allocate only after completing
their initializers. Structural promotion can allocate a cell even when a later
capturing declaration is skipped. A nonempty capture environment allocates only
when its declaration executes; context-free declarations allocate no environment
and add no hidden parameter. Capturing helpers receive a hidden first `i32`
environment argument. No declaration executes its helper body.

See [the private layout and lifetime contract](memory-abi.md#private-closure-storage-30)
for uniform eight-byte cells/slots, allocation traps, monotonic growth and host view
refresh obligations. None of this adds a public callable ABI or changes explicit
string/array marshaling or external range restrictions.

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
Nested forwarding, context-requiring callable dependencies, captured compound
places and captured `for` bindings remain deferred, as does full WP11 release.
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
current shared-capture subset.
