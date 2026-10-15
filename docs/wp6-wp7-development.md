# Issues #12 and #11: development handoff

**Workspace relocation:** This historical record retains its original paths and
commands. Current implementation/tests belong to `crates/compiler/`, `crates/cli/`,
and `crates/browser/`; shared test assets remain at root. See
[current validation](validation.md) for live commands.

This is the historical independent-development record. WP6 is now integrated on
`main` and #13's release validation passed; see
[the release evidence](was_language_extensions_plan.md#issue-13-wp6-release-validation).
Issues #11/#14 subsequently integrated WP7 inference on main, #15 accepted its
named/default interactions, and [#16 release validation passed](was_language_extensions_plan.md#issue-16-wp7-release-validation).
Every recursive component member still needs an annotation in the initial release.
References below to uncommitted branches and pending #13 describe
the earlier development checkpoint, not current release status.

## Starting point and gates

Base: `1d7dbf3` on `main`, including #10 commit `156512e`. The working tree
was clean. Issues #9–#12 and their comments, the extension plan §4, and ADRs
0001/0002 were read. #10 records all acceptance criteria and full offline,
browser, and ignored-runtime gates as passed. Its focused tests were rerun:
9 named-argument, 11 checked-lowering, and 5 runtime tests passed.

#12 develops on `implement/wp6-defaults`. #11 develops only in the isolated
`target/wp7-inference` worktree on `develop/wp7-initial-inference`. Neither is
staged or committed. The WP7 worktree starts at the same #10 baseline; it must
not be merged into WP6 before #13's release gate passes. #13 remains out of scope.
No issue is closed and parent #9 is not edited. Compiler execution stays sequential.

The test seams already approved in #9 are source analysis, public typed-program
compilation, and source-to-Wasm/decoded ABI/bounded runtime behavior.

## Frozen interfaces

### WP6 (#12)

- Raw `Param` gains `default: Option<DefaultExpression>`;
  `DefaultExpression { value: Expr, span: Span }` retains the expression's
  declaration origin. Raw result annotations remain mandatory on this branch.
- Existing `TypedNormalizedCall`, `TypedCallBinding`, and `CallBindingOrigin`
  are unchanged. Defaults follow explicit bindings; `Default { formal }` indexes
  their actual formal and increases strictly. The mapping remains a bijection.
- Private `FunctionSignature` gains
  `defaults: Vec<Option<defaults::CheckedDefault>>` in formal order.
  `CheckedDefault { value: TypedExpr, span: Span }` owns a declaration-checked
  template whose `VariableId`s identify earlier formals, assigned in formal order.
- `SemanticAnalyzer::validate_defaults(&Function)` populates these templates
  using earlier-formal scope and the existing expression checker. It runs for
  all declarations before body checks when defaults occur. Legacy declarations
  retain their previous diagnostic ordering.
- `SemanticAnalyzer::analyze_bound_call(name, args, span, return_type)` implements
  named/default policy. The integrator routes named or omitted-default calls here;
  complete ordinary positional calls retain the direct path.
- Explicit IDs are reserved from the caller allocator before their expressions.
  Defaults are instantiated by remapping resolved formal IDs through the mapping,
  never by name. No synthetic name enters caller lookup. Defaults can read only
  owned earlier-formal bindings; explicit initializers cannot read owned bindings.
- New semantic errors: `NonTrailingDefault { name, span }`,
  `InvalidDefaultExpression { reason: &'static str, span }`,
  `InvalidDefaultNumberLiteral { literal, span }`, and
  `DefaultNumberLiteralOutOfRange { literal, span }`. Existing type/name errors
  use default declaration spans. Existing named-call variants remain unchanged.
- Private `crate::number_literal::parse(&str) -> Result<f64, LiteralError>` is the
  sole numeric literal rule implementation; `LiteralError::{Invalid, OutOfRange}`.
  Semantic defaults and backend literals map it to their existing stage errors.
- Backend validates whitelist/default reads and ordering using the sequence's
  mapping and origins; no source-authenticity claim is made for public metadata.

### WP7 (#11), isolated worktree only

- Raw `Function.return_type` becomes `Option<Type>`; typed signatures remain `Type`.
  Parameter types stay mandatory. Defaults are not independently implemented here;
  #12 will be integrated only after its required gate.
- `semantic/inference.rs` owns `SemanticAnalyzer::infer_results(&Program)`.
  It resolves lexical calls (including unreachable/skipped children), rejects any
  recursive component containing an unannotated member, and rejects inferred-to-
  inferred dependencies with `UnresolvedResultDependency { name, callee, span }`.
  No chain scheduling or fixed-point solver is added.
- Integrator supplies `SemanticAnalyzer::infer_result(&Function, FunctionId)`.
  It invokes the existing checker in private `ReturnContext::Infer`, composes
  reachable return constraints through `Flow`, and returns a concrete `Type`.
  Final body checks run in source order using `ReturnContext::Known(Type)`.
- `Flow::return_value(Type)` and `Flow::return_type()` retain reachable return
  categories through existing `then`/`join`/loop operations. Value-return syntax
  is tracked independently by the raw traversal, not as unconditional constraints.
  Unreachable returns cannot choose a result but are checked in the final pass.
- New errors: `RecursiveResultAnnotationRequired { name, span }`,
  `UnresolvedResultDependency { name, callee, span }`,
  `CannotInferFunctionResult { name, span }`, and
  `IncompatibleInferredReturns { expected, found, span }`.
- Declarations retain source identities and output order. Fully annotated programs
  bypass inference staging so legacy diagnostic ordering stays unchanged.

## Exclusive writers

| Writer | Exact files |
| --- | --- |
| Integrator, WP6 | `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`, `src/lib.rs`, `src/number_literal.rs`, `src/codegen/call.rs`, `src/codegen/expression.rs`, `grammar.bnf`, affected existing AST consumer tests, `tests/default_lowering.rs`, `tests/default_runtime.rs`, documentation |
| WP6 policy agent | `src/semantic/defaults.rs`, `src/semantic/call_binding.rs`, `tests/named_defaults.rs` |
| Integrator, isolated WP7 | worktree `src/ast.rs`, `src/parser.rs`, `src/semantic.rs`, `src/flow.rs`, `grammar.bnf`, affected existing AST consumer tests, documentation |
| WP7 policy agent | worktree `src/semantic/inference.rs`, `tests/result_inference.rs` |

Agents request changes to all other files. Sessions remain available for follow-up.
No other writer edits a delegated file, including during integration fixes.

## Validation

Final development validation ran on 2026-10-12 from each worktree independently.
Both remain uncommitted. This is not the WP6 (#13) or WP7 integration/release gate.
No required cached dependency, Node/browser tooling, or Wasm target was missing;
no dependency/tool installation or network fetch was requested. Browser packages
were rebuilt from each tree's current source before browser/full ignored tests.

### WP6: `implement/wp6-defaults`

| Command | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline --test named_defaults --test default_lowering` | Passed: 33 declaration/binding tests and 3 decoded/forged-tree tests |
| `cargo test --offline --test default_runtime -- --include-ignored` | Passed: 4 tests, including 3 normally ignored Node/browser cases at all levels |
| `cargo test --offline --test named_arguments --test named_call_lowering --test named_call_runtime -- --include-ignored` | Passed after default-origin test migration: 9 frontend, 11 lowering, 5 runtime tests |
| `cargo test --offline` | Passed; Node/browser cases retained their ignored policy |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; current-source package rebuilt |
| `node tests/browser.cjs pkg` | Passed: existing 22 groups |
| `cargo test --offline -- --include-ignored` | Passed; all ignored runtime/browser tests executed, including new default cases |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| Scoped Rust formatting and `git --no-pager diff --check` | Passed |

Coverage includes full-arity signatures and decoded local/call order, dependent
per-invocation defaults, written explicit order, scalar rebinding, shared array
mutation, string/array equality, range-handle copies, same-call-site bounded host
reentrancy, recursion, caller returns/breaks/continues, default traps after explicit
effects, declaration/call origins, exact browser/native bytes and rendered errors,
legacy fixtures/import snapshots/export selection, and optimization parity.

### WP7: isolated `develop/wp7-initial-inference`

| Command | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline --test result_inference -- --include-ignored` | Passed: 34 tests, including 3 normally ignored bounded Node/browser cases |
| `cargo test --offline` | Passed; normally ignored runtime tests remained ignored |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; isolated current-source package rebuilt |
| `node tests/browser.cjs pkg` | Passed: existing 22 groups |
| `cargo test --offline -- --include-ignored` | Passed; all runtime/browser cases executed |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| Scoped Rust formatting and `git --no-pager diff --check` | Passed |

Coverage includes every concrete result type/private ranges, void/divergence and
Never-only syntax, nested flow and conservative loops, unreachable final checks,
lexical shadowing and initializer scope, conservative recursive components,
source-ordered declaration/dependency diagnostics, deferred unresolved-callee
rejection, forged result/body mismatches, decoded ABI, exact explicit/inferred
bytes at all levels, and browser/native diagnostics/recovery. See
[the isolated contract](result-inference-development.md).

### Review and integration status

Two-axis review used pinned base `1d7dbf36db7592b0a57665faf99ab9e9bbaafa37`,
including untracked files. It found no confirmed implementation defect. Coverage
findings were addressed with range execution, same-site reentrancy, and dedicated
rebuilt-browser default parity; the private binding path's duplicate target lookup
was removed. Staged declaration-order regressions were observed red and fixed
without changing either legacy path. Early runtime-test expectations were corrected
to retain existing parameter-entry descriptor validation and independently worked
arithmetic values. No golden fixture was regenerated.

Follow-up Standards and Spec reviews each reported zero outstanding findings.
Local Markdown checks passed for 112 paths/anchors in each tree. Final diff and
status review found only the assigned feature changes; nothing was staged.

Both full validation sequences were rerun after production/review fixes. WP7 has
not been merged into WP6. Combined feature integration tests are deliberately
pending #13; running each complete suite does not establish that separate gate.
Issues #9/#11/#12/#13 remain unchanged and open (except already closed #10).

## Issue #11 integration ownership after #13

Integration base: main e92b06b; existing WP7 commit: 1019b8b. Both trees
were clean. The integrator alone resolves shared AST/parser/semantic/Flow,
default and call-binding interfaces, module wiring, existing test migrations,
and documentation. A dedicated test agent owns only tests/result_inference_defaults.rs;
an adapter test agent owns only tests/result_inference_adapters.rs.
They request all other file changes from the integrator. Existing public seams
remain source analysis, checked typed-program compilation, decoded ABI and
bounded CLI/browser/Node behavior. No #14 dependency scheduling or #15 release
acceptance is implemented. ReturnContext::Known(Type)/Infer stays private;
default whitelist checking precedes typing with Known(Void) because admitted
defaults cannot return. Named/default explicit arguments use the caller return
context; default declarations precede graph/probe/final-body checks.

The two dedicated test agents stopped at their usage limits. The integrator
assumes exclusive ownership of both test files to finish and validate their
partial work; no concurrent writers remain.

Combined validation and red/green diagnostic regression evidence are recorded in
[result inference development](result-inference-development.md#combined-validation-2026-10-12).
Final read-only Standards review found no outstanding hard violations. Spec
review found a shortened positional/default dependency-origin mismatch; it was
fixed and verified with source, CLI and rebuilt-browser regressions. Follow-up
Spec review reported zero outstanding findings. The discretionary private context
parameter rename was left unchanged to avoid incidental churn.
