# Function-result inference development

WP7 acyclic chains/diamonds and named/default integration are implemented;
[#16 release validation passed](was_language_extensions_plan.md#issue-16-wp7-release-validation).
Every recursive component member still requires an annotation in the initial release.
See [the current inference contract](#issue-14-acyclic-chains-and-diamonds) and
[release acceptance coverage](#issue-16-nativebrowser-release-acceptance).
The earlier #11/#14/#15 sections retain historical evidence for their own checkpoints.

## Historical issue #11 integration

This is the deliberately incremental #11 implementation on
`develop/wp7-initial-inference` in `target/wp7-inference`. Released WP6 from main
`e92b06b` is integrated after #13's completed gate. The full WP7 release gate
remains separate; this work does not implement #14 chain scheduling or #15.
See [the coordination handoff](wp6-wp7-development.md).

## Incremental language contract

With this incremental implementation, `fn square(x: number) { return x * x; }` infers
`number`. Parameters still require explicit types. Reachable explicit returns
can infer number, bool, string, numeric array, or internal range. Bodies without
value-return syntax can infer void, including divergence-only statement bodies.
Mixed reachable return categories and incomplete nonvoid paths are rejected.
Named functions still require explicit return statements; ordinary tails are
not implicit returns.

Never is not a function signature. Value-return syntax with no reachable
concrete value return requires an annotation, even when that syntax is
unreachable. Unreachable returns do not select the inferred result but are
checked against its final concrete type. All skipped/unreachable children
remain checked.

Calls to explicitly annotated user functions, registered hosts, and builtins
supply concrete result information. This initial slice rejects calls from an
inferred function to another unannotated function with
`UnresolvedResultDependency`, regardless of declaration order. Forward chains
and dependency scheduling belong to #14, which is not implemented here.
Conservative lexical recursive components include unreachable/skipped calls;
every recursive member must be annotated, including anchored recursion through
an explicitly annotated member. No fixed-point solver is introduced.

## Representation and diagnostics

The raw API changes `Function.return_type` to `Option<Type>`:
`Some(type)` preserves an explicit annotation; `None` requests inference.
Typed signatures remain concrete `Type`, with unchanged full arity and ABI.
There are no public unresolved types or flow annotations. Source identities,
typed function order, and emitted layout remain source-ordered.

Private inference uses the existing expression checker in `ReturnContext::Infer`
and composes reachable return constraints through `Flow`. Final body checks
use resolved concrete signatures and run in source order. Graph discovery is a
lexical traversal, not a second expression checker. Fully annotated legacy
programs without defaults bypass staging and preserve diagnostics and bytes.
Programs with omitted results or defaults stage source-order declaration/default
validation, graph errors, result probes, then final body checks. Duplicate
parameters are declaration errors in this path. Default whitelist checks precede
typing with `Known(Void)` because admitted defaults cannot transfer control.
Normalized explicit arguments retain the caller's return context and written
argument origins, including shortened positional/default calls. Default templates
survive result resolution; all signatures remain concrete and full-arity.

Range import/selected-export restrictions remain unchanged. Inferred and explicit
programs produce identical bytes at all current optimization levels.

## Evidence

`tests/result_inference.rs` covers frontend/scope/flow, recursion, typed signatures,
private ranges, forged trees, decoded signatures and bounded Node/browser behavior.
`tests/result_inference_defaults.rs` adds declaration-order/diagnostic origins,
normalized argument return/break/continue flow, dependent defaults, scalar capture,
shared aliases, traps, IEEE-754 values, forged defaults, decoded locals and exact
inferred/explicit byte parity. `tests/result_inference_adapters.rs` checks CLI,
host snapshots, exports, rebuilt-browser parity and diagnostic recovery. All four
optimization levels are covered. Runtime tests remain normally ignored; rebuild
`pkg` before browser or full ignored tests. No unbounded guest is executed.

## Combined validation (2026-10-12)

Run from `target/wp7-inference`, after integrating main `e92b06b`. All dependencies
and prerequisites were available locally; no installation or fetching was needed.

| Exact command | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed |
| `cargo test --offline --test result_inference --test named_defaults --test default_lowering --test named_arguments --test named_call_lowering --test result_inference_defaults --test result_inference_adapters` | Passed after migrating obsolete syntax rejection and correcting new test expectations |
| `cargo test --offline --test result_inference_defaults --test result_inference_adapters --test result_inference -- --include-ignored` | Passed: 34 original, 15 interaction and 5 adapter tests |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current combined source |
| `node tests/browser.cjs pkg` | Passed: 22 groups |
| `cargo test --offline -- --include-ignored` | Passed; complete bounded runtime/browser coverage executed |
| `cargo test --offline` | Passed; normal ignored policy retained |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |

Original partial adapter tests needed the existing `RangeExport.function` field,
function-level incompatible-return origins, and the independently calculated sum
`349 + 236 + 160 + 2 = 747`. These were test corrections, not behavior changes.
No legacy golden fixtures were regenerated. #14 and full WP7 release acceptance
remain out of scope; main is not modified by this worktree integration.

Review found a default-normalized positional dependency origin mismatch. The new
`default_normalized_positional_dependencies_keep_written_argument_origins` test
failed with a statement span, then passed after lexical discovery mirrored the
checker's normalization decision. Full-arity positional origins stay unchanged.

## Issue #14: acyclic chains and diamonds

Issue #14 completes acyclic inferred-to-inferred calls, including forward chains
and diamonds. The production change is confined to `src/semantic/inference.rs`;
no new IR, public unresolved type, backend representation, or ABI is introduced.

Recursion and scheduling use separate policies. Conservative recursion detection
uses the full lexically resolved user-call graph, including annotated functions
and skipped/unreachable calls. Every recursive member must have an explicit
result annotation; an annotated scheduling barrier cannot hide an omitted result
inside a recursive component. No fixed-point inference is attempted.

Scheduling waits for every omitted-result callee of an inferred body, including
calls whose values are discarded or whose syntax is skipped/unreachable. Annotated
signatures are already concrete and act as scheduling barriers. Among currently
ready functions, the earliest source declaration is probed next; newly ready
functions participate in that same source-order selection. Repeated call edges
are released together, preserving deterministic chain/diamond resolution.

The existing expression checker and private `Flow` still infer reachable explicit
returns. Void, Never-only value-return syntax, exact joins, definite return, and
final checking of all skipped/unreachable children retain the #11 rules. Typed
programs own concrete, full-arity signatures and host snapshots. Function IDs,
typed-function order, emitted function/layout order, and default templates remain
source-ordered or otherwise unchanged by probe scheduling.

For staged programs, first-error order is:

1. Declaration/default validation in source order.
2. Recursive omitted-result annotation errors in source order.
3. Result inference in stable dependency order, with source-order ready ties.
4. Final body checks in source order against concrete signatures.

Fully annotated legacy programs retain their existing path and diagnostic order.

### Test coverage and acceptance boundary

`tests/result_inference_chains.rs` adds forward chains, diamonds in both declaration
orders, concrete/Never joins, void chains, anchored recursion rejection, and
unreachable-return checking. It checks owned source-order signatures/IDs, host
snapshots, decoded imports/functions/exports/calls, inferred-versus-explicit bytes
at all optimization levels, descriptor/internal-range signatures, and forged
result/body mismatches. Bounded Node cases exercise values and effect order;
those runtime tests retain the normal ignored policy.

Migrations in `tests/result_inference.rs`, `tests/result_inference_defaults.rs`,
and `tests/result_inference_adapters.rs` replace obsolete dependency-rejection
expectations with the resolved-chain contract and retained diagnostics.
At the #14 checkpoint, combined-feature acceptance (#15) and the full WP7
integration/release gate remained pending. The historical #11 validation and
coordination handoff remain evidence only for their recorded checkpoint.

### Issue #14 validation (2026-10-13)

The initial forward-chain test failed with `UnresolvedResultDependency` before
scheduler implementation, then passed after dependency resolution was added.
The following commands passed from the repository root:

- `cargo check --offline --all-targets`
- `cargo test --offline --test result_inference_chains -- --include-ignored`
  (10 tests, including two bounded Node execution cases)
- `cargo clippy --offline --all-targets -- -D warnings`
- `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline`
- `node tests/browser.cjs pkg`
- `cargo test --offline --test result_inference --test result_inference_defaults --test result_inference_adapters -- --include-ignored`
  (56 tests, including rebuilt-browser and CLI cases)
- `cargo test --offline -- --include-ignored`
- `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings`
- `git --no-pager diff --check`

Two-axis review found no documented-standard breaches or confirmed spec defects.
Two low-priority test-maintainability observations were addressed with explicit
case expectations and clearer test names. No golden fixtures were regenerated.
These results validate this change, not the separate #15/full WP7 acceptance gate.

## Issue #15: combined named/default and inference acceptance

Issue #15 establishes the combined contract after #13's completed WP6 release
and #14's acyclic scheduling. No production change was needed: the integrated
implementation passes the new acceptance cases. No AST, public seam, IR, ABI,
default whitelist, inference policy, or optimization behavior changes here.
The final WP7 release ticket still owns full adapter/release acceptance; these
results do not mark that ticket or the parent #9 complete.

Named/default calls can depend on forward inferred chains and diamonds, including
ordinary full-positional calls to the same resolved callees. Default templates
retain their declaration origins and earlier-formal identities through scheduling;
normalization retains written explicit order, then omitted-default formal order.
All results are concrete and machine signatures remain canonical and full-arity.
Source-order identities, layout, import snapshots and export policy are unchanged.

### Acceptance coverage

| Boundary | Evidence |
| --- | --- |
| Combined source, owned typed output and independently decoded Wasm | `tests/result_inference_combined.rs` exercises forward diamonds, named/default/direct calls, source-order IDs/imports/exports, direct instruction sequencing, full arity, required memory, determinism and inferred/explicit byte parity at all levels. |
| Dependent defaults and bounded execution | The same suite executes number/bool/string/array/internal-range chains, captures earlier scalars, observes shared alias mutation, evaluates defaults afresh per invocation, and checks independent effect markers and trap order. |
| Caller completion | Nested value blocks, ifs and value loops in explicit arguments preserve caller returns and nearest-loop breaks/continues. Transfers bypass later explicit effects and trapping defaults; skipped/unreachable children remain checked. |
| Staged diagnostics | `tests/result_inference_combined_checks.rs` progressively removes errors from multiply-invalid programs to pin declarations/defaults before recursion, dependency-order inference and source-order final checks, with written call/default origins and ready-function ties. Unused and explicitly supplied defaults remain checked; ordinary body literal errors stay at lowering. |
| Forged public typed input | Nested normalized target/result, formal mapping, binding ownership/default scope and escaped-binding mutations are rejected, as are body/signature mismatches in skipped, unreachable and unexported code. |
| Existing native/browser seams | `tests/result_inference_adapters.rs` now uses an omitted-result forward named/default diamond in its CLI/library/explicit-byte, host snapshot, raw arity and rebuilt-browser checks. Browser-compiled output is also instantiated to check values/effect order. |

The new suites add 19 tests, including three normally ignored bounded Node cases.
The five existing adapter tests now cover the combined diamond; their two runtime
cases retain the ignored policy. Existing default, inference-chain, named-call,
legacy ABI/emitter/diagnostic fixtures and runtime suites remain unchanged.
Independent literal expectations supplement `wasmparser::Validator`; passing
module validation alone is not the acceptance claim. No unbounded guest runs.

### Issue #15 validation (2026-10-13)

All commands below ran from the repository root using offline Cargo builds and
installed tooling. `pkg` was rebuilt before browser and full ignored execution.
New cases passed against the existing implementation, so no production regression
red/green cycle or golden-fixture regeneration was necessary.

| Exact command | Actual result |
| --- | --- |
| `cargo check --offline --all-targets` | Passed after adding the suites and extending adapter sources. |
| `cargo test --offline --test result_inference_combined --test result_inference_combined_checks --test result_inference_adapters` | Passed: 19 active tests, five normally ignored runtime cases. |
| `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; rebuilt current bindings. |
| `cargo test --offline --test result_inference_combined --test result_inference_combined_checks --test result_inference_adapters -- --include-ignored` | Passed: all 24 tests, including Node execution and rebuilt-browser parity/execution. |
| `node tests/browser.cjs pkg` | Passed: 22 groups. |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed. |
| `cargo check --offline --target wasm32-unknown-unknown --lib` | Passed. |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed. |
| `cargo test --offline -- --include-ignored` | Passed: complete suite, including normally ignored runtime/browser cases and legacy fixtures. |
| `cargo test --offline` | Passed: complete default suite; normal ignored policy retained. |
| Scoped `rustfmt --edition 2024 --config skip_children=true` on the three edited Rust test files | Passed. |
| Local Markdown link/path/anchor check | Passed: 109 references across the six edited Markdown files. |
| `git --no-pager diff --check` | Passed. |

Independent Standards review identified string-valued test modes and a related
repeated-dispatch maintainability issue. Replacing these with exhaustive local
enums and co-locating context construction/navigation resolved both; the focused
11-test suite and Clippy passed afterward. Follow-up Standards review reported
zero outstanding findings. Spec review reported zero missing, incorrect or
out-of-scope requirements. Production and golden fixtures remain unchanged.

## Issue #16: native/browser release acceptance

The WP7 release accepts acyclic inference through every existing compiler adapter,
including named/default chains and diamonds. The conservative recursion limitation
remains: every recursive component member needs an explicit result, even across
annotated barriers or calls in skipped/unreachable syntax. WP8–WP12 remain proposed;
WP13 optimization requires separate authorization.

`tests/result_inference_release.rs` adds seven public core/CLI acceptance tests
(one bounded Node case stays ignored by default). It checks cross-adapter rejection
variants, original Unicode/CRLF/tab origins, unchanged artifacts after failure,
default/recursion/dependency/final-body diagnostic staging, ordinary body-literal
lowering errors even in hidden source, source-order full-arity signatures/imports,
owned snapshots, selected-export range restrictions, and CLI-produced Wasm execution
with normalized host bool boundaries and string/array descriptors at all levels.

Two additional groups in `tests/browser.cjs` use the rebuilt browser public API to
check inferred/explicit bytes and WAT, bounded diamonds and written effect order,
full raw arity, bool normalization, descriptor identity/aliasing/memory growth/traps,
internal ranges, atomic host rejection/import order and export remedies. Their
rejection matrix independently pins rendered inference/binding/default diagnostics
through both compile methods, including conservative recursion, lexical shadowing,
incompatible returns, incomplete paths, Never-only syntax, implicit tails and invalid
unused/explicitly supplied defaults. `tests/browser.rs` requires the new 24-group
completion summary.

The existing inference/default/combined suites retain forged result/normalization,
decoded layout/instruction, effect/trap/transfer and legacy fixture coverage. No
production code, optimizer, dependency, public AST shape or golden fixture changed.
New acceptance tests passed against existing implementation; initial void/statement-if
and arithmetic test expectations were corrected to the established language contract,
not by changing compiler behavior. The full ignored run caught the stale browser
completion count before its update; the focused wrapper and full suite then passed.

`examples/inferred-defaults.was` is a runnable forward named/default diamond;
independent arithmetic is `236 + 348 = 584`. Current support, raw-AST migration,
initial recursion restrictions and raw runtime/browser ABI guidance are synchronized.
See [exact commands and results](was_language_extensions_plan.md#issue-16-wp7-release-validation)
for current-source release evidence, rather than the historical worktree runs above.
