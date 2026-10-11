# Use one private allocated range representation in WP5

WP5 adopts one immutable bump-allocated range record rather than introducing both
handle and allocation-free scalar representations. This keeps internal calls and
checked lowering uniform while supporting exact safe-integer membership semantics;
scalar replacement is deferred to WP13 until measurements and semantic proofs
justify its complexity.

Range handles are internal only: host imports and selected user exports with range
signatures are rejected rather than extending the public descriptor ABI or silently
omitting requested exports. This avoids committing to a host range-memory contract
before marshaling, ownership, and validation requirements are designed.

The approved layout, boundary checks, and export-selection rules are specified in
[the extension specification, §5.4](../was_language_extensions_spec.md#54-representation-and-performance)
and [the WP5 implementation plan](../was_language_extensions_plan.md#wp5--safe-ranges-rangearray-for-membership-spec-56).
WP5's first checkpoint now implements construction and internal calls with this
representation and boundary policy. Construction evaluates all numeric operands
once, left-to-right, before eager safe-integer/nonzero-step validation, including
invalid, empty, and discarded ranges. Copies share the immutable allocated record;
internal range parameters validate its bounds and canonical fields. Exact numeric
membership (`in`) is now implemented using signed `i64` interval and congruence
checks. The consumed record is validated before the candidate guard, so malformed
records trap even for invalid candidates; valid records return false for nonfinite,
fractional, or out-of-safe-domain candidates and accept both signed zeros.
Range helpers append `Construct`, then `Validate` after memory and optional
remainder helpers; optional `Contains` follows only when membership exists.
Construction-only and feature-free bytes remain unchanged; membership requires no
floating remainder helper. See the [memory ABI](../memory-abi.md#private-range-record)
for the arithmetic bounds proof. This is not completion of WP5: progression
advancement, iteration, and `for` remain proposed.
