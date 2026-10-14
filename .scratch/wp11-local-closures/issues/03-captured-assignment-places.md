## Parent

Part of #28 — Implement WP11 local shared-binding closures. This slice completes assignment-place semantics on top of shared capture storage.

## What to build

Make captured numeric compound updates and indexed assignment/compound operations preserve the original checked target and old value across RHS closure effects, including reassignment of the captured array binding.

## Acceptance criteria

- [ ] Support captured numeric binding updates with all five compound operators, retaining existing numeric type rules and floating remainder behavior. Ordinary noncaptured places remain compatible.
- [ ] For indexed ordinary assignment, resolve the original array target and validate its descriptor/bounds before RHS evaluation. For indexed compounds, also save the old f64 value before RHS effects and store to the original checked element address.
- [ ] RHS closure effects that rebind the array variable do not redirect the pending write. Array aliases and later closure reads observe their respective established handles and updated contents.
- [ ] Captured scalar compounds retain the old value across RHS mutation and write the computed result to the original shared cell. Evaluate each target/index/RHS once, in established left-to-right order.
- [ ] Keep compound assignment statement-only and numeric-only; preserve invalid-target/type diagnostics, read-only provenance, skipped/unreachable validation, and checked forged-input rejection.
- [ ] Extend the existing checked place/lowering interface, not a separate assignment implementation. Preserve typed scratch/local planning and branch/stack correctness under divergent or transferring RHS expressions.
- [ ] Add source acceptance/rejection and forged typed cases, wasmparser validation, independent saved-address/old-value/call-order decoding, and bounded ignored runtime effects for scalar RHS mutation and array-binding rebinding.
- [ ] Cover invalid descriptors/bounds trapping before RHS events, ordinary indexed assignment, all compound operators, aliases, and applicable NaN/infinity/signed-zero/extreme numeric behavior with independent expectations.
- [ ] Verify all optimization levels, deterministic output, existing compound compatibility coverage, and focused CLI/core parity. Update implemented-subset documentation and run focused offline validation.
