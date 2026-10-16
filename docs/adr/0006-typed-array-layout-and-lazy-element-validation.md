# Keep typed array layouts untagged and validate elements on consumption

The memory evolution design retains pointer/count descriptors, using static element
types to select numeric, byte-boolean, or string-handle payload operations. Array
entry checks validate complete payload bounds without scanning its elements;
checked access validates consumed booleans/string handles instead. This avoids
making every array parameter check O(n), at the cost of deferring failures in
unread elements and retaining the trusted-host obligation not to reinterpret types.

Runtime tags and eager element scans were considered but are not needed for the
initial fixed-length scalar-element arrays. Boolean payloads use canonical byte
values `0`/`1`; string payloads use four-byte references to immutable strings.
Eight-byte descriptors and allocator alignment remain unchanged. This is not a
security mechanism against a host with access to exported memory, nor permission
to assume allocation provenance or valid UTF-8 from structural checks.

These design decisions were confirmed on 2026-10-15. The full
[design interview](../memory-evolution-design.md) remains in progress; this ADR does
not authorize implementation or claim release validation. Nested arrays, reclamation,
and optimization transformations remain outside the initial scope.
