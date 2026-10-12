# Initial function-result inference: isolated issue #11 development

This feature is implemented and validated only in the uncommitted
`target/wp7-inference` worktree (`develop/wp7-initial-inference`). The main
WP6 working branch still requires result annotations. WP7 integration/release
remains blocked on the separate WP6 release gate #13; no gate is waived here.
See [the coordination handoff](wp6-wp7-development.md).

## Incremental language contract

Inside the isolated worktree, `fn square(x: number) { return x * x; }` infers
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

The isolated raw API changes `Function.return_type` to `Option<Type>`:
`Some(type)` preserves an explicit annotation; `None` requests inference.
Typed signatures remain concrete `Type`, with unchanged full arity and ABI.
There are no public unresolved types or flow annotations. Source identities,
typed function order, and emitted layout remain source-ordered.

Private inference uses the existing expression checker in `ReturnContext::Infer`
and composes reachable return constraints through `Flow`. Final body checks
use resolved concrete signatures and run in source order. Graph discovery is a
lexical traversal, not a second expression checker. Fully annotated programs
bypass inference staging and preserve legacy diagnostics and bytes. Programs
using inference stage declarations, graph errors, result probes, then final
body checks; duplicate parameters are declaration errors in this path.

Range import/selected-export restrictions remain unchanged. Inferred and explicit
programs produce identical bytes at all current optimization levels.

## Evidence

`tests/result_inference.rs` in the isolated worktree covers frontend/scope/flow,
recursion and deferred-dependency rejection, source IDs, forged result/body
mismatches, independently decoded signatures, native/browser byte and diagnostic
parity, and bounded execution. Node/browser tests retain the default ignored
policy. Exact validation commands/results are recorded in the coordination
handoff; development validation is not WP7 integration or release acceptance.
