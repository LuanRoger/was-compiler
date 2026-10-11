# Use one private allocated range representation in WP5

WP5 adopts one immutable bump-allocated range record rather than introducing both
handle and allocation-free scalar representations. This keeps internal calls and
checked lowering uniform while retaining exact safe-integer progression semantics;
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
internal range parameters validate its bounds and canonical fields. Range helpers
append after memory and optional remainder helpers; feature-free bytes remain
unchanged. This is not completion of WP5: exact progression, membership (`in`),
iteration, and `for` remain proposed.
