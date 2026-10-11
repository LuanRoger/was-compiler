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
These decisions authorize implementation; they do not indicate WP5 is implemented.
