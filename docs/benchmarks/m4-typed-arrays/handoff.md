# Issue #37 M4 measurement and optimization handoff

## Scope and evidence

This completes the comparative-measurement/optimization-handoff portion of M4,
not #37's parent release-validation gate. It implements no optimization, allocator
change, production profiling export, dependency or manifest change. The new test
and support files use the already approved public source/typed compilation,
encoded-module and execution seams. Original M0 files/evidence are untouched.

The [recorder](../../../tests/support/record_memory_m4.py) ran on 2026-10-15 UTC,
23:58:17–23:58:41. See [environment.json](environment.json) for exact commands,
working-tree status, revision, toolchains, original input/evidence hashes, ten
measurement/support file hashes and 77 current production/source/manifest hashes.
The tree is dirty atop `e720e2b0330ae140bc0d2436358858d44ab1114f`; `HEAD` alone
does not identify M1–M3 or adapter/runtime implementation. [stability.json](stability.json)
confirms those inputs and production files stayed unchanged during measurement.

CPU/OS/runtime match M0's recorded machine: Intel Core Ultra 5 125H, 18 logical
CPUs, Windows `10.0.26300`, x64, 16,730,726,400 physical-memory bytes; rustc/Cargo
1.98.0, LLVM 22.1.8, Node 24.12.0, V8 `13.6.233.17-node.37`. The recorder uses
Python 3.13.14 stdlib only. Test callers use Rust's debug profile; emitted guest
code uses default compiler options, with all-four-level byte parity tested.
Compiler timings separately use Cargo's optimized bench profile.

## What is established

[comparison.md](comparison.md) has complete human-readable tables;
[comparison.json](comparison.json) retains hashes, sizes, allocations, pages and
medians for independent analysis. All samples remain in the raw logs.

1. **Legacy output identity:** all 17 original M0 workload source hashes and
   generated-module hashes match exactly after M1–M3. Module sizes, aligned bump
   consumption, results and initial/final page counts also match. This is stronger
   than successful Wasm validation or equal module sizes. It applies to these
   workloads, not all possible source programs.
2. **Existing compiler corpus:** all nine source/module size pairs match M0;
   all 45 compiler timing cases ran twice with unchanged harness/corpus hashes.
   M0 recorded module sizes, not module hashes, for that separate compiler corpus;
   do not extend the 17-workload byte-identity result to those nine modules.
3. **No attributable timing improvement:** two current guest runs and two current
   compiler runs are recorded. Most original guest medians are higher than M0's,
   despite identical Wasm and identical recorded runtime versions. Compiler run 1
   versus run 2 also varies strongly (e.g. `source_compile/calls_64` 2.022/1.347 ms;
   M0 1.282 ms). CPU power, thermal state, scheduling/background load and V8 tiering
   were not controlled. A finite warmup is not a settled-tier guarantee. These are
   observed timing differences, not evidence of an emitted-code regression or a
   speedup attributable to this implementation. One historical compiler run cannot
   provide a repeated historical compiler distribution. Do not normalize away the
   variation, pool it as a stable budget, or invent a percentage gate.
4. **New types establish a baseline, not a pre/post implementation comparison.**
   There was no M0 bool/string-array implementation to time. Matched source
   controls below hold counts/operation shapes constant, not semantic types or
   validation costs. Width effects are not optimizer wins.

### Matched construction and storage observations

Each `construct_N` call constructs 128 fresh arrays. Each `page_8` call constructs
8,192 fresh eight-element arrays. Headers remain 8 bytes; whole allocations round
to 8 bytes. `bool[]` slots are 1 byte, `string[]` slots are 4 bytes, and numeric
slots remain 8 bytes. These are fixed-length arrays, not growable lists.

| Observation | number[] | bool[] | string[] |
| --- | ---: | ---: | ---: |
| construct_0 aligned bytes/call | 1,024 | 1,024 | 1,024 |
| construct_1 aligned bytes/call | 2,048 | 2,048 | 2,048 |
| construct_2 aligned bytes/call | 3,072 | 2,048 | 2,048 |
| construct_3 aligned bytes/call | 4,096 | 2,048 | 3,072 |
| construct_7 aligned bytes/call | 8,192 | 2,048 | 5,120 |
| construct_8 aligned bytes/call | 9,216 | 2,048 | 5,120 |
| construct_9 aligned bytes/call | 10,240 | 3,072 | 6,144 |
| page_8 aligned bytes/call | 589,824 | 131,072 | 327,680 |
| page_8 initial → final pages | 1 → 10 | 1 → 3 | 1 → 6 |
| capture_shared aligned bytes/call | 48 | 48 | 48 |
| capture_iteration aligned bytes/call | 8,232 | 8,208 | 8,216 |
| construct_0 module bytes | 776 | 1,051 | 1,053 |
| construct_8 module bytes | 890 | 1,124 | 1,143 |

The smaller bool/string outer allocations follow **type widths**, not allocation
elimination or optimized numeric storage. Empty/singleton rounded reservations
are identical across types; capture storage remains uniform and does not shrink
with payload width. String literals here are static/interned (9 UTF-8 bytes per
literal); repeated string slots share immutable payloads. Their static data
contributes to module size/initial layout, not per-call bump consumption. These
figures do not price host-created or dynamically encoded per-occurrence strings.

Both marker and capacity recordings check independent worked expectations.
Separate descriptor-byte fixtures verify 8-byte `f64` slots, canonical one-byte
booleans, four-byte string handles, UTF-8 contents and repeated immutable-string
handle sharing. They add no probe function to measured modules.

### Execution observations for follow-up selection

All values below are median nanoseconds per **main call**, with two current runs;
these are not per-operation costs or claimed speedup ratios.

| Matched case | number[] run 1 / 2 | bool[] run 1 / 2 | string[] run 1 / 2 |
| --- | ---: | ---: | ---: |
| equality_length | 5,412.5 / 6,962.5 | 4,937.5 / 3,775 | 4,087.5 / 3,950 |
| equality_early | 6,662.5 / 5,525 | 6,400 / 5,650 | 13,387.5 / 12,775 |
| equality_late | 9,912.5 / 9,612.5 | 20,075 / 18,075 | 43,737.5 / 29,862.5 |
| equality_equal | 10,350 / 6,962.5 | 20,100 / 18,275 | 29,887.5 / 32,312.5 |
| capture_iteration | 26,300 / 27,662.5 | 36,637.5 / 27,875 | 47,612.5 / 43,225 |

Equality cases compare eight-element arrays 128 times, except the one-element
right operand in length exits. A string mismatch compares equal-length UTF-8
payloads differing at the last byte. Equal static strings share interned handles,
but content equality still consumes/validates values. The observed late/equal
versus early/length work motivates targeted profiling, not removing checks based
on an assumed cost model. Raw ranges/outliers are preserved; string late versus
equal ordering is not stable enough to support a finer conclusion.

## Ranked follow-ups — proposals only, separate WP13 authorization required

Ranking prioritizes demonstrated allocation pressure, then repeated-operation
work, then module footprint. These synthetic cases establish leverage, not real
application frequency, achievable savings, or permission to change contracts.
Before any implementation, approve the specific optimizer/parity contract and
proof/test matrix. ADRs [0001](../../adr/0001-private-range-representation.md),
[0004](../../adr/0004-local-callable-ownership-and-storage.md),
[0005](../../adr/0005-compiler-and-adapter-workspace-boundaries.md) and
[0006](../../adr/0006-typed-array-layout-and-lazy-element-validation.md) still apply.

### 1. Non-escaping loop-local literal scalar replacement / allocation avoidance

**Evidence:** `page_8` consumes 589,824 / 131,072 / 327,680 aligned bytes in one
call; `construct_0` alone consumes 1,024 bytes for empty arrays. The original M0
page workload is still 131,072 bytes and three pages: numeric allocation pressure
has not improved. Candidate first experiment: a compiler-proven non-escaping
literal used only for length/local scalar operations. No result here establishes
that any allocation can legally be removed.

- **Aliasing:** prove the handle is not returned, passed to a host, shared through
  parameters/assignments/captures, compared by host-visible identity or read through
  another live binding. Preserve fresh storage for every observable literal.
- **Lifetime:** prove uses are invocation-local across recursion and host reentry;
  do not invalidate previously returned arrays/string offsets. This is not general
  heap rewind or reclamation.
- **Host effects:** preserve once-only element evaluation and written effects in
  order; hosts can observe memory capacity, raw handles and allocator positions.
- **Trap order:** storage reservation precedes element effects; removing/moving
  allocation may remove an OOM trap or alter which effects complete. An explicit
  decision about observable allocation/failure parity is required, not an implicit
  assumption that a dead handle makes allocation unobservable.
- **First gate:** decoded allocation/effect-order assertions plus runtime escape,
  alias, reentry, allocator-pressure/trap and all-optimization-level tests; measure
  the same sources again only after that contract is approved.

### 2. Non-escaping capture cell/environment specialization

**Evidence:** matched iteration captures consume 8,232 / 8,208 / 8,216 bytes for
one initial four-element array and 512 yielded cells/environments. The common
8,192-byte capture component dominates the 16/24/40-byte initial arrays. Shared
captures cost 48 bytes for every type. Smaller array widths therefore do not solve
capture pressure. Candidates: inlining, proven stack-local cells, or fused/packed
private environments; the measurements do not identify which is safe or fastest.

- **Aliasing:** retain original owner-qualified shared binding identity across
  creator/helper writes, nested forwarding and live callable dependencies. No
  snapshots of mutable captures; read-only iteration captures stay read-only.
- **Lifetime:** each yield and recursive/reentrant invocation needs its own live
  context. Skipped declarations currently still have eager iteration-cell
  promotion. Retained array/string values must survive later calls and traps.
- **Host effects:** direct helpers can invoke hosts or reenter creators. Do not
  reuse a still-live cell/environment just because callables are not first-class.
- **Trap order:** eager cell and executed-environment allocation are observable
  failure points. Packing/lazy promotion changes padding, allocation count, memory
  growth and potential traps; approve any parity exception explicitly.
- **First gate:** closure #30–#34 lifetime/alias/forwarding/trap suites plus typed
  bool/string capture fixtures, independently decoded layouts and repeated runs.

### 3. Proven redundant structural-check / checked-loop specialization

**Evidence:** no per-iteration allocation occurs in ordinary matched iteration
(40/16/24 bytes once), while late/equal equality and captured traversals execute
substantially more work than length exits in these samples. String late equality
medians are 43,737.5/29,862.5 ns versus 4,087.5/3,950 ns for length exits. This
supports profiling address/outer-validation/consumption paths, not assigning the
whole difference to redundant checks. `codegen/array.rs` currently separates
structural helpers, width-specific access and consumed-value validation.

- **Aliasing:** keep live alias mutation visible; cache no element snapshots.
  Ordinary replacement must retain its checked pre-RHS destination and must not
  consume the old value. Numeric compounds retain their pre-RHS old value.
- **Lifetime:** saved offsets, invocation contexts and source-reassignment versus
  cached iterable-handle behavior must survive growth and nested/reentrant calls.
- **Host effects:** hoisting/caching needs a proof of no intervening invalidation
  or host/reentry effect; trusted-host assumptions are not a blanket license to
  suppress the existing forged-input/backend checks.
- **Trap order:** validate both complete outer storages before equality length
  exits; consume left/right pairs in order and stop at the first mismatch. Do not
  eagerly scan skipped elements, hoist their traps, omit bool/string consumption,
  or add an identity shortcut (including numeric NaN self-comparison).
- **First gate:** profile a longer bounded scaling series, then one narrow
  no-host/no-invalidation region with negative/forged/early-exit/effect tests.

### 4. New-type helper reachability specialization for module footprint

**Evidence:** empty construction modules are 776 / 1,051 / 1,053 bytes; bool/string
add 275/277 bytes even with no elements. Inspection of `memory.rs` confirms
per-type groups of five boolean and four string-array helpers are appended when
the type is observed. These deltas include all emitted differences, not a promise
that all 275/277 bytes are removable. Numeric-only M0 modules are already identical.

- **Aliasing:** reachability must include all helpers required by typed places,
  captures, internal signatures and live callable dependencies, not just exports.
- **Lifetime:** retain every reachable allocator/validation/capture operation;
  pruning a helper must not change any surviving instance's storage behavior.
- **Host effects:** preserve import snapshots, required `memory`/`__alloc` exports,
  deterministic helper/callee indexing and static-string discovery order.
- **Trap order:** never prune source/body validation or unreachable/skipped
  malformed typed-tree checks. Only prove an emitted helper unreachable; do not
  remove semantic checking or reachable validation calls.
- **First gate:** propose a deliberate optimized-module compatibility policy.
  Current #37 forbids general legacy helper pruning; keep default/None legacy
  bytes unchanged unless separately approved, with golden/decoded/ABI validation.

## Deferred / not inferred from this evidence

No garbage collection, reference counting, general heap rewind, bit-packed bool
arrays, allocation-free ranges, automatic marshaling or caller-view lifetime
change is proposed for this issue. The allocator is still **monotonic**, and
repeated calls on a long-lived instance can exhaust storage. Capacity pages are
not live bytes, RSS or a reclamation measurement.

Compiler timings motivate a controlled repeated/profiled follow-up before
selecting any compiler-planning optimization; this noisy comparison does not
identify a hot compiler function. Browser execution, Node marshaling/copy budgets,
dynamic host string allocation, application traffic and stable performance budgets
are not measured here. The parent still owns current-source rebuilt-browser,
full ignored workspace/runtime and release-validation evidence. Do not close those
gates based on this handoff.
