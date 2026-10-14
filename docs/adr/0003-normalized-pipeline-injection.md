# Preserve pipeline injection in normalized calls

WP10's approved design lowers named-only pipeline stages into nested existing
`TypedNormalizedCall` sequences and adds `CallBindingOrigin::PipelineInput`, rather
than introducing another typed expression kind or disguising the injected input as
written explicit argument 0. This reuses WP6's checked evaluation-order and ownership
boundary while preserving written-argument indices and allowing lowering to verify
that injection is unique, first, and mapped to formal 0.

This is an accepted design for proposed WP10, not implemented support. Raw pipeline
syntax remains distinct until semantic analysis for stage dependency discovery,
whitelist rejection, and source diagnostics; see the
[WP10 interview decisions](../was_language_extensions_plan.md#wp10--named-only-pipelines-spec-12).
