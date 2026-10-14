## Parent

Part of #28 — Implement WP11 local shared-binding closures. Use the established dedicated identity and private capture-storage interfaces.

## What to build

Make nested closures share original captured cells and make later/nested helpers directly invoke earlier local callables using the original callable's live context, including through intermediate closures with no direct value reads.

## Acceptance criteria

- [ ] Resolve nested value captures to the original binding owner/ID and preserve shared type/mutability instead of copying intermediate values or allocating duplicate original cells.
- [ ] Distinguish lexical callable dependencies from value captures and callable aliases. Preserve earlier-callable identity, lexical availability, initialization, and live environment through nested scopes and intervening helpers.
- [ ] Forward the original callable environment address, not copied/reconstructed environments. A helper without direct value captures still receives context if its callable dependencies require it.
- [ ] Canonically order value-capture slots before callable-dependency slots; order dependencies by callable source discovery, deduplicate metadata, omit pointer slots for context-free dependencies, and reject duplicate/noncanonical public records.
- [ ] Preserve declaration-order visibility, initializer-before-binding resolution, later shadowing behavior, direct-only invocation, independent control contexts, and named-function result/recursion rules. No new callable value, self/forward recursion, or indirect dispatch.
- [ ] Verify complete transitive capture/dependency requirements and owner relationships in existing checked lowering, including forged missing/extra/wrong-owner/type/context records and unavailable or uninitialized dependencies.
- [ ] Validate all nested helper bodies, including unexported/unreachable/skipped syntax, with independent owner-local plans and source-preorder helper ordering rather than inference/hash ordering.
- [ ] Add frontend/forged-input tests, wasmparser validation, independent signatures/dependency slot/direct-target decoding, and bounded ignored runtime cases for shared mutation, original-versus-shadowed binding identity, nested helper parameters/locals, sibling helper composition, and context-forwarding-only intermediate closures.
- [ ] Check context-free dependency omission and all optimization levels, deterministic bytes, ordinary named/default/contract calls, and focused CLI/core parity. Document the implemented nesting/dependency contract and run focused offline validation.
