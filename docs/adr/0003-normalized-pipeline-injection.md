# Preserve pipeline injection in normalized calls

WP10's approved design lowers named-only pipeline stages into nested existing
`TypedNormalizedCall` sequences and adds `CallBindingOrigin::PipelineInput`, rather
than introducing another typed expression kind or disguising the injected input as
written explicit argument 0. This reuses WP6's checked evaluation-order and ownership
boundary while preserving written-argument indices and allowing lowering to verify
that injection is unique, first, and mapped to formal 0.

Issues #25/#26 implemented value-producing stages and named/default binding;
their validation records remain historical subset evidence. Issue #27 completes
WP10 with final Void stages only where the result is discarded;
[release gates passed](../was_language_extensions_plan.md#wp10-release-validation)
on 2026-10-14. Raw pipeline syntax remains distinct until
semantic analysis for stage dependency discovery, whitelist rejection, and source
diagnostics; see the [normative WP10 rules](../was_language_extensions_spec.md#12-feature-specification-pipeline-operator-).

Discard permission is a source-semantic context propagated through discarded
block tails, if branches and match arms, with transparent parentheses. It is not
an operand/argument permission and does not relax structural expression joins.
Keep the existing normalized-call representation without a typed wrapper or public
discard flag: source discard provenance is not wholly reconstructible from forged
typed input, while backend injection ownership, exact-type and flow checks remain
independent. This preserves ordinary binding/default behavior and the full-arity ABI.
