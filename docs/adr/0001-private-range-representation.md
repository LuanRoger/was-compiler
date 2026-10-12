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
for the membership and advancement bounds proofs.

Issue #4 adds range-only statement `for` without changing this representation or
helper ordering. The iterable evaluates once in the enclosing context, before
activating the loop/binding; consumed record validation precedes caching exact
`i64` current/end/step and `i32` inclusivity scratch. A body latch makes continue
reach `I64Add` advancement and signed endpoint retesting before exact `f64` yield.
With `M = 2^53−1`, `|current + step| <= 2M < 2^54` fits i64; a next value outside
the safe domain fails the endpoint test and exhausts normally before conversion,
with no further addition. Yielded zero is canonical +0. No new range helpers or
public mutable-binding metadata are introduced: checked lowering reconstructs
read-only iteration-binding restrictions from For provenance. This is not
completion of WP5: broader WP5 adapter/interaction release gates remain pending.

Issue #5 extends the same statement For to `number[]` without changing parser/AST
shapes or this range decision. Arrays use the existing public descriptor ABI, not
private range records: evaluate once before activating the loop/binding, fully
validate, then cache the handle and `u32` length. Unsigned index iteration loads
current elements through checked `ArrayAddress`, so alias mutation is visible
without snapshotting; source reassignment leaves the captured handle unchanged.
The numeric binding remains read-only with mutable inner shadowing, and continue
uses the index-advancement latch. No ABI/helper-order changes are introduced;
array-only iteration needs no range helpers. Current CLI/browser compilation uses
the shared core for both iterable types, but range and array iteration remain
partial WP5 until the adapter/interaction release gates are met.
