## Parent

Part of #28 — Implement WP11 local shared-binding closures. This slice adds capture storage to existing range/array iteration semantics.

## What to build

Allow closures to capture range and numeric-array for-loop bindings while preserving their read-only provenance and allocating a distinct shared cell for each successfully yielded iteration.

## Acceptance criteria

- [ ] Capture iteration bindings as read-only numbers. Reject ordinary and compound source writes through captures, including unreachable/skipped writes; preserve mutable inner shadowing lets as distinct original bindings.
- [ ] Allocate a fresh cell after a successful range yield or checked array element load and before body execution. Empty/exhausted iterations allocate no cell, and iteration cells are never reused/reclaimed.
- [ ] Preserve once-only iterable evaluation/validation, enclosing iterable control context, range endpoint-before-conversion rules, array checked address/load, captured handle semantics, and alias mutations visible on later yields.
- [ ] Preserve continue's existing latch/advance/retest path and nearest-loop targets. Closure transfers cannot target creator loops merely because their binding was captured.
- [ ] Reconstruct read-only provenance from iteration ownership during checked lowering, not trusted public mutability flags. Reject forged iteration/capture owners, type/initialization records, and illegal writes.
- [ ] Add source acceptance/rejection, forged trees, wasmparser validation, independent allocation placement/loop-depth/storage decoding, and bounded ignored runtime cases for empty/nonempty ranges and arrays, continue/break, source rebinding, alias mutation, and inner shadowing.
- [ ] Provide independent evidence of fresh allocation per yield rather than only checking values that would also pass with a reused cell. Test allocation-growth/trap ordering and no allocation on failed/exhausted yields where feasible.
- [ ] Verify deterministic output and optimization parity with focused CLI/core checks. Document fresh iteration storage and monotonic growth and run focused offline validation.
- [ ] Leave combined nested/transitive iteration-capture acceptance to the final convergence gate; this slice does not require the nested-dependency ticket to start.
