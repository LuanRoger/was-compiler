# Limitations and alternatives

This document separates existing restrictions, workarounds available today, and
possible future designs. Future choices are **not implemented features or delivery
promises**. This document does not propose code optimization.

## Status and scope

M0–M9, including CLI/runtime integration, numeric-array equality, and floating-point
remainder, are implemented. Current behavior below is grounded in
the frontend/backend, [CLI](../src/cli.rs), [diagnostics](../src/diagnostic.rs),
[export policy](../src/exports.rs), [Node runtime](../runtime/index.cjs), and their
unit/integration tests. See
[Evidence and validation boundaries](#evidence-and-validation-boundaries).

The compiler produces WebAssembly bytes; it is not itself an execution engine.
The [memory ABI](memory-abi.md) is the authoritative descriptor/ownership contract.
The [runtime integration guide](runtime-integration.md) covers CLI and embedding
usage. The [roadmap](compilation-roadmap.md) describes milestone scope and
implemented language extensions.

## Source language and frontend

Evidence: [lexer.rs](../src/lexer.rs), [parser.rs](../src/parser.rs),
[ast.rs](../src/ast.rs), and [semantic.rs](../src/semantic.rs).

Comments are supported between tokens anywhere whitespace can appear: `//` ends
at LF, CR, or EOF; `/* ... */` can span lines but ends at the first `*/` and does
not nest. Delimiters inside strings remain text. Comments separate tokens, never
join identifier fragments or operator bytes, and are skipped by the lexer without
source preprocessing so original byte spans are preserved. An unterminated block
produces an `Invalid` token spanning the opening slash through EOF, then `Eof`;
existing parser errors report it, not a new diagnostic stage.

Local `let` annotations are optional: `let name = initializer;` infers the
initializer's concrete nonvoid type (`number`, `bool`, `string`, or `number[]`),
including `number[]` for `[]`. `let name: type = initializer;` remains supported
with exact type checking. The initializer is analyzed before binding the name,
so a same-named reference requires an outer binding; nested shadowing is allowed.
The binding's type is then fixed. An inferred void call initializer is rejected
with the existing `SemanticError::InvalidVoidType` for the local.

| Restriction now | Practical effect | Current alternative | Possible future choice and tradeoff |
| --- | --- | --- | --- |
| Only `number`, `bool`, `string`, `number[]`, and `void` exist. Parameters and return types require annotations; every `let` requires an initializer. No function inference, whole-program type unification, promotions, implicit coercions, or source `i32` type. | Local inference does not change the `f64` numeric model or allow later assignments of another type. A boolean cannot stand in for a number, and a numeric value cannot serve as a condition; `void` is not a variable/parameter type. | Use inferred or explicitly annotated locals and explicit helper functions, branches, or registered host conversions with matching signatures. | Explicit conversions or broader inference require separate type-system and error-ordering decisions; neither is implied by local initializer inference. |
| No source modules/import declarations, structs, generics, first-class functions, or closures. Calls resolve named user functions, registered hosts, or the built-in `len`. | The source program is one collection of functions; host registration is an embedding/CLI responsibility. | Put related functions in one source unit and supply external capabilities through host functions. Numeric-array conventions can model simple records, but have no field/type safety. | Modules or dedicated aggregate/function types need name-resolution, type-system, and ABI decisions. Host-side modeling avoids those changes but keeps application-specific conventions outside the language. |
| Control flow is `if`/`else`, `while`, and `return`; no `break`, `continue`, `for`, or `foreach`. Every nonvoid function must return on all statically recognized paths. A loop alone is not a guaranteed return, even `while true`. | Some familiar loop forms and intentionally non-returning nonvoid functions are rejected. | Use a numeric index and `len(array)`, boolean loop flags, nested conditions, and an explicit fallback return. Use `void` where no result is intended. | New loop forms or a non-returning type require semantic/control-flow rules and tests. They should not be introduced as behavior-preserving cleanup. |
| Array indexing in source starts from an identifier; there is no general postfix chaining. | `make_values()[0]` and `[1, 2][0]` are not supported source forms. | Assign the result/literal to an inferred or annotated `number[]` local, then index that local. | General postfix parsing is a grammar feature and needs explicit assignment-target rules. |
| Identifiers are ASCII letters/underscore followed by ASCII letters/digits/underscore. | Unicode identifiers are unsupported. | Use ASCII identifiers. | Define a Unicode identifier policy with normalization/confusable-name decisions. |
| Numeric literals contain digits, optionally a dot with digits on both sides. No exponent notation, `.5`, or `1.`. A leading minus is an operator. | `1e3`, `.5`, and `1.` cannot be used as number literals. | Write `1000`, `0.5`, and `1.0`; use unary `-` for negative values. | Broader literal syntax needs matching lexer and backend validation so forged typed literals do not bypass the grammar. |
| Source parsing requires at least one function, although empty function bodies are valid. | An empty source file is not an empty module. The lower-level typed-AST API can represent an empty program. | Use `fn main() -> void {}` for a minimal source program. | Allowing empty source is a deliberate parser/API behavior change, not a test-harness refactor. |

## Numbers, strings, arrays, and failures

Evidence: [expression.rs](../src/codegen/expression.rs),
[values.rs](../src/codegen/values.rs), [memory.rs](../src/codegen/memory.rs),
[numeric.rs](../src/codegen/numeric.rs), [remainder tests](../tests/remainder.rs),
and [memory tests](../src/codegen/memory_tests.rs).

| Restriction now | Practical effect | Current alternative | Possible future choice and tradeoff |
| --- | --- | --- | --- |
| All numbers and numeric-array elements use IEEE-754 `f64`. | Decimal fractions can round; integers outside the safe range `-(2^53 - 1)` through `2^53 - 1` are not reliably distinguishable. Finite literals can underflow to zero; overflowing literals are rejected. Arithmetic/host values can still produce NaN, infinities, and signed zero. Division by zero is floating-point behavior, not an integer-style exception. | Use tolerance-based comparisons where appropriate and validate application ranges at boundaries. Keep exact large integers/decimals in the host, e.g. as strings with host operations; merely using a JavaScript number does not add precision. | Separate integer/decimal types or a host-backed exact-number representation would require overflow, conversion, representation, and ABI policies. |
| `number % number` implements truncating floating remainder (JavaScript `%`/`fmod`), not mathematical modulo or nearest-quotient IEEE remainder. WebAssembly has no `f64.rem`; an optional guest helper implements it. | Operands execute once, left-to-right. For finite operands and a nonzero divisor, the result has the dividend's sign, including signed zero, and magnitude below the divisor's magnitude. A NaN operand, infinite dividend, or zero divisor yields NaN; a finite dividend with an infinite divisor is unchanged. | Use source `%` directly with no implicit host import or memory runtime. An explicitly registered synchronous host function is still an option for a different numeric contract, not a required workaround. | Mathematical modulo or nearest-quotient remainder would need separately specified semantics; they must not silently replace `%`. |
| Strings are immutable UTF-8 values with literals, parameter/local/return use, and byte-content `==`/`!=`. No escape processing, concatenation, indexing, string `len`, normalization, or case folding. | Backslash sequences such as `\n` remain literal characters; a quote ends the literal. Visually identical differently normalized Unicode strings can compare unequal. | Use actual Unicode characters where possible and host functions for quoted/control-character construction, concatenation, text length, and normalization. Construct host results according to the memory ABI. | Add selected operations only after defining byte, code-point, or grapheme semantics. Escape parsing is a lexical feature; normalization changes equality expectations and may require additional runtime support. |
| Arrays contain only numbers, have fixed lengths, and are mutable references. No resize/push operations. Assignment/arguments/returns copy handles, not elements; `==`/`!=` compare contents. | Two guest variables can alias the same storage. Every evaluated array literal allocates new storage, even in loops and recursive calls. | Pass an existing array explicitly when sharing is intended. For an independent or resized result, a trusted host can create a new descriptor/payload. | Dynamic arrays require a capacity/growth/aliasing policy; generalized element types require new layouts and type rules. Neither is implied by the current descriptor ABI. |
| Numeric-array equality requires equal lengths and exact elementwise `f64` equality, not identity, byte equality, or a tolerance. `!=` is its complement. | Signed zeros compare equal; any compared NaN makes equality false, even for an array compared to itself. Comparisons read current contents after both operands are evaluated once, left-to-right, so RHS mutations are visible. Matching-length comparison takes O(n) time and O(1) auxiliary space; both descriptors are validated before early exits. | Use `==`/`!=` for exact content comparisons. Implement an explicit loop or host function for tolerance-based comparisons and a chosen NaN policy. | Approximate equality or other element types require explicit semantics. A pointer shortcut is invalid with NaN and must not be introduced as a behavior-preserving refactor. |
| Array indices must be finite, integral, nonnegative, and less than `len(array)`. Negative zero selects element zero. | Fractions, NaN, infinities, negative values, and out-of-range accesses trap instead of being rounded/truncated. Indexed assignment checks its target/bounds before evaluating the RHS. | Produce indices with bounded integral counters. Use explicit checks or host validation for externally supplied indices; do not rely on an implicit cast. | A checked-result API or language exceptions could make failures recoverable, but would change signatures/control flow and need a structured error contract. |
| Allocation is a growable, eight-byte-aligned bump allocator. No free, garbage collection, or reference counting. Values live until the instance is discarded. | Repeated allocation consumes memory even when values are no longer reachable. Memory grows in 65,536-byte pages to a hard maximum of 65,535 pages, subject to tighter engine limits; overflow or failed growth traps. | Bound work/allocation in the embedding and discard/recreate an instance between independent jobs. Handles cannot survive that reset or cross instances. Do not manually reset the heap while live handles may exist. | Explicit arenas with a lifetime contract, ownership/freeing, or tracing GC offer different application models. Each needs alias/liveness rules; choosing one is runtime feature work. |
| No compiler-configurable memory quota, execution fuel, or recursion limit. Guest loops and recursive calls execute synchronously. | Nontermination and stack/resource exhaustion depend on the engine and embedding. A same-thread JavaScript timer cannot interrupt a running synchronous Wasm call. | Use an engine with resource controls or run work in a terminable worker/process. Bound normal input sizes and recursion explicitly; limits in test harnesses are not guest execution controls. | Add runtime quotas or a cancellation/fuel contract if required. Instrumented checks change generated behavior/output and must be scoped as a separate feature. |
| Bounds/allocation checks use traps, not a structured guest error status. | JavaScript observes runtime exceptions; a failure is not a typed return value, and prior host effects or mutations are not rolled back. | Catch failures at the embedding boundary, validate inputs before calls, and define application-level status returns where useful. Do not assume retries on the same instance are transactional. | Structured results or exceptions need explicit propagation and partial-effect policies. |

### Floating remainder is not a naive arithmetic rewrite

Source `%` now implements truncating remainder directly. The optional guest
`(f64, f64) -> f64` helper is appended after users and any seven memory helpers
only when `%` occurs anywhere in the typed AST, including unexported bodies,
unreachable statements, and runtime-skipped operands. It adds no implicit host
import and requires no Memory/Global/Data sections. Output without `%` is unchanged.

After special-value handling, the helper doubles the absolute divisor until the
next double would exceed the absolute dividend, then halves back to the original
divisor while conditionally subtracting each scale. Scaling is exact relative to
the original divisor, including subnormals; each subtraction is exact by Sterbenz's
lemma because `scaled <= r < 2 * scaled`. It uses constant auxiliary space with at
most 2,097 doublings and 2,098 reduction iterations, forms no overflowing quotient,
and performs no integer casts. This is arithmetic lowering, not an optimization.

Do **not** substitute `a - trunc(a / b) * b` as a generally correct floating
remainder implementation. Division overflow, intermediate rounding, special
values, and the sign of a zero result can all make that formula wrong. A registered
host function remains an explicit option for another contract, such as
nearest-quotient IEEE remainder; document and test its semantics rather than
treating it as interchangeable with source `%`.

## Host trust and the raw ABI

Evidence: [host.rs](../src/host.rs), [abi.rs](../src/codegen/abi.rs),
[call.rs](../src/codegen/call.rs), and [memory-abi.md](memory-abi.md).

- Registration declares a source name, import module/field, and language signature.
  It does **not** install an executable callback. Every registered import must be
  supplied at instantiation, even if the source never calls it.
- There is no built-in I/O, filesystem/network access, or WASI binding. An import
  named `print` is an application-provided capability, not a compiler intrinsic.
  Grant host capabilities deliberately; providers run with their embedding's
  privileges.
- `number` crosses the raw boundary as `f64`; `bool` as `i32` (host-call boundaries
  normalize nonzero values to `1`); `string` and `number[]` as distinct language
  types represented by `i32` descriptor handles. Raw Wasm signatures cannot
  distinguish these `i32` meanings. Descriptors carry no runtime type tag.
- Handles are instance-local. Hosts must keep published descriptor metadata
  unchanged, preserve valid UTF-8 and immutable string bytes, and use the
  instance's exported `__alloc`/`memory` to construct results. Trusted raw-ABI
  hosts may mutate array elements. Recreate memory views after possible growth.
- Guest checks validate descriptor alignment/ranges and array alignment, not
  UTF-8, ownership, string immutability, or the host's honesty. Exported memory
  gives the host direct access; bounds checks do not isolate a malicious host.
- Registration uniqueness is by source name, not `(module, field)`. Different
  source names can target the same import property, even with different language
  signatures. One JavaScript import property supplies one raw callback for all
  those declarations. The runtime does not infer language types or reject aliases
  based on language signatures. Use distinct module/field names for incompatible
  declarations, or deliberately design a callback satisfying every raw contract.
- Host calls are synchronous. A Promise is not a valid substitute for a declared
  number/bool/descriptor result, and a `void` call does not await asynchronous
  work. Complete async work before invoking Wasm, or design an explicit staged
  application protocol. Future async integration would require a separate ABI.
- The Rust library compiles/analyzes programs; it does not instantiate Wasm or
  execute native callbacks. Use an external Wasm engine or JavaScript's
  `WebAssembly` API. Adding a native execution engine is a separate embedding
  choice with its own dependency and resource-policy costs.

## CLI and raw runtime boundary

Shipped behavior is implemented in [cli.rs](../src/cli.rs),
[diagnostic.rs](../src/diagnostic.rs),
[codegen/mod.rs](../src/codegen/mod.rs), [runtime/index.cjs](../runtime/index.cjs),
[runtime/abi.cjs](../runtime/abi.cjs), and [runtime/run.cjs](../runtime/run.cjs).
[main.rs](../src/main.rs) is a thin entrypoint.
[CLI tests](../tests/cli.rs) and [runtime tests](../tests/runtime.rs) cover the
compiler-to-runner path, failures, raw calls, and explicit ABI conversion.

| M7 surface | Contract and limitation | Alternative / follow-up |
| --- | --- | --- |
| CLI input/output | One required UTF-8 source-file path (`PathBuf`); no built-in example or stdin support. `-` is a literal source filename. `-o`/`--output PATH` selects a file, and `-o -` selects text stdout. | Pass a source file, such as `examples/array-sum.was`. Use `--` before a dash-prefixed source path, or write it as `./PATH`. Multiple source files are not supported. |
| Format selection | `--emit wasm\|wat` overrides inference. Otherwise only the exact lowercase output extension `.wasm` selects binary; all other extensions, including `.WASM`, select WAT. Binary requires a filesystem output path, never stdout. | Use `--emit wasm -o program.wasm` for runner input. CLI and library binary output are the same raw bytes for matching source and options; externally assembled WAT can also be instantiated. |
| Filesystem output | After successful compilation/serialization, the CLI creates a sibling temporary file with `create_new`, writes and syncs it, then renames it to the destination. It refuses an existing output that canonicalizes to the input path and does not create parent directories. | Create the destination directory first and choose a distinct output path. Tests cover replacement, unchanged outputs after compilation failure, canonical same-input spellings, and temporary cleanup. This is not a guarantee of power-loss durability or race-proof file identity. |
| User-function exports | Repeated `--export NAME` or `--no-exports`; the default is all user functions. Public `ExportOptions::All`, `None`, and `Only(Vec<String>)` are separate from the unchanged, `Copy` `CompilerOptions`. Unknown/repeated selected names are rejected; emitted exports retain source order. | Treat selection as an API visibility policy only: it removes no function body or registered import and does not change function indices. Memory-backed modules still export `memory` and `__alloc`, including with no user-function exports. It is not a memory/privacy boundary. |
| Host declarations | Repeated `--host 'print=env.log_number(number)->void'` declares source/import names and types using compact delimiter-based syntax. No default I/O callbacks or WASI are installed. | Supply callbacks through the provider/embedding and quote declarations in the shell. For import names not representable by the compact syntax, use the Rust registration API and direct instantiation. |
| CLI exit status | No arguments or invalid arguments exit `2`; help exits `0`. Host-registration, parse, semantic, code-generation, input/output, and serialization errors exit `1`; success exits `0`. Diagnostics go to stderr. Unknown or duplicate selected exports are code-generation errors, not usage errors. | Check the status before running or distributing an artifact. An existing destination remains unchanged on compilation failure; a stale artifact is not evidence that the current command succeeded. |
| Level flags | `-O0` through `-O3` select the existing level configuration. All current levels have identical lowering and diagnostics. | Do not rely on the flags to change language support, allocation lifetime, or safety/resource limits. This document proposes no optimization work. |
| Raw Node calls/imports | `instantiate(bytes, imports)` returns `{ instance, call }`. Export calls and CommonJS provider callbacks use raw `f64` numbers and `i32` booleans/descriptor handles; no automatic string/array marshaling or language interface is supplied. | The application must know its signatures and provide every import. Providers are trusted executable code, not sandboxed guest source. |
| Explicit conversion | `createAbi(() => instance)` offers `encode(type, value)` and `decode(type, value)`. Strings/arrays are copied only when the application explicitly converts them. | Pass encoded handles to raw calls and return encoded handles from hosts. Mutating a decoded native array does not mutate guest storage; use direct memory access for shared mutation or zero-copy access. |
| Artifact format | The Node runner needs binary Wasm, not WAT. There is no WASI `_start` or automatic guest start function. | Explicitly invoke an exported user function after instantiation. WAT is for inspection or must be assembled externally; a WASI entrypoint would be separate packaging/runtime work. |
| Explicit-copy limits | `createAbi` limits each payload copy to 64 MiB. Its numeric-array encoder accepts plain JavaScript arrays of numbers, not typed arrays, and its string decoder rejects invalid UTF-8. | Use direct memory access for a different application-defined policy. Copy limits are not a guest heap quota or execution budget; guest descriptor checks alone do not validate UTF-8. |
| Export collisions | Memory-backed modules require `memory` and `__alloc`, so user exports cannot collide with them. Scalar-only modules may export functions with those names; the Node runtime does not reserve names. | Leave conflicting functions unselected or use different public names. Hiding a function does not remove its body; callable ABI helpers remain accessible through the raw runtime. |
| Runner invocation | `--args` accepts a JSON array of numeric raw arguments and defaults to `[]`; `--entry` defaults to `main`; `--imports` loads a CommonJS provider. Native strings/nested arrays are not marshaled. JSON has no NaN or infinity literals. Runner failures, including invalid options, exit `1`; successful `void` calls print no result. | Use programmatic raw calls for special numbers or explicit string/array allocation. Descriptor arguments must already be valid handles in the instance. Promise results are not awaited. |

The CLI host grammar has no internal quoting/escape convention. It splits at the
first `=` and `(`, then `)->`, and at the first `.` separating module from field;
module/field text is trimmed. Shell quoting preserves the argument, but does not
change these delimiters. For example, a module name containing a dot or names
requiring leading/trailing whitespace cannot be represented faithfully. The Rust
API accepts arbitrary nonempty module/field strings (including Unicode, controls,
and punctuation); only the source-visible host name must satisfy registration's
ASCII identifier/reserved-name rules.

Raw Wasm `i32` signatures cannot distinguish booleans, strings, and arrays.
The embedding must maintain application-known signatures and provider bindings;
there is no interface metadata or replacement format. The runtime reports file,
compilation, instantiation, and invocation stages. It does not introduce
recoverable guest error types, rollback, or hostile-provider isolation.

## Diagnostics and public typed-AST boundary

Evidence: [tokens.rs](../src/tokens.rs), [typed_ast.rs](../src/typed_ast.rs),
[compiler.rs](../src/compiler.rs), [diagnostic.rs](../src/diagnostic.rs), and
[function context](../src/codegen/function.rs).

- Tokens, statements, functions, and parameters have byte-offset spans, but
  untyped/typed expressions have no independent span. Semantic and nested
  code-generation expression errors generally report the enclosing statement.
  Parsing/analysis/code generation return the first error; there is no recovery
  or multiple-error collection. The CLI already renders located errors with a
  source path/label, line/column, excerpt, and carets; unlocated errors do not
  invent line numbers. UTF-8 boundaries, CRLF, tabs, EOF, and malformed spans have
  renderer tests. Columns count Unicode scalar characters; excerpt tabs expand
  to four spaces, not terminal tab stops. Grapheme/display-width alignment is not
  modeled. Fix/recompile incrementally and treat the highlighted statement as
  context, not an expression-precise location. Stage errors expose a common
  `span() -> Option<Span>` accessor, delegated by `CompileError`; accessors return
  raw byte offsets and the renderer alone clamps presentation. Unlocated errors
  remain `None`. Precise expression spans/recovery are separate enhancements.
- `TypedProgram` and its nodes/IDs/type annotations are public and mutable.
  `Compiler::analyze` creates a checked program, including an owned host-import
  snapshot, but callers can forge inconsistent input before `compile`. The
  backend checks many malformed IDs, scopes, types, calls, and literals; this is
  not a guarantee that every arbitrary public typed tree produces a recoverable
  error. A private [checked constructor](../src/codegen/validation.rs) now borrows
  the tree, completes layout, and lowers all user bodies through existing backend
  checks before module assembly can proceed. It preserves first-error order and
  caches the bodies rather than running a second independent validator. This is
  an internal boundary, not a public checked-AST API or complete source-semantic
  revalidation. Planning/lowering still contain `expect`/`assert` invariants for
  memory, interned strings, and temporary slots. Prefer analyze-then-compile
  without unchecked edits. Optional public checked construction and stronger
  arbitrary-input guarantees need separate API and validation decisions.
- Compile errors return no partially emitted module. This does not make runtime
  traps transactional or protect host callbacks from application-level errors.

## Evidence and validation boundaries

The [backend tests](../src/codegen/tests.rs) check deterministic output, import-first
indices, signature lowering, errors, evaluation order, and IEEE-754 behavior.
The [memory tests](../src/codegen/memory_tests.rs) cover aliasing, UTF-8 payloads,
invalid indices/descriptors, allocator growth, and trusted raw-host mutation.
[Export tests](../src/codegen/export_tests.rs) characterize selection, unchanged
non-export sections/indices, required memory exports, and validation of hidden bodies.
[Remainder tests](../tests/remainder.rs) cover optional helper layout, result
types/spans, exact binary64 edge cases against an independent reference, and
once-only left-to-right operand effects. The compiler-in-Wasm path uses the same
pipeline; [browser tests](../tests/browser.rs) execute emitted guest modules.

[CLI integration tests](../tests/cli.rs) cover required source files, format
inference, raw binary output, exports/imports, exit codes, diagnostics, output
preservation, and canonical same-input refusal. [Runtime tests](../tests/runtime.rs)
cover raw invocation/imports, explicit ABI conversion, descriptor validation,
UTF-8, copy limits, memory growth, bounds traps, and reported error stages.
Diagnostic unit tests live in their implementation module.

Run the focused integration suites with Node enabled:

- `cargo test --offline --test cli -- --include-ignored`
- `cargo test --offline --test runtime -- --include-ignored`
- `cargo test --offline` for the default suite.

Node cases are marked `#[ignore = "requires Node.js"]`; default test success
does not execute them. Use `cargo test --offline -- --include-ignored` to include
all backend and integration runtime cases when Node.js is available. These tests
verify the supported contract, not hostile-host isolation, transactional guest
execution, or crash-durable filesystem behavior.
