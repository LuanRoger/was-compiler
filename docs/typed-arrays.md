# Typed arrays — issue #37

## Implementation and validation status

[Issue #37](https://github.com/LuanRoger/was-compiler/issues/37) defines this
extension. The compiler core, CLI/browser adapters and explicit Node marshaling
are implemented for all three array types. The final ordered offline release
sequence passed after the new-array accessor TOCTOU fix: **1,054 passed / 188
ignored** in the default suite, **1,251 passed / 0 ignored** in the full workspace
run, **3 workspace-contract tests**, **25 browser smoke groups**, both Clippy
checks, the portable core Wasm build and current-source browser rebuild.
All gates exited 0; see [exact commands, output, tool versions and final hashes](typed-arrays-validation-2026-10-15.txt).
The October 15 release record retains actual run timestamps on October 16 UTC.
Earlier WP5/WP11 evidence remains historical, not #37 validation.

M4 [comparison tables](benchmarks/m4-typed-arrays/comparison.md) and
[ranked follow-up handoff](benchmarks/m4-typed-arrays/handoff.md) are complete as
historical measurements. Their runtime hash predates the accessor fix; the new
release record separately identifies final production/test sources and rebuilt
artifacts. These correctness runs do not remeasure historical timings or imply
that the old runtime hash matches the final source.

The [memory ABI](memory-abi.md) owns layout and consumption rules;
[runtime integration](runtime-integration.md#typed-array-host-integration-37) and
[the browser guide](browser.md#typed-arrays-37) describe the implemented adapters.
[ADR 0006](adr/0006-typed-array-layout-and-lazy-element-validation.md) records the
untagged-layout/lazy-validation decision. This extension improves extensibility,
not reclamation or optimization; the allocator remains monotonic.

## Language contract

Supported arrays are `number[]`, `bool[]`, and `string[]`. They are fixed-length,
mutable shared references. Assignment, arguments, returns and captures share a
handle; each evaluated literal allocates fresh storage, including empty and
discarded literals. String elements refer to immutable strings: replacement
changes the reference, not the string's bytes.

```text
fn choose(values: bool[], fallback: bool[] = values) -> bool[] {
    return fallback;
}
fn empty_texts() -> string[] { return []; }
fn main() -> bool {
    let flags = [true, false];
    let alias = flags;
    flags[1] = true;
    let empty: bool[] = [];
    for flag in choose(flags) {
        if !flag { return false; }
    }
    return alias == [true, true] && len(empty) == 0;
}
```

Indexing yields the exact scalar element type. Ordinary indexed replacement,
`len(array)` (returning `number`), same-type content equality, iteration, locals,
parameters/results, blocks, if/match values and value loops support all three
arrays. Boolean equality compares values; string-array equality compares string
contents, not handles. Numeric equality retains exact IEEE-754 semantics: signed
zeros compare equal, matching infinities compare equal, and NaN is unequal even
when comparing an array to itself. `!=` complements `==`. Different array types
cannot be compared or assigned without an explicit application-level conversion.

`for` yields a read-only scalar `number`, `bool`, or `string` according to the
iterable; range iteration still yields `number`. An inner shadowing let is mutable.
Captures retain original binding type/mutability, shared cells, forwarding and
invocation-local lifetime behavior. Capturing an array shares its handle/storage;
capturing an iteration binding retains that iteration's yielded scalar value in
its original read-only cell. Uniform eight-byte capture cells/environments do not
shrink to match boolean payload width or string-array slot width.

All five compounds (`+= -= *= /= %=`) remain numeric-only: numeric variables and
`number[]` elements, including captured places. Neither boolean/string elements
nor whole arrays accept compounds, including in unreachable code.

There are no nested arrays, `range[]`, `void[]`, growable lists, slices, chained
indexing, implicit coercions, first-class callables, or new anonymous-result
annotation syntax. String `len`/indexing/concatenation remain unsupported.

## Contextual literal typing and stable inference

- A known annotation, assignment destination, mapped call formal, or explicitly
  annotated named-function result supplies an expected array type. Parentheses,
  value-block tails, if branches, match arms and value-loop break paths propagate
  that expectation. Indexed replacement expects its scalar element type.
- Without an expectation, nonempty literals infer one exact supported scalar type
  from elements that normally complete; mixed types are rejected. Empty and
  all-Never literals retain the `number[]` storage fallback. A destination can
  select bool/string storage for an all-Never literal without making it complete.
  Every element remains checked, even after divergence.
- Sibling branches, other returns, equality operands and callers do not supply
  backward inference. For example, unannotated `if true { [] } else { [true] }`
  does not infer `bool[]`; `let flags: bool[] = if true { [] } else { [true] };`
  does. `flags == []` does not reinterpret the right operand from the left.
- Omitted named-function results and anonymous callable results retain independent
  inference across probe/final checking. `fn empty() { return []; }` and
  `let empty = () => [];` infer numeric-array results; a later bool-array use
  rejects them rather than specializing their bodies. Concrete call results are
  checked against destinations, never specialized backward.
- Named arguments are mapped before selecting formal expectations. Structural
  binding errors retain precedence; explicit expressions execute in written order,
  defaults afterward in formal order, with existing hygiene and caller transfers.
  This applies to concrete positional formals as well as named user calls.
- Pipeline inputs are analyzed independently before stage lookup/binding.
  `[] |> take_flags` remains numeric and fails if formal 0 expects `bool[]`.
  Later stages cannot reinterpret earlier inputs/call results. Written stage
  arguments receive their mapped remaining formal expectations normally.
- Defaults/contracts retain their existing whitelist: array-reference reads,
  same-type equality and `len` generalize, but array literals and indexing are
  still forbidden, including skipped operands. A default can alias an earlier
  array formal; it cannot construct `[]`. Pure does not mean nontrapping.

## Execution and host storage

See [descriptor layout](memory-abi.md#descriptor-layout) and
[typed-array element consumption](memory-abi.md#typed-array-element-consumption-37).
Descriptors are untagged pointer/count pairs; static types or explicit host type
arguments select widths. String payload bytes have width 1 and byte lengths;
`string[]` payload slots have width 4 and element counts. Boolean slots have width
1 and accept only bytes `0`/`1` when consumed, independently of scalar host-bool
normalization.

Outer validation checks complete storage bounds/alignment without element scans.
Length and parameter checks do not consume elements. Access/iteration/equality
validate only consumed bool bytes/string handles; invalid unvisited elements may
remain untrapped. UTF-8 validity and string immutability remain host obligations.
There is no runtime type tag, provenance check or malicious-host isolation.

Literal storage/descriptor initialization precedes element effects. Replacement
checks and retains the destination before RHS effects, validates the replacement,
and stores without consuming the overwritten element. Equality checks both outer
storages before length exits and consumes left then right values in index order
until a mismatch. Iteration caches handle/count, checks each live access and
validates the yielded value before binding/capture promotion/body effects; aliases
can change later elements. Transfers may skip consumption. Traps retain completed
effects/allocations; saved Wasm offsets survive growth, JavaScript views do not.

## Public Rust migration

Import both types from the library root:

```rust
use was_compiler::{ArrayElementType, Type};

let numbers = Type::Array(ArrayElementType::Number);
let flags = Type::Array(ArrayElementType::Bool);
let texts = Type::Array(ArrayElementType::String);
```

- Replace every former `Type::NumberArray` constructor/pattern with
  `Type::Array(ArrayElementType::Number)`. There is no compatibility variant or
  deprecated alias. Numeric source syntax and descriptor bytes are unchanged.
  Legacy numeric diagnostic formatting may still say `NumberArray`; that is not
  a Rust variant.
- Exhaustive `Type` matches must distinguish `Array(element)` and all three
  `ArrayElementType` cases. The closed, small `Copy` model adds no source generics
  or recursive array types. `ArrayElementType::value_type()` yields its scalar
  `Type`.
- `TypedExprKind::Array(Vec<TypedExpr>, ArrayElementType)` now records storage
  independently of `TypedExpr.ty`, including when that category is Never.
  Update constructors and matches, not just enclosing result annotations.
- `TypedStatementKind::For` adds `element_type: Type`: `Number` for ranges/numeric
  arrays, `Bool` for boolean arrays, `String` for string arrays. Preserve all
  existing IDs, scopes and iterable/body fields.
- Host registration uses the same `Type::Array(...)` signatures; all arrays lower
  to raw `i32` handles and are eligible for imports/selected exports in core.
  External range restrictions remain unchanged. Forged public typed trees still
  receive independent backend element/storage/result and iteration-metadata checks,
  including unreachable/skipped children.

Old-type-only modules retain established helper emission/order/indices and golden
compatibility requirements. New-type helpers are selected deterministically;
this work does not authorize general legacy-helper pruning or optimization.

<a id="node-copy-contract-under-integration"></a>

## Node copy contract

Explicit `createAbi` encode/decode supports `bool[]` and `string[]`, with plain
JavaScript arrays containing only booleans or only strings. Encoding and decoding
copy values; mutating native inputs or decoded arrays does not mutate guest
storage. Direct raw-memory access remains the explicit sharing/zero-copy route.
Refresh views after every allocation/call that can grow memory, including nested
string allocations. Handles remain instance-local.

New-array encoding reads the initial input length once and captures each indexed
scalar once, validating its type and accounting its UTF-8 bytes against the same
snapshot that is copied. Accessors cannot substitute unchecked values or alter
the copied count between validation and storage. Oversized/invalid captured input
is rejected before guest instance access/allocation; this is not atomic native
snapshotting or rollback of getter side effects. Getter regressions for changing
values, growing/shrinking lengths and aggregate budgets passed in the final run.

Each new-array encode/decode operation has a **64 MiB (67,108,864 byte) aggregate
logical-copy budget**:

- `bool[]` of length `N`: `N` bytes.
- `string[]` of length `N`: `4 * N + sum(UTF8_bytes(each occurrence))` bytes.
  Repeated strings/handles count repeatedly; even empty strings retain the
  four-byte-per-slot contribution.

Accounting is checked; this is not a JavaScript object-overhead, RSS, guest heap
or execution quota. Existing standalone string and numeric-array 64 MiB payload
limits remain unchanged. No automatic signature metadata/marshaling is emitted.

<a id="pending-release-evidence"></a>

## Release validation

The [final release record](typed-arrays-validation-2026-10-15.txt) contains all
eight commands in execution order, their complete output/counts, UTC timestamps,
tool versions, final production and validation-input SHA-256 manifests, rebuilt
browser artifact hashes and unchanged-source verification. All **78 production
files and 99 validation inputs** in the manifest remained unchanged across the
sequence. The default and full suites overlap; their totals are not unique-test
counts to add together.

The full run includes these #37 targets (all passed, none ignored):

| Target | Passed |
| --- | ---: |
| Core `typed_arrays` | 8 |
| Core `typed_arrays_lowering` | 2 |
| Core `typed_arrays_runtime` | 5 |
| Core `typed_array_marshaling` | 6 |
| Core `typed_array_measurements` | 6 |
| CLI `typed_arrays_adapters` | 4 |
| Browser `typed_arrays_adapters` | 1 |

The six marshaling cases include both accessor regressions. Rebuilt-browser
coverage, raw host storage, copy isolation/budgets/growth, forged typed metadata,
named/default/contextual typing and captured runtime effects are covered through
the existing test seams. Runtime/browser cases remain ignored by default; only
the explicit full run establishes their execution. Follow [current validation](validation.md)
and [root commands](../AGENTS.md#validation-commands) for future changes.

M4 [measurements](benchmarks/m4-typed-arrays/comparison.md) and the
[optimization handoff](benchmarks/m4-typed-arrays/handoff.md) retain their original
hashes and scope. They show no attributable timing speedup; smaller new-type
payloads follow widths, not numeric allocation elimination or reclamation.
WP13 changes still require separate authorization. The old pending-evidence and
under-integration anchors are retained for existing links.
