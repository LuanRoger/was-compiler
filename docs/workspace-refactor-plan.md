# Compiler workspace refactor handoff

## Status and scope

The user confirmed this consolidated design and authorized implementation on
2026-10-15. Package extraction, test relocation, boundary checks, the Divan harness,
and Bacon configuration are implemented. Validation evidence is recorded below;
this plan does not authorize language or optimization changes.

[ADR 0005](adr/0005-compiler-and-adapter-workspace-boundaries.md) records the
architectural decision and trade-offs. This is a behavior-preserving package
extraction, not a language, API, ABI, or optimization change. WP12 and WP13 remain
outside scope. Preserve existing user changes and do not stage or commit.

## Workspace shape

```text
Cargo.toml                       # Virtual workspace; shared lockfile
bacon.toml                       # Developer jobs and validation chains
crates/
  compiler/                      # was-compiler; library was_compiler
    src/
    tests/
    benches/
  cli/                           # was-compiler-cli; binary was-compiler
    src/
    tests/
  browser/                       # was-compiler-browser
    src/
    tests/
tests/                           # Shared test-only assets and support
runtime/                         # Existing Node execution helpers
examples/                        # Existing examples and playground
pkg/                             # Generated browser artifacts, still ignored
target/                          # Generated output, still ignored
```

Compiler and CLI are default members. Keep lexer, parser, semantic analysis,
checked lowering, and module assembly as compiler modules, not separate crates.
Keep native filesystem/CLI dependencies out of core production dependencies and
browser bindings in their adapter. Shared test support is not a production crate.
Do not move or remove the separate Python implementation as incidental cleanup.
Preserve `target/wp7-inference`.

## Compatibility

Preserve compiler package/library identity and public module/type/API contracts,
including owned analysis snapshots and independent validation of mutable public
typed trees. Preserve deterministic emitted bytes, diagnostics, optimization-level
parity, host/export ordering, and memory ABI. Existing ADRs remain in force.

Preserve CLI executable name, arguments, exit statuses, output inference,
filesystem safety behavior, and library-byte parity. Preserve browser JavaScript
API and generated `was_compiler` filenames; root `pkg/` remains the output location.

Retain root `cargo test --offline` and `cargo run --offline -- ...` workflows.
Local CLI installation changes to `cargo install --offline --path crates/cli`.
Browser build commands change to use `crates/browser` and its new relative output
path. Document repository-relative path changes and downstream path-dependency
updates. Crates.io publication and release automation are not part of this work.

## Test ownership and additions

- Compiler: frontend, inference, forged typed-input checks, planners/lowering,
  independent decoded Wasm assertions, ABI/emitter/diagnostic fixtures, and guest
  execution behavior. Private implementation tests stay with implementation.
- CLI: argument, file/output, executable, and CLI/core parity contracts. Tests using
  `CARGO_BIN_EXE_was-compiler` belong to the package owning the executable.
- Browser: JavaScript API and native/browser parity, independent of CLI selection.
- Split mixed adapter suites as necessary; retain shared fixtures, Node scripts,
  and bounded subprocess support at repository level where useful. Explicitly
  attach test targets to their owning packages: a virtual root discovers none.
- Replace assumptions that `CARGO_MANIFEST_DIR` is the repository root. Preserve
  golden bytes and newline conventions rather than regenerating expected results.

Required new evidence:

1. A downstream-style consumer can use the existing public compiler API.
2. Core production dependencies exclude CLI/browser adapter dependencies; inspect
   production edges separately from dev-only benchmark/validation dependencies.
3. Core builds natively and for `wasm32-unknown-unknown`.
4. CLI/core bytes and diagnostics remain equal after extraction.
5. Browser/core parity uses freshly rebuilt current-source artifacts.
6. Shared fixtures and subprocess helpers resolve from their new owning packages.

Audit for additional concrete gaps after those boundaries are covered. Do not use
an arbitrary test-count or coverage-percentage target. Retain existing ignored
runtime/browser policy; default test success is not runtime release evidence.

## Native compiler benchmarks

Use Divan as a development-only dependency if the entire dependency graph resolves
and builds offline. Local presence of Divan 0.1.21 is not proof of offline build
availability. If resolution fails, report the prerequisite; do not download a
replacement or silently change harnesses.

Use existing public APIs for five measurements:

| Measurement | Prepared outside timing |
| --- | --- |
| Lexing | Source |
| Parsing | Tokens |
| Semantic analysis | Untyped AST |
| Typed-program compilation | Typed program |
| End-to-end source compilation | Source |

Include stage processor construction and output allocation/destruction in timing;
end-to-end timing includes compiler construction. Initial cases register no hosts.
Consume outputs to prevent eliminated work and validate all workloads outside the
timed region. Do not introduce public APIs for private backend substages.

Use a small fixed corpus: scalar, call/control-flow, and arrays/ranges/closures,
with deterministic size variants. Record source size and emitted module size, plus
machine, toolchain, workload revision, and commands. Run optimized benchmark builds
and preserve baselines as measurement evidence without enforcing a percentage gate.
Do not treat cross-machine results as directly comparable regression evidence.

Guest execution, CLI/browser startup, WAT serialization, allocation profiling,
private backend instrumentation, and optimization implementation are deferred.

## Bacon workflow

Use a root `bacon.toml`, not a validation shell script or another Rust package.
Bacon 3.22.0 was available during design; do not silently install or upgrade tools.

- Default `check`: quick native compiler/CLI checks.
- Individual jobs: tests, strict Clippy, core Wasm build, architecture checks,
  browser rebuild/checks, ignored runtime tests, and benchmarks.
- `fast`: ordered native tests, strict Clippy, core Wasm build, and architecture
  checks. Do not select the browser package implicitly in native-only commands.
- `full`: fast checks, current-source browser rebuild, browser checks, full ignored
  test coverage across owning packages, and browser-target Clippy.
- Benchmarks and expensive chains are explicitly selected and have automatic
  file-change reruns disabled. Use native success actions, stop on failure, avoid
  cycles, and preserve browser-rebuild-before-browser-test ordering.
- Watch relevant workspace sources and shared test assets for ordinary development
  jobs, while avoiding generated-output rebuild loops.

Document a direct finite command for every job and the ordered direct-command
sequence for each chain. Direct commands are the authoritative bounded agent and
release evidence; Bacon is the interactive interface, not a one-shot runner.
`--headless` still watches. A successful final job does not prove prior jobs ran.
Verify the installed Bacon configuration behavior during implementation rather
than assuming job selection or disabled watching implies one-shot process exit.
All Cargo resolution remains offline. Hosted CI is deferred.

## Sequential implementation and release gates

1. Record baseline validation and representative compiler output before moves.
   Include scalar, memory/helper, host integration, and local-callable paths; use
   independent existing fixtures rather than deriving all expectations from core.
2. Extract the three packages and relocate tests; fix dependency and asset paths.
   Do not combine this with unrelated cleanup or new behavior.
3. Pass compatibility checks before introducing new tests and benchmark machinery.
4. Add required package-boundary coverage, audit additional gaps, add the benchmark
   corpus/harness, and configure Bacon with equivalent direct commands.
5. Update README, agent guide, browser/runtime guides, and affected live paths and
   workflow references. Keep historical validation evidence clearly historical.
6. Run final native and current-source browser release checks, then smoke-run the
   benchmark suite. Review diff/status and run `git --no-pager diff --check`.

Use existing documented offline validation commands as the baseline, adapting
package selection to the accepted ownership. Full validation includes native
compiler/CLI tests and strict Clippy; native/Wasm core build and architecture
checks; current-source wasm-pack rebuild with `--mode no-install --no-opt --offline`;
Node browser checks; explicitly included ignored tests across owning packages; and
browser-target strict Clippy. Do not run an unbounded watcher in agent tools.

Do not claim completion if required gates are unavailable or failing. Report the
exact command, missing prerequisite or failure, and remaining unverified coverage;
do not fetch tools/dependencies, alter fixtures, or fix unrelated failures just to
make validation appear successful.

## Implementation and validation evidence — 2026-10-15

The workspace and test ownership split are complete. An audit against the original
Git tree preserved all 724 existing integration-test declarations and their 140
ignored attributes. Six new tests cover the standalone consumer, production
dependency isolation, shared asset paths, benchmark determinism/validation/parity,
workload size scaling, and independent Node execution of all nine workloads. The
new Node test is ignored by default, retaining the repository's runtime policy.

All production source files are unchanged beyond relocation. Only two existing
source files differ: `crates/compiler/src/lib.rs` and
`crates/compiler/src/codegen/validation_tests.rs` update test-only shared-asset
paths. Golden fixtures, Python implementation, runtime helpers, and browser
playground behavior were preserved.

Passed before extraction: the existing full native ignored-test run, native
Clippy, current-source browser rebuild and all 25 browser script groups, and
browser-target Clippy. After extraction and before new coverage/harness work,
passed root tests, full workspace ignored tests, native Clippy, a fresh browser
rebuild, browser script groups, and four module hash comparisons.

Final direct validation passed:

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo build --offline -p was-compiler --lib --target wasm32-unknown-unknown
wasm-pack build crates/browser --target web --out-dir ../../pkg --out-name was_compiler --mode no-install --no-opt --offline
node tests/browser.cjs pkg
cargo test --offline --workspace -- --include-ignored
cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings
cargo bench --offline -p was-compiler --bench compiler -- --test
```

The standalone downstream fixture builds/runs offline with its own lockfile and
isolated target directory. CLI smoke compilation/execution returned `65`. Local
CLI installation passed with `cargo install --offline --path crates/cli --root
target/workspace-install --no-track`, followed by the installed binary's help
command; this did not install into the user's normal Cargo binary directory.

Bacon 3.22.0 parsed/listed the jobs; both actual fast/full chains ran successfully
under bounded subprocesses, temporarily overriding only the final node's success
action to quit. Static checks also verified acyclic chains, manual watch settings,
and browser rebuild ordering. The repository configuration remains interactive,
not a finite runner. Direct commands above provide the release evidence.

The optimized 45-case measurement run also passed with 100 samples and one
iteration per sample using the OS timer. See [benchmark boundaries and recorded
baseline](benchmarks.md); the initial laptop measurements are not a performance
budget. All Markdown local link targets and whitespace checks passed.

### Before/after module hashes

The CLI was used before and after extraction with the same source/options and
ordered host declarations `double=env.double(number)->number`, then
`print=env.print(number)->void` (including unused declarations). All four SHA-256
hashes matched:

| Case | Source | SHA-256 |
| --- | --- | --- |
| Scalar | `examples/fib.was` | `fa33d1cd2562e7322225ea9cfc33c636c8da4789f156355c2ab43a569b58515e` |
| Memory | `examples/array-sum.was` | `e4ba85719fa9ff975927a184a04d52e705840f847dd01f1903aa51e4a5e8404b` |
| Shared closure | `fn main() -> number { let n = 1; let f = () => { n += 2; n }; return f(); }` | `dafc3e2c21ff1736c5882b503fad8d4e2070e34da10d42bbd263c73ade3362de` |
| Host import | `fn main() -> number { return double(21); }` | `ab2cb3fe084562070440f67cb423b2cd4a41414e493ddae35c6f1e6ddda101c9` |

Generated baseline inputs/modules and smoke-install artifacts are under ignored
`target/`; historical golden fixtures were not regenerated. No dependencies or
tools were fetched, no optimization/language change was made, and no commit or
branch was created.
