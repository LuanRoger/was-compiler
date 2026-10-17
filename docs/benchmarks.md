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

## Issue #37 M0: guest allocation, capacity, and execution baseline

M0 is measurement-only, before typed-array production changes. It adds no
production exports, dependencies, manifest entries, compiler transformations, or
marshaling API. The original 45 compiler benchmarks and their timing boundaries
above are unchanged; guest execution is **not** included in those timings.

### Workloads and confirmed seams

The deterministic source generator is
[`tests/support/memory_corpus.rs`](../tests/support/memory_corpus.rs), with the
Node harness in
[`tests/support/memory_measure.cjs`](../tests/support/memory_measure.cjs).
The auto-discovered
[`memory_benchmarks` integration target](../crates/compiler/tests/memory_benchmarks.rs)
uses issue #37's approved public source analysis/compilation and execution seams.
It checks valid Wasm, source/typed compilation equivalence, deterministic bytes,
all four optimization levels, decoded export/memory contracts, and no imports or
start effects. Execution checks independent, worked result/allocation expectations.
No private compiler instrumentation or testing-only production abstraction is used.

Seventeen bounded cases cover:

- `construct_{0,1,7,8,9}`: 128 fresh literals, including empty arrays and lengths
  around an eight-element boundary. Numeric payloads remain eight bytes per element.
- `page_crossing`: 8,192 fresh one-element arrays, consuming two pages of aligned
  allocation and crossing from one to three pages of capacity on a fresh instance.
- `alias_index_replace`: shared handles, indexed reads, replacement and length,
  repeated 128 times after one construction.
- `iteration_live_alias`: 128 array traversals; a write during traversal changes
  the later yielded element, rather than a snapshot.
- `equality_{length,early,late,equal}`: 128 comparisons of separately allocated
  arrays, exercising length, first-element, last-element and no-mismatch exits.
- `string_{7_equal,8_early,9_late}`: 128 UTF-8 content comparisons, with payload
  byte lengths around eight-byte alignment. Literals are static/interned: their
  storage contributes to module/initial-memory size, not runtime bump consumption.
- `capture_shared`: shared scalar and array captures, alias visibility, 128 calls.
- `capture_iteration`: 128 traversals of four elements; fresh read-only iteration
  cells/environments, 512 helper calls in total.

A separate raw-allocator check requests 0, 1, 7, 8, and 9 bytes and independently
expects reservations of 8, 8, 8, 8, and 16 bytes. These are baseline expectations,
not production constants imported back into tests. Each workload is bounded in
source; the existing subprocess helper also applies a 60-second child deadline.
That deadline is not guest fuel or process-tree cancellation.

### Measurement isolation and commands

Run from the repository root with cached dependencies and Node.js 24+ on PATH:

```sh
cargo test --offline -p was-compiler --test memory_benchmarks
cargo test --offline -p was-compiler --test memory_benchmarks memory_workloads_execute_with_independent_results_and_allocation_checks -- --ignored --exact
cargo test --offline -p was-compiler --test memory_benchmarks record_memory_markers -- --ignored --exact --nocapture --test-threads=1
cargo test --offline -p was-compiler --test memory_benchmarks record_memory_capacity -- --ignored --exact --nocapture --test-threads=1
cargo test --offline -p was-compiler --test memory_benchmarks record_memory_guest_timing -- --ignored --exact --nocapture --test-threads=1
```

The three recording commands deliberately use separate Node processes/modes:

| Mode | Observation | Excluded / interpretation |
| --- | --- | --- |
| `marker` | `after - before - 8` using existing ABI `__alloc(0)` around one `main()` | Subtracts the first marker's minimum reservation; second marker's address precedes its reservation. Aligned consumption includes padding, cells and environments, not live bytes. No page/timing measurements. |
| `capacity` | Initial/final `memory.buffer.byteLength / 65536`, one call on a fresh unmarked instance | Capacity only, not live bytes, allocator consumption, process RSS, or static-data bytes in use. No markers or timing. |
| `timing` | 8 warmup calls, then 20 samples of 4 calls; fresh unmarked instance per sample, shared compiled module | `hrtime.bigint()` around guest-call batch only. Includes JS/Wasm call, loop/checksum and guest growth overhead. Excludes source generation/compilation, module compilation, instantiation, warmup, assertions, hashes, reporting, host marshaling and process startup. No markers or page observations. |

The verification mode combines marker/capacity correctness checks only on **distinct**
instances; it does not report timings. Recording tests are ignored by default,
consistent with existing Node/runtime policy. Default Cargo success is not evidence
that they executed. For the entire focused suite, use `-- --include-ignored
--test-threads=1`. Timings are nanoseconds **per main call**, not per inner operation;
most calls perform 128 operations, while page crossing performs 8,192 allocations.
Warmup is finite, not a promise of settled V8 tiering. Fresh sample heaps include
first-touch/growth behavior; allocation state never accumulates across samples.

Run the unchanged compiler timing baseline separately, not concurrently:

```sh
cargo bench --offline -p was-compiler --bench compiler -- --sample-count 100 --sample-size 1 --timer os --color never
```

### Recorded M0 baseline — 2026-10-15

Full evidence (commands and exit codes included):

- [Environment, revision/status, Rust/Cargo/Node and file SHA-256 hashes](benchmarks/2026-10-15-m0-environment.txt).
- [Marker consumption, source/module sizes and per-workload SHA-256 hashes](benchmarks/2026-10-15-m0-markers.txt).
- [Unmarked initial/final memory pages](benchmarks/2026-10-15-m0-capacity.txt).
- [Guest timing run 1, all raw samples](benchmarks/2026-10-15-m0-guest-1.txt)
  and [run 2](benchmarks/2026-10-15-m0-guest-2.txt).
- [Separate unchanged compiler harness, all 45 timings](benchmarks/2026-10-15-m0-compiler.txt).
- [Validation command output](benchmarks/2026-10-15-m0-validation.txt).

Revision: `e720e2b0330ae140bc0d2436358858d44ab1114f` with uncommitted M0 files
and the user's pre-existing glossary/design/spec/ADR edits. Those user files were
not changed. The metadata snapshot precedes creation of these evidence files and
this documentation update. Source/module hashes identify exact generated workloads;
harness/corpus/test hashes identify the uncommitted measurement implementation.

Machine: Windows `10.0.26300`, x64, Intel Core Ultra 5 125H, 18 logical CPUs,
16,730,726,400 bytes physical memory; rustc/Cargo 1.98.0, LLVM 22.1.8,
Node 24.12.0 / V8 13.6.233.17-node.37. Rust integration caller uses the debug
profile; Wasm output uses default compiler options and is invariant across levels.
The separate native compiler timings use the optimized bench profile. Runs were
sequential; power, thermal state and background activity were not controlled.
These small samples are baseline evidence, **not** a speedup claim or threshold.

| Workload | Module bytes | Aligned bump bytes/call | Pages before → after | Guest median ns/call, run 1 / 2 |
| --- | ---: | ---: | --- | ---: |
| construct_0 | 776 | 1,024 | 1 → 1 | 1,950 / 2,237.5 |
| construct_1 | 790 | 2,048 | 1 → 1 | 2,300 / 2,362.5 |
| construct_7 | 876 | 8,192 | 1 → 1 | 3,712.5 / 3,537.5 |
| construct_8 | 890 | 9,216 | 1 → 1 | 4,137.5 / 3,825 |
| construct_9 | 904 | 10,240 | 1 → 1 | 5,325 / 5,100 |
| page_crossing | 803 | 131,072 | 1 → 3 | 84,637.5 / 91,662.5 |
| alias_index_replace | 884 | 32 | 1 → 1 | 6,425 / 5,925 |
| iteration_live_alias | 943 | 40 | 1 → 1 | 14,012.5 / 11,575 |
| equality_length | 947 | 88 | 1 → 1 | 2,362.5 / 1,950 |
| equality_early | 1,046 | 144 | 1 → 1 | 2,625 / 2,200 |
| equality_late | 1,046 | 144 | 1 → 1 | 4,250 / 6,100 |
| equality_equal | 1,046 | 144 | 1 → 1 | 4,425 / 4,000 |
| string_7_equal | 786 | 0 | 1 → 1 | 3,675 / 3,600 |
| string_8_early | 803 | 0 | 1 → 1 | 2,237.5 / 1,962.5 |
| string_9_late | 812 | 0 | 1 → 1 | 3,750 / 3,100 |
| capture_shared | 946 | 48 | 1 → 1 | 3,675 / 3,375 |
| capture_iteration | 929 | 8,232 | 1 → 1 | 13,075 / 13,900 |

M0 does not implement new array types, alter allocation/checking, or authorize M1.
Boolean/string-array measurements belong to their later implementation milestones.
No reclaimed/live-byte, RSS, browser execution, dynamic-string construction, host
marshaling, or performance-budget claims are made by this synthetic baseline.

## Issue #37 M4: comparative measurements and optimization handoff

The M4 measurement portion is recorded in
[`m4-typed-arrays/`](benchmarks/m4-typed-arrays). This does **not** claim the
parent's full rebuilt-browser/ignored-runtime release gates. No production,
existing test, M0 input/evidence, dependency, or Cargo manifest was changed by
this measurement task.

### Reproduction and isolation

Run from the repository root, offline with cached Rust dependencies, Node.js 24+
(and Python for the optional stdlib-only recorder). Choose a **new, empty** output
directory; the recorder refuses to overwrite evidence:

```sh
python tests/support/record_memory_m4.py docs/benchmarks/NEW-M4-RUN
```

The [recorder](../tests/support/record_memory_m4.py) checks original M0
corpus/harness/test/compiler-benchmark hashes against M0's metadata; records the
current dirty tree, toolchains and production file hashes; runs the unchanged M0
numeric marker/capacity/timing modes and new matched typed-array modes in separate
processes; repeats guest and unchanged compiler timings twice; then checks that
production, measurement inputs and historical evidence stayed unchanged. Each
command has a 120-second subprocess bound; each Rust→Node call retains the
existing support helper's 60-second deadline. It generates comparisons without
fetching anything, changing working-tree sources, or adding profiling exports.

Direct commands (all exact invocations are also in the raw records):

```sh
# Original M0 target and timing contract, unchanged; repeat timing command twice.
cargo test --offline -p was-compiler --test memory_benchmarks record_memory_markers -- --ignored --exact --nocapture --test-threads=1
cargo test --offline -p was-compiler --test memory_benchmarks record_memory_capacity -- --ignored --exact --nocapture --test-threads=1
cargo test --offline -p was-compiler --test memory_benchmarks record_memory_guest_timing -- --ignored --exact --nocapture --test-threads=1

# New matched number/bool/string controls, separate from M0; repeat timing twice.
cargo test --offline -p was-compiler --test typed_array_measurements record_typed_array_markers -- --ignored --exact --nocapture --test-threads=1
cargo test --offline -p was-compiler --test typed_array_measurements record_typed_array_capacity -- --ignored --exact --nocapture --test-threads=1
cargo test --offline -p was-compiler --test typed_array_measurements record_typed_array_guest_timing -- --ignored --exact --nocapture --test-threads=1

# Original compiler harness, separately and sequentially; repeat twice.
cargo bench --offline -p was-compiler --bench compiler -- --sample-count 100 --sample-size 1 --timer os --color never
```

New workload/support files:
[`typed_array_measure_corpus.rs`](../tests/support/typed_array_measure_corpus.rs),
[`typed_array_measure.cjs`](../tests/support/typed_array_measure.cjs), and the
[`typed_array_measurements` integration target](../crates/compiler/tests/typed_array_measurements.rs).
They add 48 cases: matched number/bool/string construction at lengths 0/1/2/3/7/8/9,
eight-element page crossings, alias/index/replacement, live iteration, equality
length/early/late/equal exits, shared captures and read-only iteration captures.
There are 128 inner repetitions, except 8,192 page-crossing constructions.

The new harness follows M0's timing protocol: 8 warmup calls, 20 samples of 4
calls, fresh unmarked instance per sample and shared compiled module. Timer scope
includes JS loop/checksum, JS/Wasm calls and guest work/growth, but excludes
compilation, instantiation, warmup, checks, hashing, reporting and marshaling.
Marker and page modes remain independent. Descriptor-byte fixtures separately
confirm widths/alignment/UTF-8/shared handles, without adding a function to measured
modules. Validation uses public source/typed compilation, encoded contracts and
actual execution, never private profiling interfaces.

### Evidence and interpretation

- [Environment, current production/input hashes and preserved M0 hashes](benchmarks/m4-typed-arrays/environment.json).
- [Input/production stability across measurements](benchmarks/m4-typed-arrays/stability.json).
- [Full comparison tables](benchmarks/m4-typed-arrays/comparison.md) and
  [machine-readable comparison](benchmarks/m4-typed-arrays/comparison.json).
- Numeric [markers](benchmarks/m4-typed-arrays/numeric-markers.txt),
  [capacity](benchmarks/m4-typed-arrays/numeric-capacity.txt),
  [guest run 1](benchmarks/m4-typed-arrays/numeric-guest-1.txt) and
  [run 2](benchmarks/m4-typed-arrays/numeric-guest-2.txt).
- New typed [markers](benchmarks/m4-typed-arrays/typed-markers.txt),
  [capacity](benchmarks/m4-typed-arrays/typed-capacity.txt),
  [guest run 1](benchmarks/m4-typed-arrays/typed-guest-1.txt) and
  [run 2](benchmarks/m4-typed-arrays/typed-guest-2.txt).
- Unchanged compiler harness [run 1](benchmarks/m4-typed-arrays/compiler-1.txt) and
  [run 2](benchmarks/m4-typed-arrays/compiler-2.txt), all 45 timing cases each.
- [Validation output](benchmarks/m4-typed-arrays/validation.txt) and
  [final source/M0 hash, link and syntax checks](benchmarks/m4-typed-arrays/final-checks.json).

All **17/17 M0 source and module SHA-256 hashes match**. Allocation consumption,
module sizes, results and pages match too. The separate compiler corpus retains
all nine source/module size pairs; its historical evidence contains sizes, not
module hashes. Same machine/runtime/toolchain versions are recorded, but CPU
power/thermal/background activity and V8 tiering were uncontrolled. Most numeric
guest medians are higher despite identical emitted bytes; current compiler runs
also vary substantially. No attributable speedup or implementation regression is
established. Only one historical M0 compiler run exists, so a repeated historical
compiler distribution cannot be reconstructed.

Matched eight-element construction (128 allocations/call) consumes **9,216 /
2,048 / 5,120 aligned bytes** for number/bool/string arrays. The larger page case
consumes **589,824 / 131,072 / 327,680 bytes**, with capacity **1→10 / 1→3 / 1→6
pages**. This is **element-width savings**, not a numeric optimization or
reclamation: descriptors and minimum/alignment policy are unchanged; string
payloads here are static/interned, not per-occurrence host-encoded copies. Empty
arrays still consume 1,024 bytes per call in every matched type. Iteration captures
still consume 8,232 / 8,208 / 8,216 bytes: 8,192 bytes of uniform cells/environments
plus the initial array. The allocator remains monotonic, not reclaiming storage.

### Ranked follow-up handoff

The [detailed handoff](benchmarks/m4-typed-arrays/handoff.md) records measured
motivation and separate aliasing, lifetime, host-effect and trap-order proof
obligations for each candidate, ranked as:

1. Proven non-escaping loop-local literal scalar replacement/allocation avoidance.
2. Non-escaping capture cell/environment specialization.
3. Proven redundant structural-check / checked-loop specialization.
4. New-type helper reachability specialization for module footprint.

These are proposals, **not authorization**. Each needs explicit separate WP13
approval, an optimizer/parity agreement, correctness/forged-input/effect tests,
and controlled repeated measurements. Allocation removal can alter observable
allocator positions, memory capacity and OOM/effect order; those are contract
questions, not incidental cleanup. No heap rewind, reclamation, general legacy
helper pruning or unchecked consumed-value fast path is authorized here.

## Issue #38: integer-specialization baseline and comparison

**Status:** Integrated baseline harness for 27 workloads and four presets.
No production specialization is implemented by #38. See
[proof, preset and release contracts](integer-specialization.md)
and [the scheduling exception](was_language_extensions_plan.md#issue-38--proven-integer-specialization-baseline-exception).
All levels currently remain identical no-ops; `None` is the default byte baseline.

### Separate harness and timing boundaries

The shared corpus is `tests/support/integer_corpus.rs`, the Node verifier
and timing harness is `tests/support/integer_measure.cjs`, and the compiler-owned
integration target is `crates/compiler/tests/integer_baselines.rs`. The new
`crates/compiler/benches/integer_specialization.rs` is a **separate Divan target**
using existing tooling. It does not extend or reinterpret the original 45-case
`compiler` benchmark or change its default-option baseline. Historical #37 M0/M4
inputs, outputs, allocation observations and timing protocols remain historical;
their 8-warmup/20-by-4 guest protocol is not replaced by this one.

| Observation | Timing / comparison boundary |
| --- | --- |
| Compilation cost | Separate typed-program and source-to-Wasm measurements through public APIs; prerequisite inputs outside timing, fresh compiler construction/host registration and output destruction inside timing, inputs/outputs black-boxed. Corpus generation and validation remain outside timing. |
| Module size | Exact emitted Wasm byte length and module hash recorded outside timing, matched by workload and preset; not WAT length or an instruction-count proxy. |
| Guest execution | 128 warmup calls on a separate warm instance, then 20 samples of 64 calls with a fresh instance per sample and shared compiled module/JIT state. Timer includes guest work, JS/Wasm calls, batch-loop overhead and host JS mark bookkeeping for `effects`. Raw samples are nanoseconds per main call, not batch durations. |
| Correctness / trap controls | Independent assertions, expected effects and expected traps outside timing. Trapping controls are verify-only and receive no guest-time result. |

Guest timing excludes source generation, compiler execution, Node startup, Wasm
module compilation, instantiation, warmup, assertions, hashing and reporting.
Runtime flags are recorded so matched runs use the same conditions. Warmup is
finite, not proof of settled JIT tiering. Correctness assertions are outside timing;
the guest's host calls and JS effect-array pushes remain inside timing for `effects`.
There are 26 timed workloads per guest run; `trap_order` is verify-only. The
[independent oracle and corpus table](integer-specialization.md#workloads-and-correctness-evidence)
define results, special inputs and effect/trap expectations. All modules declare
`env.mark: number -> number` and `main(x: number) -> number`, even when the import
or argument is unused. Retain bounded workloads and subprocess deadlines, not an
unbounded guest benchmark. No new dependencies, production instrumentation or
automatic installation is needed.

### Reproduction commands

Prerequisites are cached Rust dependencies, Node.js 24+ and Python 3 with only its
standard library for the recorder. Nothing is installed or fetched. Run from the
repository root with a **new** directory under `docs/benchmarks`; the recorder
rejects another working directory and any existing output directory, even empty:

```sh
python tests/support/record_integer_baseline.py docs/benchmarks/NEW-DIRECTORY
```

[`record_integer_baseline.py`](../tests/support/record_integer_baseline.py) records
commands/environment, revision/status and source/production hashes before and
after the run. It runs correctness and all-preset verification first, then two
compiler runs and two guest runs sequentially, with 120-second command deadlines.
It preserves raw output and failure metadata, checks input stability, and generates
`summary.md` / `summary.json` only after validating the run records and current
cross-preset source/module identity. The Node subprocess retains its 60-second
bound; neither deadline is guest fuel or process-tree cancellation.

Recorder safety/parsing/summary tests are available without measurements:

```sh
python tests/support/record_integer_baseline.py --self-test
```

Direct component commands, run sequentially:

```sh
cargo test --offline -p was-compiler --test integer_baselines
cargo test --offline -p was-compiler --test integer_baselines execute_integer_workloads -- --ignored --exact
cargo test --offline -p was-compiler --test integer_baselines record_integer_guest_timing -- --ignored --exact --nocapture --test-threads=1
cargo bench --offline -p was-compiler --bench integer_specialization -- --sample-count 100 --sample-size 1 --timer os --color never
```

`INTEGER_BASELINE_LEVEL` selects `None` (default), `Basic`, `Standard` or
`Aggressive` for both compiler benchmarks and guest timing. For example, prefix a
direct command with `INTEGER_BASELINE_LEVEL=Basic` in a compatible shell, or set
that environment variable before invoking the recorder. Verification always
executes all four presets, independent of this measurement selection.

When recording manually, run the Divan timing command twice and repeat the guest
recording command twice, not concurrently. Preserve both complete runs and raw
samples rather than only selected medians. Default tests skip the ignored execution/recording cases;
default Cargo success alone does not establish guest coverage. Report missing
prerequisites or failed commands without installing tools or inventing evidence.

### Recorded baseline — 2026-10-17

The baseline uses `None` on Windows 11 (OS build 10.0.26300), Intel Core Ultra 5
125H, rustc/Cargo 1.98.0, and Node 24.12.0 / V8 13.6.233.17-node.37. The work was
uncommitted atop `2889396f3f5f4878f7818e2946d56ba9fa62dde5`; revision/status logs and
before/after manifests identify the exact measured sources. All recording commands
succeeded and measured inputs remained unchanged. Verification checked 108
workload/preset pairs; source/module identities match all presets and both runs.

Representative medians below preserve the two runs separately. Compiler columns
are microseconds per compilation; guest columns are nanoseconds per main call.
Complete results, spread and raw guest samples are in the adjacent evidence files.

| Workload | Module bytes | Source compile µs (1 / 2) | Typed compile µs (1 / 2) | Guest ns (1 / 2) |
| --- | ---: | ---: | ---: | ---: |
| `literal_32` | 1156 | 146.3 / 155.7 | 28.64 / 60.84 | 95.3125 / 90.625 |
| `locals_32` | 1828 | 330.5 / 523.7 | 77.49 / 99.64 | 164.0625 / 121.09375 |
| `compare_32` | 4164 | 922.9 / 1515 | 139.8 / 297.9 | 403.90625 / 379.6875 |
| `negate_32` | 1668 | 296.6 / 617.9 | 68.69 / 141.9 | 166.40625 / 125 |
| `loop_local` | 1208 | 37.29 / 80.49 | 36.99 / 28.79 | 408.59375 / 296.875 |

These are usable byte/workload baselines and initial timing observations, **not a
stable performance budget**. Power, thermals and background activity were not
controlled. Compilation and guest timings both show run-to-run variation; small
scalar cases include substantial JS/Wasm call and timer overhead. V8 may optimize
constant guest expressions independently of compiler lowering, and finite warmup
does not establish steady-state tiering. A future comparison must remeasure matched
`None` and enabled runs on the same machine, preferably interleaving paired runs
under controlled conditions and increasing sampling when spread obscures a result.
These measurements alone cannot establish a benefit, regression, or release decision
for a pass that does not yet exist.

### Evidence and future matched comparison

Consult the [summary](benchmarks/integer-specialization/summary.md) and
[environment](benchmarks/integer-specialization/environment.json) for recorded
results under `docs/benchmarks/integer-specialization/`. The recorder writes
`hashes-before.json`, `hashes-after.json`, `stability.json`, command metadata and
raw `.txt` outputs, including `compiler-1/2` and `guest-1/2`. Node records include
CPU/OS, Node/V8, flags, source/module sizes and hashes, and raw per-call samples;
separate logs preserve Rust/Cargo versions and revision/dirty-tree identity.
The summary reports guest medians and byte/hash identity, leaving complete
compiler output unparsed. Power/thermal/background activity is uncontrolled;
interpret sampling spread rather than attributing all variation to compiler code.
This procedure itself asserts no baseline numbers or enabled-pass benefit.

Before transformations, record the unoptimized baseline and confirm current
cross-level byte parity. After separately authorized implementation, compare
matched `None` versus enabled presets using identical workloads and measurement
boundaries, machine, toolchains and runtime. Record any implementation revision
change; rerun matched pairs if harness inputs change. Cross-machine or historical
M0/M4 timings are not attributable integer-specialization comparisons. Compare
all presets against independent expected behavior, with current-source rebuilt
browser byte parity checked within each level, not byte identity across future
optimized and unoptimized levels.

Release needs demonstrated size **or** guest-time benefit, reported compilation
trade-offs, and explicit user acceptance of material regressions. Integer opcode
counts alone are insufficient, and no percentage threshold is invented in advance.
Inconclusive or regression-only evidence remains experimental pending a decision;
it does not establish release success or justify changing the default. Correctness,
diagnostics and adapter parity gates remain separate mandatory evidence.
