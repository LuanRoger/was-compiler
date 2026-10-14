# Compiling source in the browser

The dedicated `browser/` workspace crate (`was-compiler-browser`) depends on the
root compiler library and builds it as WebAssembly with a `wasm-bindgen`
JavaScript interface. There is no root `browser` feature; the native root crate
remains the default workspace member. The browser can then compile editable
source into a **second, independent WebAssembly module**, instantiate it, and
explicitly call an exported function. Compilation happens locally; no compilation
service, npm packages, or `runtime/*.cjs` files are needed.

## Build and open the demo

Install Rust, the target, and `wasm-pack` if they are not already available:

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --locked
```

From the project root, build the web package:

```text
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline
```

The output directory is relative to the `browser/` crate, so `../pkg` places the
package in the project root. `--out-name was_compiler` preserves the generated
filenames used by the demo's existing imports.

`wasm-pack` uses release mode by default. `--no-opt` skips its external
`wasm-opt` step; this is separate from the compiler's source optimization setting.
`--offline` requires the Rust dependencies to be cached; `--mode no-install`
requires compatible wasm-bindgen tooling already installed. Installing the target,
`wasm-pack`, or any missing matching `wasm-bindgen` tooling may require network
access before the offline build can succeed.

The generated package includes `pkg/was_compiler.js`,
`pkg/was_compiler_bg.wasm`, and TypeScript definitions in `pkg/was_compiler.d.ts`
and `pkg/was_compiler_bg.wasm.d.ts`. Keep these matching files together. Do not copy only
the JavaScript wrapper or rename the Wasm file without adjusting initialization.
The demo worker imports `../../pkg/was_compiler.js`, whose default initializer
loads the compiler Wasm relative to that generated module.

Serve the **project root**, not `examples/browser/`, with your preferred static
HTTP server. For example, if Python is available:

```text
python -m http.server 8000
```

Open <http://localhost:8000/examples/browser/> in a modern browser supporting
WebAssembly and module workers. Do not open `index.html` using a `file:` URL.
This example is dependency-free and uses external module scripts, not a bundler.

The default sample registers `print`, logs `65`, and returns `65`. Edit the
source and select **Compile & run**. The demo exports only `main`, calls it with
no arguments, displays the result or diagnostic as text, optionally displays
plain WAT, and enables a download of raw `program.wasm`. Downloaded output is
not JSON or a serialized runtime artifact; it can be instantiated with standard
`WebAssembly.instantiate` and the required imports.

## JavaScript API

```js
import init, { Compiler } from "./pkg/was_compiler.js";

await init();
const compiler = new Compiler();
let bytes;
try {
    compiler.setExports(["main"]);
    compiler.setOptimizationLevel(0);
    bytes = compiler.compile("fn main() -> number { return 42; }").slice();
} finally {
    compiler.free();
}

const { instance } = await WebAssembly.instantiate(bytes, {});
console.log(instance.exports.main());
```

Adjust the import URL to your module's location. `init()` must finish before
constructing a compiler. The interface is synchronous after initialization:

| Method | Behavior |
| --- | --- |
| `new Compiler()` | Create a compiler with default options and no host registrations. |
| `compile(source)` | Compile source text into raw Wasm as an owned `Uint8Array` copy. |
| `compileWat(source)` | Compile source text and return a plain WAT string. |
| `registerHostFunction(name, module, field, params, result)` | Declare a source-visible host function and its Wasm import signature. |
| `setExports(names)` | Export only the listed user functions; pass an array of strings. |
| `exportAll()` | Export all user functions (the default). |
| `exportNone()` | Export no user functions. Required memory ABI exports remain. |
| `setOptimizationLevel(level)` | Select an integer from `0` through `3` (O0–O3); default is `0`. |
| `free()` | Release the compiler object. Do not use it afterward. |

Host parameter/result types are spelled `number`, `bool`, `string`, and
`number[]`; `void` is allowed only as a result. An empty parameter array declares
a zero-argument host function. A host parameter/result spelled `range` throws an
actionable host-registration `Error`: ranges are private to guest functions; use
supported host types instead. Registration uses core validation and preserves its
first-error ordering for names, import fields, and types. Failed registration leaves
the registry unchanged.
Invalid registrations, configuration, or source
compilation throw JavaScript `Error` objects. Source diagnostics use the shared
compiler library diagnostic module and include the synthetic filename `<browser>`
and located source excerpts where applicable.
Catch errors and display `error.message` with `textContent`, not `innerHTML`.
Instantiation/link errors and program traps are separate from compiler errors.
The demo assumes `main() -> number`: it displays a raw scalar result, not a
language-type-aware decoded value. Returning a string or array would display its
descriptor handle; use the memory ABI explicitly in your application.

Export selections refer to user functions, not host imports. Unknown or duplicate
selected names are rejected during compilation. `exportNone()` is useful for
embedding configurations, but the demo requires an exported `main`. All levels
currently produce identical output because optimization passes are not yet
implemented. `compileWat` runs the compilation pipeline again; asking for both
bytes and WAT therefore compiles twice.

The browser adapter uses the same compilation pipeline as the native library,
including `number % number` with JavaScript `%`/`fmod` truncating remainder semantics.
Operands execute exactly once, left-to-right, and the dividend's sign is preserved,
including signed zero. A NaN operand, infinite dividend, or zero divisor yields
NaN; a finite dividend with an infinite divisor is returned unchanged. For a
finite dividend and finite nonzero divisor, the remainder's magnitude is less
than the divisor's.
The emitted guest module uses an optional `(f64, f64) -> f64` helper, not a
JavaScript callback: `%` needs no host registration, import, or guest memory.
The helper follows user functions and any seven memory helpers only when `%`
occurs anywhere in the typed AST, even in unexported, unreachable, or
runtime-skipped code. Output without `%` is unchanged. See
[M9](compilation-roadmap.md#m9--floating-point-remainder--implemented) for the exact
scaled-subtraction algorithm and bounds; this feature adds no optimization pass.

The same pipeline also supports exact `number in range` membership returning
`bool`, at comparison precedence with existing left associativity. Both operands
evaluate once, left-to-right, with exact types and Never compatibility; all
children remain checked. `in` is reserved for source/host-language names, not
host module/field names. Membership validates the consumed record before the
candidate guard, so malformed records trap even for invalid candidates.
Nonfinite, fractional, or out-of-safe-domain candidates return false; both signed
zeros are accepted. Signed `i64` interval/congruence checks need no JavaScript
callback or floating remainder helper. Optional `Contains` follows `Construct`
and `Validate` only when membership exists, preserving construction-only bytes.
Range handles remain private, with no host range type or marshaling API. Use
`setExports(["main"])` for programs with internal range-signature helpers; selected
range user exports and all range host imports are rejected. Export failures retain
the shared diagnostic and suggest `setExports(["main"])` or `exportNone()` using
JavaScript API names. Exported scalar membership results use raw `i32` `0`/`1`.

The core pipeline also supports statement `for name in iterable { body }` over
`range` or `number[]` with Never compatibility, without a new JavaScript API, host
callback, or helper. `for` is reserved for source/host-language names, not host
module/field names. The iterable evaluates once in the enclosing context before
activating the new loop/binding; iterable break/continue targets the outer nearest
loop, and return/divergence is preserved. The body's `number` binding is read-only;
a distinct inner shadowing let remains mutable. Bare breaks are permitted, value
breaks are rejected even when unreachable, and continue advances before retesting.
For is a void statement, not a value expression. For ranges, cached `i64` fields and signed
endpoint checks precede exact `f64` conversion/yield (zero is +0); advancement
outside the safe domain exhausts normally before conversion, without a trap.

For arrays, full `ValidateArray` precedes caching the handle and unsigned `u32`
length once, even when empty. Unsigned index iteration loads each current element
through existing checked `ArrayAddress` then `f64` load, without an element snapshot
or cached raw payload pointer. Alias mutations remain visible; source reassignment
does not replace the captured handle. Continue exits the body latch before index
advancement/retest. ABI/helper order is unchanged, and array-only iteration needs
no range helpers. Issue #5 changes no parser or AST shapes.

The browser adapter uses this shared core for both iterable types. **WP5
implementation is complete and all release gates passed.** See
[extension plan §5](was_language_extensions_plan.md#5-documentation-and-validation-workflow)
for exact evidence: rebuilt offline/no-install bindings, 22 browser groups, and
the full ignored-test run passed. Issue #6
adds `tests/browser.cjs` coverage for private signatures, export selection,
byte/WAT and diagnostic parity across optimization levels, and interactions with
value regions, aliases, compounds, abrupt flow, and trap/effect order.
`tests/array_for_loops.rs` is the array-for suite; shared-core interaction coverage
is detailed in the [WP5 checkpoint](was_language_extensions_plan.md#wp5--safe-ranges-rangearray-for-membership-spec-56).
`tests/for_frontend.rs` and `tests/for_lowering.rs`
cover frontend and checked-lowering contracts; `tests/for_loops.rs` covers layout,
optimization parity and bounded Node execution, including exact yields/exhaustion,
effects, transfers, consumed-record validation and caching. Its Node cases are
ignored by default; default Cargo success alone does not establish runtime coverage.
See the [private record contract](memory-abi.md#private-range-record). Rebuild `pkg/`
after core/browser changes before adapter validation; stale bindings do not validate
current source.

Always release the compiler with `free()` in a `finally` block, including when
compilation fails. `compile()` returns an owned copy, not a view into compiler
memory, so its bytes remain valid after `free()`. The examples' extra `.slice()`
is optional and retained for explicit copy ownership. The generated compiler
wrapper and its Wasm memory are **not** the generated program's runtime or
memory. Do not use the compiler's memory to read program strings/arrays, and do
not pass compiler glue imports to the emitted module.

## Named arguments and defaults (WP6)

`compile` and `compileWat` accept the same user-function named/mixed-prefix calls
and trailing pure defaults as the native core; no browser-only binding API is added:

```text
fn total(base: number, extra: number = base + 1) -> number {
    return base + extra;
}
fn main() -> number { return total(base: 3); }
```

Compile and instantiate this source with the JavaScript API above. `main()`
returns `7`, but the raw export `total` still has two machine parameters:
`instance.exports.total(3, 9)` returns `12`. Calling `total(3)` directly does not
run its source default. Supply the full raw signature or expose a source wrapper
like `main`; there is no emitted default metadata or host argument generator.

Explicit source arguments are captured once in written order, then omitted defaults
run in formal order per invocation. Handles remain shared, so defaults may observe
later explicit array mutations or trap during descriptor validation. The
[default whitelist](was_language_extensions_spec.md#142-default-expression-restrictions-and-scope)
allows only earlier-formal reads, scalar literals, existing type-valid operators,
and positional builtin `len(array)`; it is checked even in skipped operands and
unused declarations. Named host/builtin arguments remain unsupported. Default
errors retain declaration origins; normalized explicit arguments retain call-site
origins in the shared `<browser>` diagnostics. Private range export/import
restrictions are unchanged, including when a default copies an earlier range.

WP6 named/default parameters are implemented, and
[#13 release validation passed](was_language_extensions_plan.md#issue-13-wp6-release-validation),
including current-source rebuilt bindings, the browser harness, and the full
ignored suite. Rebuild `pkg/` after core/browser changes before validating browser
behavior; the [handoff](wp6-wp7-development.md#validation) records earlier historical
worktree-development evidence.

## Function-result inference (WP7)

`compile` and `compileWat` infer omitted results through forward acyclic chains
and diamonds, including named/default calls, using the same core as the native CLI:

```text
fn main() { return total(base: 3); }
fn total(base: number, extra: number = base + 1) { return base + extra; }
```

The generated `main()` returns `7`. The raw `total` export still takes two `f64`
arguments and returns `f64`; `total(3, 9)` returns `12`. Source omission runs
`extra`'s default, but raw JavaScript omission does not. Inferred bool results use
`i32` with normalized host boundaries, and inferred string/array results remain
instance-local descriptor handles in the generated program's memory. No metadata,
marshaling wrappers or execution engine are added. Inferred range signatures remain
internal: use `setExports(["main"])` or `exportNone()` rather than exporting them.

Parameters still require annotations. Reachable explicit returns must agree
exactly; nonvoid functions cannot fall through. There are no implicit named-function
tails. Without value-return syntax, a procedure infers void; a void procedure calls
another procedure with `noop(); return;`, not `return noop();`. Never-only
value-return syntax requires an annotation. Every member of a recursive component
requires an explicit result, even with an anchored return or an annotated barrier;
unreachable and short-circuit-skipped calls still count. All such source is checked.

Inference, binding and default failures use the shared `<browser>` diagnostics,
retaining UTF-8 byte spans and declaration/call origins. Fully annotated source
keeps existing literal stages and first-error ordering. See the
[public raw-AST migration](named-call-interface.md#public-ast-migration) and
[#16 release evidence](was_language_extensions_plan.md#issue-16-wp7-release-validation).
WP9 match statements/values are implemented through the shared core with
[release gates passed](was_language_extensions_plan.md#wp9-release-validation),
including 25 current-source rebuilt-browser cases and full ignored runtime
validation. `tests/match_adapters.rs` covers CLI/browser byte and diagnostic parity;
`tests/browser.cjs` includes the WP9 browser case. Match does not change raw result
ABIs or explicit descriptor marshaling. Runtime/browser tests retain their default
ignored policy; rebuild bindings after core changes before browser validation.
See [the frozen match rules](was_language_extensions_spec.md#82-match-expression-syntax).
WP10 named-only pipelines are implemented (#27), with
[release gates passed](was_language_extensions_plan.md#wp10-release-validation)
on 2026-10-14. WP11 implements capture-free local callables (#29) and
immediate-creator shared parameter/let captures (#30). Broader captures/full WP11
release and WP12 placeholders remain deferred. WP13 optimization requires separate
authorization. [#30 validation passed](local-callable-interface.md#issue-30-validation-2026-10-14),
including current-source rebuilt-browser coverage. Rebuild `pkg/` after future
core changes before browser validation.

The shared core accepts `let twice = (x: number) => x * 2;` and direct positional
lexical calls, plus shared captures of immediate-creator parameters/ordinary lets
of type `number`, `bool`, `string`, `number[]` or `range`. Captured reassignment and
ordinary indexed array mutation preserve original bindings and alias semantics.
Nested helpers may capture their immediate creator's own bindings; ancestor
forwarding, context-requiring callable dependencies, captured compound places
(including creator-side promoted bindings) and captured `for` bindings remain deferred.
See [the subset and public `Captured` AST migration](local-callable-interface.md).

Helpers are private, with no callable exports or JavaScript callback ABI.
Context-free declarations add no environment, hidden argument or callable-only
memory. Capturing helpers use a private hidden first `i32` environment pointer;
captured parameter cells allocate in formal order after validation/contracts,
captured let cells after completing initialization, and nonempty environments
only when their declarations execute. Cells/slots are uniformly eight-byte aligned
and sized; see [the private storage contract](memory-abi.md#private-closure-storage-30).

Allocation is monotonic, can grow guest memory or trap, and does not roll back
prior effects or reclaim storage on return/trap. Even skipped capturing declarations
can have eagerly promoted cells. Refresh JavaScript views of guest memory after
calls or host allocations that may grow it. This guest memory is distinct from the
browser compiler's own Wasm instance memory. Existing explicit string/array
marshaling, descriptor obligations and external range restrictions are unchanged.
Use the separate [#30 browser/runtime evidence](local-callable-interface.md#issue-30-validation-2026-10-14),
not historical #29 results; full WP11 release remains deferred.

## Named-only pipelines (WP10; release gates passed)

The shared core supports named user-function stages with positional-prefix/named
arguments and omitted trailing defaults. Input occupies formal 0 before written
arguments/defaults; bare and empty stages are equivalent. Evaluation remains once
only and ordered, with callee parameter validation and ordered contracts. Host
imports and builtin `len` are ineligible stages; use user-function wrappers for
host effects. No new JavaScript API, implicit callback or marshaling is introduced.
Ordinary calls/defaults and raw full-arity exports remain unchanged; JavaScript
invocations still supply all machine arguments and do not synthesize defaults.

Final declared-Void stages require discarded results: direct expression statements
and propagated discarded block tails/if branches/match arms, with transparent
parentheses. Outer discard does not authorize operands, call arguments, pipeline
inputs or value-producing contexts. Nonfinal declared Void remains rejected even
after Never; structural joins for expression if/match are unchanged. Shared
`InvalidPipeline` diagnostics use stage spans and the messages
`a nonfinal stage cannot return void` and
`a final void stage requires a discarded result`.

There is no typed wrapper or public discard flag. Source-semantic discard is not
wholly reconstructible from forged typed input; backend injection ownership, type
and flow checks remain independent. See [normative §12](was_language_extensions_spec.md#12-feature-specification-pipeline-operator-)
and [migration](named-call-interface.md#wp10-pipeline-migration-issue-25).
WP10 is implemented, and [#27 release validation passed](was_language_extensions_plan.md#wp10-release-validation),
including a current-source browser rebuild, 25 browser smoke cases and the full
ignored runtime suite. The plan's #25/#26 records remain historical subset evidence,
not full-release evidence. Future browser validation still requires freshly rebuilt
bindings; runtime/browser cases remain ignored by default. Existing generated `pkg/`
output alone is not current-source evidence.

## Function preconditions (WP8)

`compile` and `compileWat` accept named-function `requires` clauses through the
shared core, with the same declaration checks and expression-only UTF-8 spans as
the native adapter:

```text
fn divide(a: number, b: number = 1) requires b != 0 { return a / b; }
fn main() { return divide(a: 12, b: 3); }
```

After compilation/instantiation, `main()` returns `4`; the full-arity raw export
`divide(12, 3)` also returns `4`, and `divide(12, 0)` throws a standard
`WebAssembly.RuntimeError`. Raw JavaScript calls do not apply source defaults.
Source calls evaluate explicit arguments, then omitted defaults; the callee validates
all parameters before checking contracts in declaration order, then runs its body.
Internal and recursive calls enforce the same checks. No custom contract metadata,
portable error message, check elimination, or rollback is added.

Conditions require exact `bool` and share the [narrow default whitelist](was_language_extensions_spec.md#142-default-expression-restrictions-and-scope),
with all formals readable. All declarations and skipped operands are checked before
inference/body analysis; false conditions do not excuse missing returns or change
inference. `requires` is reserved for source/host-language names, not host module/field
names. Existing bool normalization, descriptor marshaling and private range ABI
restrictions are unchanged.

WP8 is implemented and [release gates passed](was_language_extensions_plan.md#wp8-release-validation),
including freshly rebuilt bindings, browser parity and the full ignored runtime suite.
See [actual suites and frozen contracts](was_language_extensions_plan.md#wp8--whitelist-contracts-spec-13).
Rebuild bindings after any further core change before browser validation.

## Host function example

Registration only declares an import; it does not install a JavaScript callback.
Supply the corresponding callback when instantiating the emitted module:

```js
const compiler = new Compiler();
let bytes;
try {
    compiler.registerHostFunction("print", "env", "log_number", ["number"], "void");
    compiler.setExports(["main"]);
    bytes = compiler.compile(`
        fn main() -> number {
            print(6 * 7);
            return 42;
        }
    `).slice();
} finally {
    compiler.free();
}

const logs = [];
const { instance } = await WebAssembly.instantiate(bytes, {
    env: {
        log_number(value) {
            logs.push(value);
        },
    },
});
const result = instance.exports.main();
```

This snippet assumes `init()` has completed and `Compiler` has been imported as
above. `number` crosses the boundary as `f64`; booleans use canonical `i32`
`0`/`1`. Strings and numeric arrays cross as `i32` descriptor handles, not native
JavaScript strings or arrays. Such hosts must use the **emitted instance's**
`memory` and `__alloc` and follow the [memory ABI](memory-abi.md). The numeric
host in this demo needs neither allocator nor memory access. Every registered
import must be provided, even if the source never calls it. Uncheck the numeric
host option only after removing source calls to `print`.

## Worker lifecycle and limits

`examples/browser/main.js` creates a new module worker for each run.
`worker.js` initializes the compiler, configures it, compiles source (and optional
WAT), copies the bytes, and frees the compiler **before** instantiating the
program. It posts a transferable copy of the artifact to the page, then supplies
`env.log_number`, instantiates the original bytes, and explicitly invokes
`instance.exports.main()`. It retains at most 1,000 log entries.

The page enforces a 15-second loading/compilation deadline and starts a separate
3-second deadline when it receives the compiled artifact. That second deadline
covers instantiation and execution. **Stop**, a deadline, a result, or an error
terminates the worker. A later run starts fresh. These are wall-clock limits,
not instruction budgets, and background-tab timer throttling may delay them.
Since compiler calls and Wasm execution are synchronous, a timer inside the
worker cannot interrupt an infinite loop; the page must call `worker.terminate()`.
Keeping both operations in the worker prevents them from blocking the UI.

Artifacts arrive before execution, so a trap or execution timeout still leaves
Wasm/WAT available. Downloads always correspond to that compiled source
snapshot, not later edits. Logs are delivered when execution finishes or throws;
logs from a terminated run are not delivered. A compilation timeout leaves no
artifact. Disabling WAT avoids the second compilation and its output cost.

A worker and a timeout are **not a security sandbox** or a memory quota. Generated
Wasm has access to the JavaScript imports you explicitly supply; those callbacks
have the worker's JavaScript privileges. Keep imports narrow and trusted. Do not
supply network, storage, or other privileged callbacks to untrusted source
without your own policy. Very large sources, output, or allocations can still
exhaust memory. The demo intentionally provides only a bounded numeric logger.

## Serving, MIME types, and CSP

- Serve JavaScript modules with a JavaScript MIME type such as
  `text/javascript`, and `.wasm` with `application/wasm` for streaming
  initialization. A server that substitutes an HTML fallback for a missing
  `pkg/` file will cause a module or Wasm loading failure.
- Keep the page, module worker, generated JavaScript, and compiler Wasm on the
  same origin for the simplest setup. Cross-origin deployments require the
  appropriate CORS headers. Shared memory and cross-origin isolation are not
  needed for this demo.
- A Content Security Policy must allow the module scripts and worker (normally
  `script-src 'self'` and `worker-src 'self'`), Wasm compilation (normally
  `'wasm-unsafe-eval'` in `script-src`), and fetching the compiler Wasm
  (`connect-src 'self'`). The demo's inline stylesheet also needs an allowed
  style hash/nonce or `style-src 'self' 'unsafe-inline'`. Browser CSP support
  varies; consult the console rather than disabling your application's policy.
- Browsers execute raw Wasm, not Wasmtime `cwasm` artifacts. There is no `cwasm`
  compilation, loading, or execution support in this interface or demo.

## Smoke validation

After building the web package, run either smoke-test entry point from the
project root. Both require Node.js 24 or newer with ES module support and the
built `pkg/` package; the Cargo entry point also requires Rust:

```text
node tests/browser.cjs pkg
cargo test --offline --test browser -- --ignored
```

These tests load the actual compiler Wasm through the generated JavaScript
interface. They do not replace checking the playground in a browser for worker,
HTTP, MIME, and CSP behavior.
