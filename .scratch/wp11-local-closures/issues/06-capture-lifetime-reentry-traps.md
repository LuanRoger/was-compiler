## Parent

Part of #28 — Implement WP11 local shared-binding closures. Exercise the shared storage and nested live-context model under host integration and repeated invocations.

## What to build

Ensure recursive and host-reentrant creator invocations retain distinct live capture contexts, failed calls do not cause unsafe storage reuse, and returned or host-retained arrays/strings remain valid across later capture allocations and memory growth.

## Acceptance criteria

- [ ] Bounded recursive named creators, with required explicit result annotations, preserve independent parameter/local cells and nested callable environments while sharing each original binding only within its correct invocation.
- [ ] Host callbacks can reenter creator exports without overwriting an outer invocation's live captures/dependencies. All supplied imports and existing host-signature/snapshot rules remain unchanged.
- [ ] After a deliberate bounded trap, subsequent invocations remain correct without rewind, reclaim, or trap-unwind cleanup. Earlier effects remain observable according to the established nontransactional contract.
- [ ] Return arrays and host-created strings from functions that allocate captures; retain their handles across further allocations, recursive/reentrant calls, and traps, and independently verify bytes and array mutations.
- [ ] Exercise actual memory growth and refresh JavaScript views after possible growth. Stored offsets remain valid; stale views are not treated as persistent references.
- [ ] Keep cells/environments monotonic until instance disposal. Verify no general heap-pointer reset or storage reuse invalidates persistent values; document allocation failure, unbounded cumulative growth, and absence of host isolation.
- [ ] Add checked/decoded assertions where needed and ignored-by-default bounded Node/host integration cases with independent expected values/events and all optimization levels. Use existing process deadlines without claiming guest fuel, rollback, or process-tree cancellation.
- [ ] Include focused adapter smoke checks where host-visible memory behavior crosses the existing execution seam. Diagnose and fix root causes revealed by the scenarios rather than merely adding permissive tests.
- [ ] Update lifetime/runtime guidance and report focused offline validation and missing runtime prerequisites. No unbounded recursion/divergence execution, optimizer, separate frame allocator, or new dependency is authorized.
