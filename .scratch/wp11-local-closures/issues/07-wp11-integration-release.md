## Parent

Part of #28 — Implement WP11 local shared-binding closures. This is the convergence and release-evidence gate for the complete parent contract; do not edit or close the parent as part of this ticket.

## What to build

Complete consistent WP11 support through the core library, native CLI, and freshly rebuilt browser adapter, reconcile the full feature documentation, and record release evidence for every applicable acceptance gate.

## Acceptance criteria

- [ ] Demonstrate the complete integrated parent contract, including nested read-only range/array iteration captures, transitive dependencies, captured compound/indexed rebinding places, and recursion/reentry/trap lifetime scenarios. Remove staged subset limitations without admitting out-of-scope forms.
- [ ] Run dedicated frontend, lowering, runtime, and adapter closure coverage through existing compiler/typed-input/adapter seams. Verify diagnostic variants/reasons/exact independent byte spans and native/browser rendering with Unicode, comments, tabs, CRLF, and EOF cases.
- [ ] CLI bytes/WAT match library output; retain exit codes, unchanged destinations on failure, and temporary cleanup. Freshly rebuilt browser bytes/WAT/execution/diagnostics match current native source.
- [ ] Preserve host snapshots/import order, external signature eligibility, explicit export selection, user IDs, required memory exports/collision behavior, all-body checking, and optimization-level parity. Cover ordinary named/default calls, contracts, pipelines, ranges, arrays, matches, and existing compatibility fixtures.
- [ ] Independently decode complete module helper/signature/environment order, source-order determinism independent of inference scheduling, no tables/indirect calls/helper exports, correct creator/helper local separation, and context-free memory omission. Validate generated Wasm and execute bounded observable effects.
- [ ] Reconcile grammar, normative extension specification/plan, public raw/typed AST migration, supported-language guide, limitations, private memory/lifetime contract, runtime/browser guidance, and agent guide. Retain the accepted ownership/storage ADR and glossary vocabulary; keep WP12/WP13 explicitly out of scope.
- [ ] Run focused checks first, then offline default Cargo tests and all-targets clippy with warnings denied. Rebuild current browser bindings offline without installation before browser tests and the full include-ignored runtime suite; run browser-target clippy.
- [ ] Record exact commands/results, tool versions/prerequisites, and missing or unrelated failures. Missing Node.js 24+, Wasm target, wasm-pack/bindgen tooling, cached dependencies, or required coverage prevents the affected release claim; no silent fetching/installing or treating stale bindings/default Cargo success as complete evidence.
- [ ] Preserve ignored-by-default Node/browser test policy and use bounded subprocess support without fuel/process-tree/rollback claims. Format only edited Rust files, review status/diff, and finish with whitespace checks; do not regenerate compatibility fixtures to hide failures.
- [ ] Mark WP11 implemented/release-validated only when the corresponding evidence actually passes. Do not introduce immediate invocation, callable pipeline stages/placeholders, first-class callables, reclamation, or incidental optimization.
