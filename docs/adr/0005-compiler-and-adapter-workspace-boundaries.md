# Separate compiler and adapter packages without changing behavior

On 2026-10-15, the design interview accepted a portable compiler core, native CLI
adapter, and browser adapter as the workspace's initial package boundaries. Keep
compiler stages as internal modules unless a concrete need justifies another
package: adapter dependency isolation and clear ownership justify this split,
whereas stage-per-crate decomposition would impose additional interfaces without
a demonstrated benefit.

Preserve the existing compiler package/library identity and public APIs, CLI
executable name and behavior, browser JavaScript API and generated filenames,
diagnostics, and emitted Wasm bytes. Document changes to repository paths and
build/development commands; language changes, API redesign, and optimization are
outside this refactor.

Preserve existing tests and add coverage for identified contract and migration
risks rather than a test-count or coverage-percentage target. Establish
reproducible native compiler benchmarks first, separating end-to-end compilation
from selected stage measurements; guest execution and adapter startup are not the
initial benchmark scope. Measurement does not authorize WP13 optimization.

Use a virtual workspace root with `crates/compiler` (package `was-compiler`,
library `was_compiler`), `crates/cli` (package `was-compiler-cli`, executable
`was-compiler`), and `crates/browser` (package `was-compiler-browser`). Tests belong
to their compiler or adapter boundary; split mixed adapter suites as needed and
retain repository-level test-only fixtures and subprocess support. Do not add a
production test-utilities crate or make browser validation depend on CLI selection.

Use Divan as a development-only benchmark dependency if its complete dependency
graph resolves and builds offline; otherwise report the missing prerequisite.
Measure lexing, parsing, semantic analysis, typed-program compilation, and
end-to-end compilation through existing public APIs. Use deterministic scalar,
call/control-flow, and arrays/ranges/closures workloads with size variants, and
record source/module sizes alongside timings. Exclude private backend substage
instrumentation, WAT serialization, guest execution, and allocation profiling from
the initial suite.

Provide repository-level Bacon configuration rather than a shell validation
script, distinguishing fast native checks from full checks that rebuild
current-source browser artifacts and explicitly execute ignored runtime/browser
tests. Hosted CI and percentage-based performance gates are deferred; initial
benchmark results establish baselines.

Select compiler and CLI as default workspace members, retaining root native
`cargo test` and `cargo run` workflows. Document local CLI installation through
`cargo install --offline --path crates/cli` and the browser build path change;
publication and release automation are outside scope.

Migrate sequentially: record baseline validation and representative outputs;
extract packages and relocate tests; pass compatibility checks; add new boundary
coverage and benchmarks; update documentation and run final release checks.
Required coverage includes a downstream-style public API consumer, core production
dependency isolation, native/Wasm core builds, CLI/core parity, current-source
browser/core parity, and relocated fixture/support resolution. Audit additional
gaps without duplicating existing behavioral coverage.

Prepare benchmark inputs and prerequisite stage outputs outside timing; include
stage processor construction and output allocation/destruction. End-to-end timing
includes compiler construction, initially without registered hosts. Use optimized
builds, consume outputs, validate workloads outside timing, and record toolchain,
machine, workload revision, and command. Cross-machine results are not directly
comparable regression evidence.

Configure Bacon's native jobs and success actions for ordinary development jobs
and ordered fast/full chains; expensive chains and benchmarks are explicitly
selected with automatic change-triggered reruns disabled. Stop on failure, avoid
cycles, and rebuild browser artifacts before dependent tests. Retain documented,
finite direct commands as the authoritative agent/release validation evidence:
Bacon is watch-oriented, including in headless mode, not a one-shot test runner.
Do not require a shell wrapper or silently install developer tooling.

The user confirmed the [implementation handoff](../workspace-refactor-plan.md)
and authorized implementation on 2026-10-15. The workspace extraction, test
ownership split, boundary coverage, Divan harness, and Bacon configuration implement
these decisions; validation evidence is recorded with the handoff.
