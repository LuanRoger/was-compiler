# Preserve local callable ownership with shared capture storage

WP11's accepted [design interview](../wp11-local-closures-development.md) uses
owned typed closure declarations, dedicated callable/owner identities, and explicit
transitive lexical callable dependencies rather than first-class callable values,
ordinary concrete-typed lets, or additional public user functions. This preserves
lexical availability and original binding provenance so the existing checked
lowering path can verify mutable public typed input without another IR or validator.

Generate direct helpers using shared original-binding cells and private environments;
retain earlier callables' live context rather than copying environments or values.
Use eager promotion and monotonic bump allocation with uniform eight-byte cells and
pointer slots, accepting additional space and allocation trap points for a simple,
deterministic initial implementation. Lazy promotion, packed slots, and reclaimable
frames are deferred: general heap rewind could invalidate returned or host-retained
arrays/strings, and reclamation needs separate recursion, reentrancy, and trap-lifetime
proofs. Layouts are backend-private, not a public runtime callable ABI.

These architecture decisions and final shared understanding were confirmed on
2026-10-14; WP11 remains proposed. This ADR does not authorize implementation or
claim release validation. The completed handoff records the accepted contract.
