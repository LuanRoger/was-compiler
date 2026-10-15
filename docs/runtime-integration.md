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
as a result. `range` is recognized but passed to core host registration for
unsupported-ABI rejection, with CLI guidance to use supported host types; it is
not an unknown type or a public range ABI. Empty parentheses mean no parameters.
The declaration's source name
still follows host-registration identifier/reserved-name rules. This compact
notation is not an escaping format for arbitrary import names; the Rust API is
available when module/field names need characters that collide with delimiters.

Compilation finishes before an output file is touched. Files are written to an
exclusively created sibling temporary file, synced, then renamed into place;
failed writes clean up that temporary file. Output resolving to the source's
canonical path is refused. This preserves existing artifacts on compilation errors
and avoids publishing a partially written file. It does not promise directory-sync
crash durability or race-proof source-file identity checks.

## Function-result inference and raw calls

Native CLI and browser compilation accept omitted function results through acyclic
chains and diamonds, including named/default calls:

```text
cargo run --offline -- examples/inferred-defaults.was -o target/inferred-defaults.wasm --export main
node runtime/run.cjs target/inferred-defaults.wasm
```

This prints `584`. Parameters still require types and named functions require
explicit returns. Reachable returns must agree exactly; incomplete nonvoid paths
are errors. No value-return syntax infers void, while Never-only value-return syntax
requires an annotation. Every member of a recursive component needs an explicit
result, including calls in unreachable/skipped source and across annotated barriers.
These are initial inference limitations, not a new runtime or polymorphic ABI.

The concrete result ABI is identical to an explicit annotation: `number` is `f64`,
`bool` is `i32` with normalized host boundaries, strings/arrays are instance-local
`i32` descriptor handles, and void has no result. Range signatures remain private.
Source defaults do not reduce raw export arity. For the example, exporting `total`
requires all three machine arguments; use a source wrapper to apply defaults.
String/array marshaling remains an explicit host operation using the generated
instance's memory, not compiler memory or automatic JSON conversion.

Inference/binding/default errors use the existing diagnostic renderer and original
byte spans. Annotated-source bytes, ordinary body-literal error stages and existing
first-error ordering remain unchanged. See the
[raw-AST migration](named-call-interface.md#public-ast-migration) and
[release validation](was_language_extensions_plan.md#issue-16-wp7-release-validation).

## Function preconditions (WP8)

Source functions may declare ordered `requires` clauses after the optional result
annotation and before the body; see [syntax and the shared pure whitelist](was_language_extensions_spec.md#13-feature-specification-function-preconditionscontracts).
Source invocation order is written explicit arguments, omitted defaults in formal
order, all callee parameter validation, ordered contracts, then body. Validation of
string/array/range parameters precedes even a constant false contract or one that
never reads those parameters. Internal and recursive calls enforce the same checks
as selected raw exports. Raw exports retain full arity: the host supplies every
machine argument; neither source defaults nor contracts add host wrappers.

A false condition executes standard Wasm `unreachable`; JavaScript observes a
`WebAssembly.RuntimeError`, without portable custom messages, contract metadata,
or rollback of prior argument/default effects. Declaration errors use expression-only
UTF-8 spans through the existing renderer. Contracts must be `bool`, read only
formals, and use the narrow default whitelist; all declarations and skipped operands
are checked before inference/body analysis. False contracts do not relax return rules
or change inference. No optimization level removes checks. `requires` is reserved
as a source/host-language name, while arbitrary nonempty host module/field names
remain valid.

WP8 is implemented and [release gates passed](was_language_extensions_plan.md#wp8-release-validation),
including rebuilt-browser and full ignored runtime coverage. See
[WP8 suites and frozen contracts](was_language_extensions_plan.md#wp8--whitelist-contracts-spec-13).
WP9 match statements/values are implemented with [release gates passed](was_language_extensions_plan.md#wp9-release-validation),
including bounded Node execution and current-source rebuilt-browser/full ignored
validation. `tests/match_runtime.rs` covers effects, first-match overlaps, strings,
fractional/NaN cases and transfers; `tests/match_adapters.rs` covers native/browser
compatibility. Node/browser cases retain their default ignored policy. Match uses
the existing raw ABI and explicit marshaling, with no new host callback or helper
kind; its scrutinee runs once and only the first matching arm executes. See
[the frozen match rules](was_language_extensions_spec.md#82-match-expression-syntax).
WP10 named-only pipelines are implemented (#27), with
[release gates passed](was_language_extensions_plan.md#wp10-release-validation)
on 2026-10-14. WP11 implements capture-free local callables (#29) and
immediate-creator shared parameter/let captures (#30), plus captured numeric
compounds (#31). #32 adds nested parameter/let forwarding and live callable contexts;
#33 adds read-only iteration captures, with [validation passed](local-callable-interface.md#issue-33-validation-2026-10-15).
Combined nested/transitive iteration-capture convergence, full WP11 release and WP12 placeholders remain deferred.
[#30 subset validation passed](local-callable-interface.md#issue-30-validation-2026-10-14),
including rebuilt-browser and full ignored runtime coverage before #31.
[#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14). WP13 optimization
requires separate authorization.

## Local callables and shared captures (#29–#33)

The CLI and core accept `let twice = (x: number) => x * 2;` followed by
`twice(21)`. Expression and tail-valued block bodies infer concrete results;
zero-argument and Void helpers work too. #30 permits shared captures of immediate-
creator parameters and ordinary lets of type `number`, `bool`, `string`, `number[]`
or `range`. Reassignment from either side and ordinary indexed array mutation are
supported. #32 forwards original ancestor parameter/let cells and earlier live
callable contexts. #33 permits read-only range/array iteration captures; ordinary
and all numeric compound writes remain rejected even when skipped/unreachable.
Each yielded captured binding receives a fresh eight-byte cell before the body,
even if the closure declaration is skipped; empty/exhausted/failed yields allocate
no cell. Cells are monotonic, never reused/reclaimed; environments allocate only
at executed declarations. Combined nested/transitive iteration-capture convergence
remains deferred. See [the #33 contract](local-callable-interface.md#implemented-read-only-iteration-captures-33). #31 supports all five numeric compounds on captured scalar/indexed
places, including creator-side promoted bindings. The checked target and old value
are retained before RHS effects, even if the RHS rebinds the array or grows memory.
Ordinary indexed assignments also preserve their pre-RHS checked element address.
See [the subset, diagnostics and AST migration](local-callable-interface.md).

Helpers are private direct-call functions after existing runtime helpers, not
exports or host callables. Context-free declarations allocate no environment and
add no hidden argument. Capturing helpers receive a private hidden first `i32`
environment argument. Captured parameter cells allocate in formal order after
validation/contracts, captured let cells after completing initializers, and
nonempty environments only at executed declarations. Structural cell promotion
can allocate even when a later capturing declaration is skipped.

[Private uniform eight-byte cells/slots](memory-abi.md#private-closure-storage-30)
use monotonic allocation: calls can grow memory or trap without rollback of prior
effects or allocation. There is no scope/return/trap reclamation or general heap
rewind; returned or host-retained arrays/strings must remain valid. Refresh
JavaScript views after guest calls or host allocations that may grow memory.
Normal handle validation, explicit marshaling and external range restrictions
remain unchanged. No public callable ABI, table or implicit callback is added.
Full WP11 release remains deferred. See the separate [#30 execution evidence](local-callable-interface.md#issue-30-validation-2026-10-14)
for shared-storage validation before #31, not captured compounds. Historical #29
results cover only capture-free callables.

## Named-only pipelines (WP10; release gates passed)

Native compilation supports user-function stages with positional-prefix/named
arguments and omitted trailing defaults. Input binds formal 0 before written
arguments and defaults; bare and empty stages are equivalent. Evaluation is once
only and ordered, followed by callee parameter validation and ordered contracts.
Host imports and builtin `len` cannot be stages; wrap host effects in a user function.

A final declared-Void stage requires a discarded result: direct expression
statements and propagated discarded block tails/if branches/match arms, with
transparent parentheses. Outer discard does not authorize operands, call arguments,
pipeline inputs or value-producing contexts. Nonfinal declared-Void stages remain
rejected even after Never; expression if/match structural joins are unchanged.
See [normative §12](was_language_extensions_spec.md#12-feature-specification-pipeline-operator-).

There is no pipeline runtime API, callback registration shortcut, typed discard
wrapper or public discard flag. Ordinary binding/defaults, imports, exports and
explicit marshaling retain the raw full-arity ABI; hosts still supply all registered
imports, and raw exported invocations do not synthesize defaults. Source-semantic
discard is not wholly reconstructible from forged typed input; backend injection
ownership, type and flow checks remain independent. See [migration](named-call-interface.md#wp10-pipeline-migration-issue-25).
The plan's #25/#26 validation records remain historical subset evidence, not
full-release evidence. [#27 release validation passed](was_language_extensions_plan.md#wp10-release-validation),
including current-source rebuilt-browser and full ignored runtime coverage.

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

Statement `for name in iterable { body }` accepts `range` or `number[]` with Never
compatibility and needs no new host callback or helper. Ranges use the same private
records; arrays use the existing public descriptors. `for` is reserved for
source/host-language names, not host module/field names. Its iterable evaluates
once in the enclosing context before the loop/binding is active; iterable
break/continue targets the outer nearest loop, and return/divergence is preserved.
The body has a read-only `number` binding (a distinct inner shadowing let remains
mutable), permits bare breaks, and rejects value breaks even when unreachable.
Continue reaches advancement before retesting. For ranges, cached `i64` fields and signed
endpoint checks before exact `f64` yield avoid rounding stalls; yielded zero is
`+0`, and a valid advance past bounds exhausts normally even outside the safe domain.
For is a void statement, not a value expression, and adds no public range ABI.

For arrays, full `ValidateArray` precedes caching the handle and unsigned `u32`
length once, including for an empty array. Unsigned index iteration loads each
element through existing checked `ArrayAddress` then `f64` load on its iteration,
not from an element snapshot or cached raw payload pointer. Alias mutations to
unvisited elements are visible; source reassignment does not replace the captured
handle. Continue exits the body latch before index advancement/retest. ABI/helper
order is unchanged, and array-only iteration needs no range helpers. Issue #5
changes no parser or AST shapes.

CLI and browser compilation support both iterable types through the shared core.
**WP5 implementation is complete and all release gates passed.** See
[extension plan §5](was_language_extensions_plan.md#5-documentation-and-validation-workflow)
for exact commands/results, including the full ignored-test run.
Issue #6 adds adapter/cross-feature coverage, including private host-signature
rejection and export selection across optimization levels. Range host declarations
fail with exit status `1` and supported-type guidance; registration failures leave
existing output files unchanged. Browser validation requires rebuilt bindings.
`tests/array_for_loops.rs` is the array-for suite; cross-feature execution coverage
is detailed in the [WP5 checkpoint](was_language_extensions_plan.md#wp5--safe-ranges-rangearray-for-membership-spec-56).
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

Source named/default calls are compiler-side conveniences only. A declaration
such as `fn f(a: number, b: number = a + 1) -> number` still exports two `f64`
parameters: raw Node/browser callers must supply both. No default metadata or
host-side argument generator is emitted. `runtime.call('f', [3])` rejects the
missing raw argument; `runtime.call('f', [3, 4])` supplies the full signature.
Direct JavaScript Wasm calls do not run source defaults either; JavaScript's
conversion of omitted arguments is not the language's omission semantics.
Use a source wrapper such as `fn main() -> number { return f(3); }` to invoke
an omitted default from the embedding.

Explicit source arguments run once in written order, followed by omitted defaults
in formal order on every invocation. Scalars are captured values; string/array/range
handles are shared, not deep copies. Later explicit array mutations can therefore
be observed by permitted default equality/`len` operations. A transfer or trap in
an explicit argument prevents later defaults and the callee body from executing;
normalization adds no function/loop context. Range import/selected-export
restrictions are unchanged.

Named arguments are user-function-only; host declarations carry no formal-name
or default contract, and builtin `len` remains positional. See the
[default whitelist](was_language_extensions_spec.md#142-default-expression-restrictions-and-scope).
WP6 named/default parameters are implemented, and
[#13 release validation passed](was_language_extensions_plan.md#issue-13-wp6-release-validation),
including CLI execution and freshly rebuilt browser/runtime coverage. WP7 result
inference is also available through the CLI/library/browser adapters; see
[#16 release evidence](was_language_extensions_plan.md#issue-16-wp7-release-validation).

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
