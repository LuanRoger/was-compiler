## Parent

Part of #28 — Implement WP11 local shared-binding closures. The parent specification is authoritative; this ticket delivers the first explicitly documented subset, not full WP11 release readiness.

## What to build

Make capture-free local callable declarations and direct lexical positional invocation work from source through checked Wasm compilation and bounded execution. Establish the dedicated callable/owner interface used by subsequent tickets without introducing first-class callable values or a public callable ABI.

## Acceptance criteria

- [ ] Accept explicitly typed positional and zero-argument parameters, expression bodies, tail-valued block bodies, Void helpers, and transparent parentheses around lambda initializers. Support existing concrete parameter/result types and existing body expressions/control flow.
- [ ] Implement exact reachable return/tail joins, no-tail Void completion rules, Never-only inference rejection, concrete call signatures, independent return/loop contexts, and checking of skipped/unreachable children, exactly as specified in #28.
- [ ] Preserve initializer-before-binding visibility, lexical shadowing, earlier context-free callable invocation including nested declarations, and ordinary undefined-name errors for unresolved self/forward references. Do not add special recursive closure visibility.
- [ ] Include named calls in lambda bodies in structural recursion discovery; require annotations on recursive named-function members, allow annotated creator/closure cycles, and keep lambda returns out of creator inference. Use bounded cases for execution.
- [ ] Establish raw lambda/call spans, owned typed closure declarations, dedicated typed lexical calls, program-unique callable IDs, owner-qualified bindings, and capture/dependency metadata interfaces consistent with the complete parent contract. Document the public AST migration before dependent work.
- [ ] Discover helpers in source depth-first preorder independently of inference scheduling, after existing runtime helpers. Borrow owned bodies, separate creator/helper local/temporary/control planning, and discover closure-only strings/ranges/remainder/memory operations.
- [ ] Omit environments/hidden arguments and callable-only memory for context-free helpers. Preserve user/import IDs/order, exports, ordinary named/default calls, contracts, and optimization-level parity; expose no generated helpers, tables, or indirect calls.
- [ ] Extend existing checked lowering to verify/cache every helper body and reject forged identities, owner/body relationships, lexical availability, calls, result annotations, and transfer targets. No second IR or independent validator.
- [ ] Implement the parent diagnostic variants/reasons, focused byte spans, staging, and first-error rules for this subset. Reject aliases, callable values/reassignment, annotations, escapes, unsupported named/default closure parameters/contracts, immediate invocation, placeholders, and closure pipelines.
- [ ] Explicitly reject capture-requiring declarations until the capture slice lands; never silently snapshot state or advertise full WP11 support. Freeze concrete interfaces and exclusive shared-file ownership before independent test authors begin.
- [ ] Add source acceptance/rejection and forged-input tests through existing compiler seams, wasmparser validation, independent direct-call/layout/no-memory decoding, bounded ignored-by-default Node effects, and focused CLI byte/diagnostic smoke coverage. Test all optimization levels and deterministic bytes.
- [ ] Reconcile syntax and subset/migration documentation, run focused offline tests and clippy, and report actual coverage/prerequisites. Preserve existing compatibility fixtures and runtime ignored-test policy.
