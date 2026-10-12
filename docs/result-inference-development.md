# Initial function-result inference: issue #11 integration

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
