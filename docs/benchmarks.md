# Native compiler benchmarks

The compiler package uses Divan 0.1.21 as a development-only dependency. It does
not enter the core's production dependency graph. Run from the repository root:

```sh
cargo bench --offline -p was-compiler --bench compiler
cargo bench --offline -p was-compiler --bench compiler -- --test
```

The second command smoke-runs all 45 cases once; it is not a timing baseline.
`bacon bench` and `bacon bench-smoke` select the same commands without automatic
source-change reruns. Dependencies/tooling must already be available offline.

## Timing contract

| Measurement | Prepared outside timing | Included in timing |
| --- | --- | --- |
| `lex` | Source | New lexer, tokenization, token destruction |
| `parse` | Tokens | New parser, AST construction/destruction |
| `semantic` | Untyped AST | New analyzer, typed-program construction/destruction |
| `typed_compile` | Typed program | New compiler, checked compilation, Wasm-buffer destruction |
| `source_compile` | Source | New compiler and entire source-to-Wasm pipeline |

The timed closures return `()` and explicitly `black_box_drop` their outputs so
Divan does not move output destruction outside the timed region. Inputs cross a
black-box boundary too. Setup and validation are outside the measured loop; each
invocation recreates the stateful processor. There are no registered hosts or
production API changes. All measurements use default compiler options (currently
all optimization levels produce identical output) and optimized Cargo bench builds.
Stage medians do not necessarily sum to the end-to-end median: setup, intermediate
lifetimes, cache state, and scheduling differ.

The corpus lives in
[`benches/support/corpus.rs`](../crates/compiler/benches/support/corpus.rs):

- `scalar`: one function with arithmetic and scalar compound updates.
- `calls`: main plus named functions, defaults, contracts, and branches.
- `captures`: local callables sharing scalar/array bindings, range and array loops.

Each family has size variants 1, 16, and 64. These are fixed synthetic workloads,
not a claim of representative production traffic. Loops and calls are bounded.
Corpus construction is deterministic, and regression tests check valid Wasm,
optimization-level parity, and increasing source/module sizes. An ignored Node test
also executes all nine workloads against independent expected results, including
compound assignment's pre-RHS read rule. The harness validates
modules outside timing and prints source/module byte sizes before measurements.

This first suite excludes guest execution, adapter/process startup, WAT printing,
allocation profiling, private backend substages, and optimization implementation.

## Baselines and comparison

For an explicitly bounded measurement run:

```sh
cargo bench --offline -p was-compiler --bench compiler -- --sample-count 100 --sample-size 1 --timer os --color never
```

Preserve the complete output and record date, CPU/OS, Rust toolchain, command,
Git revision plus working-tree status, and hashes of corpus/harness sources. Run
on an otherwise idle machine under comparable power/thermal conditions; repeat
runs before interpreting a difference. Do not treat cross-machine measurements
or one noisy run as regression evidence. There is no percentage-based gate.

The initial [2026-10-15 baseline](benchmarks/2026-10-15-workspace.txt) records all
45 measurements and module sizes. It was taken on Windows 11 / Intel Core Ultra 5
125H with rustc 1.98.0, using the command above. The workspace refactor and new
harness were uncommitted atop the recorded Git revision. The small sample-size run
is an initial measurement record, not an optimizer comparison or a stable
performance budget; visible sample spread warrants repeated measurements.
