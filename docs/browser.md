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
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --offline --no-opt
```

The output directory is relative to the `browser/` crate, so `../pkg` places the
package in the project root. `--out-name was_compiler` preserves the generated
filenames used by the demo's existing imports.

`wasm-pack` uses release mode by default. `--no-opt` skips its external
`wasm-opt` step; this is separate from the compiler's source optimization setting.
`--offline` requires the Rust dependencies to be cached. Installing the target,
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
a zero-argument host function. Invalid registrations, configuration, or source
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

Always release the compiler with `free()` in a `finally` block, including when
compilation fails. `compile()` returns an owned copy, not a view into compiler
memory, so its bytes remain valid after `free()`. The examples' extra `.slice()`
is optional and retained for explicit copy ownership. The generated compiler
wrapper and its Wasm memory are **not** the generated program's runtime or
memory. Do not use the compiler's memory to read program strings/arrays, and do
not pass compiler glue imports to the emitted module.

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
