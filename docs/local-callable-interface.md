# Local callable interface — issue #29

## Frozen public interface and implementation ownership

Issue #29 implements only capture-free local callable declarations and positional
lexical invocation. Shared capture storage and full WP11 release remain deferred.
The parent [issue #28](https://github.com/LuanRoger/was-compiler/issues/28)
defines the full contract. Capture-requiring declarations must be rejected, never snapshotted.

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
  bindings. In #29 captures must be empty; `dependencies: Vec<CallableId>` records
  unique earlier context-free callable identities in discovery order.

Backend planning borrows declaration-owned bodies, emits direct helpers after all
existing runtime helpers, and validates all owners, identities, lexical availability,
results, calls and transfers through existing checked lowering. Context-free helpers
have only explicit parameters and allocate no callable environment. Ordinary body
operations and handle signatures may still require memory.

Exclusive implementation write scopes:

- Frontend integration: tokens/lexer/parser/grammar, `src/semantic.rs` and
  `src/semantic/`, frontend constructor migrations, `tests/closures_frontend.rs`.
- Backend integration: `src/codegen/`, backend constructor migrations,
  `tests/closures_lowering.rs`.
- Integration owner: public `src/ast.rs` and `src/typed_ast.rs`, runtime/adapter tests,
  documentation and final validation. Public shapes above are frozen before work
  is dispatched; changes require coordination with the integration owner.

## Subset validation — 2026-10-14

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

Shared value captures, environments/cells, capture lifetimes and full WP11 release
remain deferred. This evidence does not establish those parent acceptance gates.
