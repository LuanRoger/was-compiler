# Memory ABI

M6 uses memory32 and one `i32` handle for each language `string` or `number[]`.
The same representation is used for locals, user-function parameters/results,
and registered host-function parameters/results. Language types remain distinct
in semantic analysis even though both lower to `i32`; descriptors have no runtime
type tag. Hosts must supply the type declared in their registration.

## Descriptor layout

A handle is an eight-byte-aligned address of this little-endian descriptor:

| Offset | Field | Representation |
| --- | --- | --- |
| `+0` | Data address | Unsigned `u32` byte offset into linear memory |
| `+4` | Length | Unsigned `u32` byte count for strings; element count for arrays |

String payloads contain UTF-8 bytes without a terminator. Embedded NUL is an
ordinary byte. Array payloads contain consecutive little-endian IEEE-754 `f64`
values and must start at an eight-byte-aligned address. Payloads can live separately
from their descriptors, although compiler-generated values place them at `handle + 8`.

Addresses below eight are invalid handles/data pointers. Descriptor headers and
complete payload ranges must fit in current memory; empty payloads can point exactly
to the end of memory. Checks use widened unsigned arithmetic to prevent wraparound.
The guest validates memory-backed parameters at user-function entry, host arguments
and results at host boundaries, and descriptors before indexing, `len`, or string
comparison. Invalid descriptors trap.

String validity and immutability are host obligations: bounds checks do not decode
UTF-8 or enforce read-only memory. Hosts must not alter descriptor metadata after
publishing a value, mutate string payloads, or reinterpret strings as arrays.
Trusted hosts may modify array elements. Exported memory is shared with the host;
these checks are not an isolation mechanism against a host that violates the ABI.

## Ownership and lifetime

- Strings are immutable values. Equal literals share a statically interned
  descriptor; `==` and `!=` compare byte contents, not addresses. There is no
  Unicode normalization or case folding.
- Arrays are mutable references. Assignment, parameter passing, and return copy
  the handle, not the payload. Mutating through one alias changes all aliases.
- Every evaluation of an array literal allocates fresh storage, including inside
  loops and recursive calls. Rebinding a variable does not modify its previous array.
- Both static and allocated values live until the WebAssembly instance is discarded.
  There is no freeing, reference counting, or garbage collector yet. Repeated
  allocation can exhaust memory; this is an intentional initial policy, not an
  optimization-level setting.
- Handles belong to one instance. Do not pass them between instances or retain them
  after their instance is discarded.

## Allocation and exports

Memory-backed modules export:

| Export | Signature | Purpose |
| --- | --- | --- |
| `memory` | `WebAssembly.Memory` | Access descriptor headers and payload bytes |
| `__alloc` | `(i32 byte_count) -> i32 address` | Reserve raw storage for host-created values |

`__alloc` interprets its input as an unsigned byte count, rounds up to eight-byte
alignment, and reserves at least eight bytes even for a zero-byte request. It does
not construct a descriptor; the host fills in the descriptor and payload. Memory
starts large enough for static data and grows in 65,536-byte pages, up to 65,535
pages. Arithmetic overflow, exceeding the address limit, or failed growth traps.
A failed allocation leaves the bump pointer unchanged. Available memory remains
subject to the runtime's own resource limits.

Memory, the heap global, seven internal memory helper functions, and active data
segments are omitted from scalar-only programs. Memory helpers follow all user
functions, so host imports and user function IDs keep their existing indices.
The optional `(f64, f64) -> f64` guest remainder helper follows users and any seven
memory helpers only if `%` occurs anywhere in the typed AST, including unexported,
unreachable, or runtime-skipped code. It requires no implicit host import or
Memory/Global/Data sections; scalar-only programs using `%` still omit the memory
runtime. Programs without `%` retain byte-for-byte output and existing helper
indices. See [M9](compilation-roadmap.md#m9--floating-point-remainder--implemented).

In memory-backed modules, selecting a user function named `memory` or `__alloc`
as an export fails with an export-name collision; unexported functions may use
those source names. User-export selection never hides the mandatory memory ABI
exports.

JavaScript receives `i32` results as signed numbers. Use `handle >>> 0` to recover
an unsigned memory offset. Recreate `DataView`/typed-array views after any operation
that might grow memory: growth can detach the previous buffer. Host callbacks may
reenter `__alloc`; generated modules do not execute a start function.

## Array indexing and length

`len(array)` returns the array's element count as a language `number` (`f64`).
It does not accept strings, following the existing language's built-in signature.

An index must be finite, integral, nonnegative, and strictly below the length.
Negative zero indexes the first element. NaN, infinities, negative/fractional
indices, and out-of-bounds accesses trap rather than being rounded or truncated.
Indexed assignment evaluates the array and index and checks bounds before evaluating
its RHS. Element expressions in array literals execute left-to-right.

## Numeric-array equality

`number[] == number[]` compares contents: lengths must match and each corresponding
pair of elements must satisfy exact `f64` equality. `!=` negates that result.
Two empty arrays are equal; distinct handles may have equal contents. `+0` and `-0`
compare equal, matching infinities compare equal, and NaN compares unequal even
when both operands reference the same array. There is no handle-identity shortcut.

Both operands are evaluated once, left-to-right, before comparison. Comparisons
read current mutable storage; if evaluating the right operand mutates the left
array, the updated contents are compared. Both descriptors are fully validated
before length or element early exits, so malformed descriptors trap rather than
silently compare unequal. The helper allocates nothing and changes no memory;
worst-case comparison is O(n) time and O(1) auxiliary space. Array assignment still
copies a handle and aliases storage, not contents.

## Explicit marshaling and raw ABI

The [Node runtime](runtime-integration.md) passes raw Wasm arguments and results,
including `i32` string/array handles. CLI and library binaries use the same ABI;
no language-signature metadata or automatic native-value marshaling is involved.

`createAbi(() => instance)` from `runtime/abi.cjs` offers explicit
`encode(type, value)` and `decode(type, value)` operations. Encoding a string or
numeric array allocates and copies it into guest memory; decoding copies it into
a native value. Mutating a decoded array does not mutate guest storage. The helper
refreshes memory views after growth, strictly decodes UTF-8, and limits each
payload copy to 64 MiB. String encoding follows `TextEncoder`, including replacement
of unpaired UTF-16 surrogates. Numeric-array encoding accepts plain JavaScript
arrays of numbers, not typed arrays.

Use direct memory access for shared array mutation, zero-copy access, or
instance-local handle identity. The following example implements explicit
conversion using the raw WebAssembly API.

## Host roundtrip example

Register memory-backed imports through the existing API:

```rust
use was_compiler::{Compiler, Type};

let mut compiler = Compiler::new();

compiler.register_host_function(
    "echo_text",
    "env",
    "echo_text",
    vec![Type::String],
    Type::String,
)?;

compiler.register_host_function(
    "adjust",
    "env",
    "adjust",
    vec![Type::NumberArray],
    Type::NumberArray,
)?;

let wasm = compiler.compile_source(r#"
    fn greet() -> string {
        return echo_text("Hello!");
    }

    fn total() -> number {
        let values: number[] = adjust([1, 2, 3]);
        return values[0] + values[1] + values[2];
    }
"#)?;

std::fs::write("module.wasm", wasm)?;
```

Registration declares imports, not callbacks. The embedding runtime implements
those callbacks using the module's own memory. This Node.js example consumes binary
Wasm, not WAT:

```javascript
const fs = require('node:fs');
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
let guest;

function view() {
    return new DataView(guest.memory.buffer);
}

function readString(handle) {
    handle >>>= 0;
    const data = view().getUint32(handle, true);
    const length = view().getUint32(handle + 4, true);
    return decoder.decode(new Uint8Array(guest.memory.buffer, data, length));
}

function writeString(value) {
    const bytes = encoder.encode(value);
    const handle = guest.__alloc(8 + bytes.length) >>> 0;
    const data = handle + 8;
    const memory = view();
    memory.setUint32(handle, data, true);
    memory.setUint32(handle + 4, bytes.length, true);
    new Uint8Array(guest.memory.buffer, data, bytes.length).set(bytes);
    return handle;
}

function readArray(handle) {
    handle >>>= 0;
    const memory = view();
    const data = memory.getUint32(handle, true);
    const length = memory.getUint32(handle + 4, true);
    return Array.from({ length }, (_, index) => memory.getFloat64(data + index * 8, true));
}

function writeArray(values) {
    const handle = guest.__alloc(8 + values.length * 8) >>> 0;
    const data = handle + 8;
    const memory = view();
    memory.setUint32(handle, data, true);
    memory.setUint32(handle + 4, values.length, true);
    values.forEach((value, index) => memory.setFloat64(data + index * 8, value, true));
    return handle;
}

const module_ = new WebAssembly.Module(fs.readFileSync('module.wasm'));
const instance = new WebAssembly.Instance(module_, {
    env: {
        echo_text: handle => writeString(readString(handle) + ' from host'),
        adjust: handle => writeArray(readArray(handle).map(value => value + 1)),
    },
});

guest = instance.exports;

console.log(readString(guest.greet()));
console.log(guest.total());
```

This prints `Hello! from host` and `9`. The callbacks copy incoming values before
allocating their results, and obtain fresh memory views after allocation.
