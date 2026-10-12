# CLI and runtime integration

The CLI compiles a required source file to raw binary Wasm or plain WAT and
supports explicit export selection. The dependency-free Node.js runtime calls
the raw Wasm ABI. The compiler does not execute Wasm: compilation and execution
are separate commands, and runtime callbacks must be supplied explicitly.

Run these commands from the repository root. Rust dependencies must already be
available locally for `--offline`; runtime examples require Node.js.

## Quick start

Compile the checked-in array example, exporting only `main`:

```text
cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main
node runtime/run.cjs target/array-sum.wasm
```

The runner prints `65`. The `sum` body remains callable by guest code, but is not
exported. To call `sum` from JavaScript, export it and explicitly allocate an array
descriptor as shown under [Programmatic Node interface](#programmatic-node-interface).
The runner does not turn a native JSON array into a guest array.

To inspect plain WAT or display help:

```text
cargo run --offline -- examples/array-sum.was --emit wat
cargo run --offline -- examples/array-sum.was -o target/array-sum.wat
cargo run --offline -- --help
node runtime/run.cjs --help
```

## Compiler CLI

```text
was-compiler [OPTIONS] <SOURCE>
```

| Option | Behavior |
| --- | --- |
| `SOURCE` | Required UTF-8 source-file path (`PathBuf`). Its extension is not significant. `-` is a literal filename, not stdin. |
| `-o PATH`, `--output PATH` | Write an artifact. Parent directories must already exist. `-` means text stdout. |
| `--emit wasm` | Produce raw binary Wasm; requires a filesystem output path. |
| `--emit wat` | Produce plain WAT. Defaults to stdout without `-o`. |
| `--export NAME` | Export only these user-function names; repeat for more names. |
| `--no-exports` | Export no user functions. Cannot be combined with `--export`. |
| `--host DECL` | Declare a host import and its language signature; repeat for more imports. |
| `-O0` … `-O3`, `-O LEVEL` | Select existing presets; they are all no-ops today. |
| `--` | Treat remaining arguments as source paths, including a source filename beginning with `-`. |
| `-h`, `--help` | Print usage and exit `0`. |

The CLI uses `clap` derive for argument parsing, generated help, and usage errors.
Long options with values also accept `--option=value`, such as `--emit=wasm`.
No arguments or invalid arguments exit `2` with an `error:` message and help hint.
There is no built-in example fallback or stdin support. Host registration and
compiler validation remain separate from argument parsing.

Without `--emit`, an output filename ending in the exact lowercase extension
`.wasm` implies binary Wasm; all other cases default to WAT. Explicit `--emit`
overrides extension inference. Binary stdout is intentionally refused so terminal
output remains readable text. The CLI rejects multiple source files,
conflicting/repeated singleton options, unknown options, unknown selected exports,
and duplicate selected names.

Host declarations use:

```text
NAME=MODULE.FIELD(PARAMETER_TYPES)->RESULT_TYPE
```

Types are `number`, `bool`, `string`, `number[]`, and `void`; `void` is only legal
as a result. Empty parentheses mean no parameters. The declaration's source name
still follows host-registration identifier/reserved-name rules. This compact
notation is not an escaping format for arbitrary import names; the Rust API is
available when module/field names need characters that collide with delimiters.

Compilation finishes before an output file is touched. Files are written to an
exclusively created sibling temporary file, synced, then renamed into place;
failed writes clean up that temporary file. Output resolving to the source's
canonical path is refused. This preserves existing artifacts on compilation errors
and avoids publishing a partially written file. It does not promise directory-sync
crash durability or race-proof source-file identity checks.

## Export selection in Rust

```rust
use was_compiler::{Compiler, ExportOptions};

let mut compiler = Compiler::new();
compiler.set_exports(ExportOptions::Only(vec!["main".into()]));

let wasm = compiler.compile_source(
    "fn main() -> number { return helper(); } fn helper() -> number { return 7; }",
)?;
```

`Compiler::exports()` exposes the current policy. `ExportOptions::All` is the
default; `None` exports no user functions. This policy is separate from the
existing, copyable `CompilerOptions` and its optimization level.

The lower-level API exposes both `codegen::compile(program)` (all user exports)
and `codegen::compile_with_exports(program, &policy)`. Export entries retain
source order even when the requested list uses another order. Selection removes
no function body, signature, registered import, static data, or runtime helper;
indices and validation of unexported bodies remain intact.

Memory-using modules still export `memory` and `__alloc`, including with `None`.
Selecting user functions with those names in a memory-backed module conflicts
with mandatory ABI exports; unexported functions may use those source names.
Scalar-only modules may export functions with those names. The Node runtime does
not reserve function names or hide callable ABI helpers.

## Internal ranges and numeric membership

The CLI/library support `number in range` returning `bool`, with comparison-tier
precedence and existing left associativity. Both operands evaluate once,
left-to-right; exact `number`/`range` types allow Never compatibility and all
children remain checked. `in` is reserved for source/host-language names, not
host module/field names. Membership validates the consumed record before the
candidate guard: malformed records trap even for invalid candidates. Nonfinite,
fractional, or out-of-safe-domain candidates return false; both signed zeros are
accepted. Exact signed `i64` interval/congruence checks require no host callback
or floating remainder helper. Optional `Contains` follows `Construct`/`Validate`
only when membership exists, preserving construction-only bytes.

Range handles remain private: reject all range host imports and selected range
user exports. Use `--export main` when helpers have internal range signatures;
default export-all rejects those signatures. A scalar membership function may
be exported normally; its bool result uses raw `i32` `0`/`1`. There is no public
range marshaling API.

Range-only statement `for name in range_expr { body }` uses the same private
records and needs no new host callback or range helper. `for` is reserved for
source/host-language names, not host module/field names. Its iterable evaluates
once in the enclosing context before the loop/binding is active; iterable
break/continue targets the outer nearest loop, and return/divergence is preserved.
The body has a read-only `number` binding (a distinct inner shadowing let remains
mutable), permits bare breaks, and rejects value breaks even when unreachable.
Continue reaches advancement before retesting. Cached `i64` fields and signed
endpoint checks before exact `f64` yield avoid rounding stalls; yielded zero is
`+0`, and a valid advance past bounds exhausts normally even outside the safe domain.
For is a void statement, not a value expression, and adds no public range ABI.
CLI and browser compilation already support range-only for through the shared
core. Array iteration and broader WP5 adapter/cross-feature acceptance remain
proposed, not pending adapter support for the current feature or full WP5 completion.
`tests/for_frontend.rs` covers frontend contracts, `tests/for_lowering.rs` covers
forged input and decoded lowering, and `tests/for_loops.rs` covers deterministic
layout/imports/exports and bounded Node execution at all optimization levels.
The runtime cases check exact yields/exhaustion, effects, transfers, record validation
and caching. They are ignored by default; default Cargo success alone does not
establish runtime coverage.
See the [private record contract](memory-abi.md#private-range-record).

## Host callbacks

Imports are raw callbacks, not typed marshaling wrappers. A CommonJS provider
can export an object of module namespaces, for example `{ env: { log_number } }`,
or a factory `getInstance => rawImports`. The factory can use
`createAbi(getInstance)` for explicit string/array conversion inside callbacks.
The getter returns the instantiated `WebAssembly.Instance` once available, but
returns `undefined` during Start callbacks; those callbacks cannot use the
instance's memory ABI. Compiler-generated modules have no Start function.

No implicit `env`, I/O, or WASI implementation is installed. Provider paths resolve
from the current working directory. Every registered import must be provided,
even if unused by the selected entry. Providers are trusted Node.js code and
retain Node's host capabilities.

| Language type | Raw host argument/result |
| --- | --- |
| `number` | JavaScript number representing `f64`, including NaN and infinities. |
| `bool` | `i32` number; generated host boundaries normalize nonzero values to `1`. |
| `string`, `number[]` | `i32` descriptor handle in the current instance's memory. |
| `void` | No Wasm result. |

Hosts must obey the declared signatures. Host callbacks receive descriptor handles,
not native strings or arrays, and must return handles for memory-backed results.
Use the instance's `memory` and `__alloc`, optionally via explicit `createAbi`
conversion. Decoding an array produces a copy; mutate guest memory directly when
reference mutation is intended. See [memory-abi.md](memory-abi.md) for layouts,
bounds, lifetime, and a complete raw host roundtrip example.

The checked-in `examples/host-provider.cjs` exports a factory using
`createAbi(getInstance)` to explicitly decode incoming string/array handles and
encode return handles. Compile and run its source example:

```text
cargo run --offline -- examples/host-calls.was -o target/host-calls.wasm --export main --host 'echo=env.echo(string)->string' --host 'print=env.print(string)->void' --host 'adjust=env.adjust(number[])->number[]'
node runtime/run.cjs target/host-calls.wasm --imports examples/host-provider.cjs
```

This prints `Hello from host` followed by `12`.

Different source names may target one import module/field. One JavaScript callback
must then satisfy every declared raw signature; the runtime does not infer language
types or build signature-specific wrappers. Prefer distinct properties for
incompatible contracts. Calls are synchronous; Promises are not awaited and are
not substitutes for declared scalar or descriptor results.

## Programmatic Node interface

Compile an artifact exporting `sum`:

```text
cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main --export sum
```

Explicitly marshal its array argument:

```javascript
const fs = require('node:fs');
const { instantiate } = require('./runtime/index.cjs');
const { createAbi } = require('./runtime/abi.cjs');

const runtime = instantiate(fs.readFileSync('target/array-sum.wasm'), {});
const abi = createAbi(() => runtime.instance);
const handle = abi.encode('number[]', [1, 2, 3]);
console.log(runtime.call('sum', [handle]));
```

This prints `6`. `instantiate(bytes, imports)` returns `{ instance, call }`.
`call(name, args)` checks the exported arity and accepts numeric raw arguments,
then invokes the callable export and returns its raw result. It does not infer
language signatures or marshal native values. `instance` is the underlying `WebAssembly.Instance`.
The embedding must know whether an `i32` means a boolean or a descriptor handle.
Use `abi.decode('string', handle)` or `abi.decode('number[]', handle)` explicitly
for memory-backed results. Hosts can use the same helper with an instance getter
to decode incoming handles and encode raw return handles.

The runner's `--entry` defaults to `main`; `--args` defaults to `[]` and accepts a
JSON array of numeric raw arguments. Use numbers for `f64`, `i32` booleans, and
valid descriptor handles, not native strings or nested arrays. Handles must
already refer to valid storage in that instance; the CLI does not allocate
payloads for them. `--imports PROVIDER.cjs` supplies raw callbacks. Successful
void calls print nothing. JSON cannot spell NaN or infinities; programmatic raw
calls can pass them.

## Binary compatibility

CLI binary output and library output are the same raw Wasm bytes for the same
source, host declarations, and compiler/export options. There is no interface
custom section, package metadata, or replacement interface format. The Node
runtime accepts either output; it needs binary Wasm, not WAT. WAT must be assembled
externally before being passed to JavaScript's binary WebAssembly API.

The generated module has no automatic start function and no WASI `_start`.
Invoking exported `main` is an embedding convention. Keep application-known
signatures and provider bindings consistent with the compiled module.

## Diagnostics and failures

Compiler failures go to stderr with an exit status of `1`; usage errors use `2`.
Where a source span exists, diagnostics include the source path, line/character
column, stage, source excerpt, and caret underline. UTF-8 byte spans, CRLF, tabs,
EOF, and multiline statements are handled. Semantic/codegen expression locations
still cover the containing statement, not individual expression nodes. Unlocated
export/layout errors report their stage without inventing line numbers.

The Node runner exits `1` for failures and reports these stages:

| Stage | Example / remedy |
| --- | --- |
| `file` | Missing Wasm/provider path; verify cwd and paths. |
| `compilation` | Invalid binary or WAT supplied as bytes; compile binary Wasm. |
| `instantiation` | Missing/noncallable provider import; supply every named module/field with the declared raw signature. |
| `invocation` | Missing/noncallable entry, invalid runner arguments, host exception, or guest trap; inspect raw inputs and the reported call context. |

A same-thread timer cannot interrupt a synchronous nonterminating guest. Run
untrusted or potentially unbounded work under worker/process or engine resource
controls. Neither traps nor host exceptions roll back prior effects. See
[limitations.md](limitations.md) for current workarounds and future choices.
