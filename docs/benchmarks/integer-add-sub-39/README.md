# Issue #39: matched exact-i32 literal/add-sub evidence

Experimental measurement evidence recorded on 2026-10-17, **not a release gate
result or authorization to change the default**. This measures the working-tree
subset: exact-i32 literals and proven add/sub expression trees specialize in
`Basic`, `Standard`, and `Aggressive`; local storage remains `f64`. It does not
measure an implemented local-storage/comparison/negation specialization pass.

## Evidence and provenance

- [Complete matched table](collection-1/summary.md) and
  [machine-readable results](collection-1/summary.json): all 27 corpus cases,
  both presets, both runs; compiler minima/maxima/means/medians in JSON.
- [Environment](collection-1/environment.json), [Rust](collection-1/rustc.txt),
  [Cargo](collection-1/cargo.txt), [Node](collection-1/node.txt),
  [revision](collection-1/git-head.txt), [initial dirty status](collection-1/git-status-before.txt),
  [initial tracked diff](collection-1/git-diff-before.txt.gz), and
  [end-of-collection dirty status](collection-1/git-status-after.txt).
- [Before hashes](collection-1/hashes-before.json),
  [after hashes](collection-1/hashes-after.json), and
  [stability](collection-1/stability.json). Every subprocess also has before/after
  input manifests. Production and measured inputs remained unchanged throughout
  collection; no instability/retry was observed. The later
  [initial handoff check](handoff-inputs.json) also matched the measured inputs
  at that time; it does **not** describe the subsequently formatted final tree. Manifests
  include untracked production `.rs` files, including the new representation
  implementation.
- [All-preset oracle verification](collection-1/verify.txt) and
  [focused native contract test](collection-1/default-test.txt).
- Raw guest records: [None 1](collection-1/guest-1-None.txt),
  [Basic 1](collection-1/guest-1-Basic.txt), [Basic 2](collection-1/guest-2-Basic.txt),
  [None 2](collection-1/guest-2-None.txt). Each contains source/module SHA-256,
  exact byte lengths, environment and **all 20 raw samples** for each of the 26
  timed cases. `trap_order` is verify-only.
- Complete compiler reports: [None 1](collection-1/compiler-1-None.txt),
  [Basic 1](collection-1/compiler-1-Basic.txt), [Basic 2](collection-1/compiler-2-Basic.txt),
  [None 2](collection-1/compiler-2-None.txt). Divan 0.1.21 exposes aggregate timings,
  not individual compiler samples through its public CLI. Reports retain all
  available aggregates, 100 samples/iterations per case and original rounded
  precision; no raw compiler samples are claimed. Each report covers 27 source
  and 27 typed compilations. Adjacent `.json` files preserve commands, timestamps,
  120-second deadlines, exit status and timeout status.

Collection ran on Windows 11 build 10.0.26300, Intel Core Ultra 5 125H (18 logical
CPUs), rustc/Cargo 1.98.0, Python 3.13.14, Node 24.12.0 / V8
13.6.233.17-node.37, with no Node flags. Git revision plus dirty-tree records and
file hashes identify the measured tree; the commit alone does not identify it.
Nothing was fetched, installed, staged or committed by this measurement task.

## Final integration reconciliation (2026-10-17)

The new [final source/status snapshot](final-source-state-2026-10-17.json)
records final SHA-256 manifests, revision and dirty status separately from the
immutable timed collection and initial handoff. **Final source hashes are not
identical to the timed files:** 14 entries in the recorder's `production` group
changed; all `measurement` entries still match. That historical group includes
Rust unit-test files under `src/`, not just shipping implementation. The snapshot
lists every changed path with its timed and final hash, and additionally hashes
final integration tests and the correctness validation log.

The integration owner reports only mechanical rustfmt changes in the edited
implementation, corrected CLI `-O` help (no longer claiming presets are no-ops),
and expanded test expectations, with no numeric algorithm changes. Hashes alone
do not establish semantic equivalence. No timing rerun was requested or performed
for these formatting/help/test-only changes: the measurements, module hashes and
size results remain observations of the **timed tree**, not a new measurement of
the final files. The final snapshot is provenance reconciliation, not a second
performance run or a confirmed code-review fixed point.

Separate [final correctness validation](../../integer-specialization.md#correctness-validation-2026-10-17)
reports successful current-source browser rebuild, 25 browser smoke groups,
workspace/native and browser Clippy, and the core Wasm build. The
[raw workspace log](../../integer-specialization-validation-2026-10-17.txt)
contains 1,261 passed, zero failed and zero ignored tests, including the previously
ignored cases. These gates were run by the integration owner, not rerun by this
documentation task. They supplement rather than retroactively broaden the timing
collection's 108-case oracle checks. **Review against `09dcb5c` is complete; release acceptance of material timing regressions remains pending.**
No release/default-change claim is made.

The final snapshot also records the bytecode/cache audit. No `__pycache__`
directories were present in either recorder location and no tracked `.pyc` or
`__pycache__` entries were found; this follow-up uses `python -B` to avoid generating
them. No generated bytecode is included in this evidence.

## Matched protocol

The [new recorder](record_matched.py) imports parsing/recording helpers from the
unchanged historical `tests/support/record_integer_baseline.py`, but does **not**
use its cross-level module-identity requirement. No historical evidence was edited.
Sources must match across all levels; modules must match within each level and
across repetitions. `Basic`/`Standard`/`Aggressive` identity is separately checked.
Every `None` source/module hash also matched the historical #38 record; the timing
comparison uses **new matched runs**, never historical timing values.

From repository root, with cached tooling/dependencies only:

```sh
python docs/benchmarks/integer-add-sub-39/record_matched.py --self-test
python tests/support/record_integer_baseline.py --self-test
python docs/benchmarks/integer-add-sub-39/record_matched.py docs/benchmarks/integer-add-sub-39/NEW-COLLECTION
```

Output must be a new child directory; existing evidence is refused. The new
recorder's three self-tests cover Divan parsing/rejection, changed-input detection,
and within-level versus enabled-preset identity. The historical recorder's four
self-tests also passed; outputs are preserved in
[the new recorder log](recorder-self-test.txt) and
[the historical recorder log](historical-recorder-self-test.txt).
Collection commands all succeeded with offline Cargo and
bounded subprocesses. Hash instability stops collection and prevents a valid
summary; request a stable production window before retrying in a new directory.
Before/after hashes detect observed changes, not a transient edit-and-revert or
proof that no unrelated CPU-consuming tasks ran.

Execution order was fully sequential, with no measurement commands in parallel:

1. Environment/status capture, focused native contract test, all-preset verification.
2. Run 1: compiler None → Basic, then guest None → Basic.
3. Run 2: compiler Basic → None, then guest Basic → None.
4. Final status and input hashes, identity checks, summary generation.

Compilation uses the existing public-API Divan harness with `--sample-count 100
--sample-size 1 --timer os --color never`: fresh compiler/host registration and
output destruction inside timing; source/typed prerequisites and validation
outside. Guest timing uses the unchanged 128 warmups, 20 samples × 64 calls,
fresh instance per sample and shared compiled module/JIT state. Guest compilation,
instantiation, assertions and reporting are outside timing; JS/Wasm calls, batch
loop overhead and host mark bookkeeping remain inside. Node retains its
60-second subprocess deadline. Deadlines are not guest fuel or process-tree
cancellation. No browser rebuild or release suite was run for this evidence task.

## Findings and trade-offs

**Deterministic size benefit:** 24/27 modules shrink and none grow. The three
byte-identical cases are `fraction`, `negate_min`, and `subnormal`. The sum of the
27 independent module sizes is 13,414 → 9,702 bytes (−3,712, −27.67%); this is a
synthetic corpus aggregate, not a production application size estimate.

| Workload | None → Basic bytes | Reduction |
| --- | ---: | ---: |
| `literal_32` | 1,156 → 542 | 53.11% |
| `locals_32` | 1,828 → 1,278 | 30.09% |
| `compare_32` | 4,164 → 2,558 | 38.57% |
| `negate_32` | 1,668 → 1,086 | 34.89% |
| `loop_local` | 1,208 → 1,179 | 2.40% |

Case names describe source workloads, not proof of specialized local storage,
comparisons or negation. Fallback controls can shrink because eligible literal
children specialize and convert back to `f64`; this does not establish integer
lowering of multiplication/division/remainder, captured reads, mutable reads,
unsafe arithmetic or negative-zero-producing operations.

**Compilation trade-off:** selected Basic typed-compilation median changes in
runs 1 / 2 were `literal_32` +23.22% / +58.00%, `locals_32` +7.21% / +75.82%,
`compare_32` +11.00% / +68.66%, `negate_32` +60.82% / +87.72%, and `loop_local`
+157.27% / +65.87%. Source-to-Wasm medians were also higher in both repetitions
for `locals_32` (+8.34% / +21.70%), `negate_32` (+13.35% / +29.77%) and
`loop_local` (+24.20% / +42.20%). Other directions reverse between runs; e.g.
`literal_32` source compilation +8.51% / −27.98%. These are observed ratios of
rounded medians, **not statistically established pass overhead**. The full table
reports every workload, including fallback cases and observed improvements.

**Guest time is inconclusive, not a demonstrated speedup:** for `literal_32`,
None medians were 85.9375 / 259.375 ns versus Basic 149.21875 / 89.84375 ns.
`locals_32` changed 385.9375 / 383.59375 → 190.625 / 384.375 ns. Reversals and
substantial run-to-run changes prevent a stable overall performance conclusion.
Observed guest medians were higher for Basic in both runs for `compare_32`,
`negate_min`, `mutable`, `capture`, and `multiply`; none are omitted from the full
table. Particularly, `capture` was 129.6875 / 117.96875 → 221.875 / 260.15625 ns
(+71.08% / +120.53%), and `multiply` was 137.5 / 82.8125 → 140.625 / 151.5625 ns
(+2.27% / +83.02%). These are follow-up regression signals, not an attribution to
a specific lowering change or acceptance of the regressions.

Power/thermal state and background activity were uncontrolled on this shared
machine; stable file hashes do not imply an idle CPU. Short constant workloads
are sensitive to call/timer overhead and independent V8 optimization; finite
warmup does not establish steady-state JIT tiering. The whole collection lasted
about 14 seconds, so repetitions are nearby observations rather than independent
controlled sessions. Increasing controlled sampling is appropriate before making
a timing claim or accepting material regressions.

**Correctness scope:** 108 workload/preset pairs passed the independent value,
signed-zero/NaN, special-input, host-effect and trap-order oracle; all measured
modules also passed the existing harness's Wasm validation. Enabled modules match
each other, and `None` preserves historical corpus bytes. This does not establish
complete diagnostics, forged-tree, workspace, CLI/browser parity, or release gates.
The size result is positive evidence for the measured subset. The separate final
integration correctness gates described above subsequently passed; they are not
performance remeasurements. Review against `09dcb5c` is complete, and shipping requires explicit acceptance of material regressions.
`None` remains the default; no release or acceptance claim is made.
