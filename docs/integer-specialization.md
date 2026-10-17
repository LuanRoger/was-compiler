# Proven integer specialization: contracts and baseline (#38)

## Status and authorization

Issue #38 establishes bounded workloads, reproducible unoptimized measurements,
and maintained contracts **before production optimization**. All four optimization
levels currently remain no-ops with identical bytes and diagnostics. `None` remains
the default and byte-compatibility baseline. This document does not claim an
implemented pass, measured benefit, or completed release validation.

The approved scope is a narrow scheduling exception ahead of deferred WP12, not
general WP13 authorization. Production implementation needs separate authorization
and final user confirmation of the complete design before compiler implementation.
The [extension plan](was_language_extensions_plan.md#issue-38--proven-integer-specialization-baseline-exception)
reconciles this exception with the ordered work packages. The broader optimizer
candidates and WP13 entry gates remain unchanged.

## First-pass representation contract

The language retains one `number` type with IEEE-754 binary64 semantics. Signed
`i32` is a private representation only where static proof establishes identical
observable behavior; integer-looking syntax alone is insufficient.

| Eligible operation or binding | Required proof / boundary |
| --- | --- |
| Literal | Its parsed binary64 value is an exact signed-i32 integer and is not negative zero; spelling is not the proof. |
| Ordinary local declaration/read | Uncaptured and never reassigned, initialized by a proven value. An ordinary local declared inside a loop may qualify independently on each execution. |
| Addition / subtraction | Both operands are proven integers and conservative bounds keep every intermediate result within signed i32, preserving zero sign. |
| Numeric comparison | Both numeric operands are proven integers; preserve the existing boolean result. |
| Unary negation | Operand bounds exclude both zero and `i32::MIN`; negation must neither manufacture negative zero nor overflow. |

Track integrality, conservative bounds in `[-2147483648, 2147483647]`, and
negative-zero possibility. Use conservative analysis arithmetic so calculating
bounds cannot itself overflow. Bounds describe possible values, not folded
runtime results. Propagate facts through supported expressions and eligible
bindings only; derive no refinements from branches, callers, or contracts.

Unknown or unsupported values remain `f64`: parameters/call results, memory loads,
captured reads, mutable locals, loop-carried state, and iteration bindings are not
eligible ordinary local storage. Multiplication, division, remainder, and other
unsupported operations retain existing lowering. An eligible subtree may still
specialize independently and convert exactly back to `f64` at an unspecialized
consumer. Negative-zero uncertainty or unproven bounds requires fallback, not a
new error or a speculative runtime guard.

Function signatures and the host ABI remain unchanged (`number = f64`), as do
array elements, captured cells, and range records. Keep public AST/types unchanged.
Compiler orchestration selects policy; private per-function representation facts
may guide checked lowering. Preserve validation of mutable/forged typed input and
all unreachable/skipped children; reuse the existing checked boundary rather than
adding another IR, duplicate validator, or preliminary unoptimized compilation.
Constant folding and algebraic rewriting are outside this pass.

## Preset and compatibility gates

- **Now:** `None`, `Basic`, `Standard`, and `Aggressive` produce identical bytes and
  diagnostics. #38 changes no production output, defaults, ABI, or existing tests.
- **After separately authorized implementation:** `Basic`, `Standard`, and
  `Aggressive` receive the same initial pass. `None` stays default and preserves
  baseline bytes; programs receiving no specialization preserve existing bytes.
- **Across all levels:** require semantic and diagnostic parity, including error
  variant, message, span, and first-error order. Future optimized bytes may differ
  from `None`; that permission is not current implementation status.
- **Within each level:** library, CLI, and current-source rebuilt-browser outputs
  remain byte-identical for matching input, imports, exports, and options.

Preserve once-only left-to-right evaluation, effects, alias visibility, traps and
trap order, signed zero, NaN/infinities, and deterministic layout. Inspect actual
specialization via existing WAT output and independent decoded instruction/local
layout tests, not new public flags, browser methods, or AST annotations.

## Workloads and correctness evidence

The integrated harness comprises:

- `tests/support/integer_corpus.rs`: shared deterministic bounded source corpus.
- `tests/support/integer_measure.cjs`: independent execution verification and
  separately scoped guest timing.
- `crates/compiler/tests/integer_baselines.rs`: compiler-owned integration target.
- `crates/compiler/benches/integer_specialization.rs`: separate Divan target using
  existing development tooling, without changing the historical `compiler` target.

The corpus contains 27 bounded workloads; verification covers 108 workload/preset
pairs across all four levels. Independent worked expectations live in
[`integer_measure.cjs`](../tests/support/integer_measure.cjs), not in the future
proof analysis.

| Cases | Count | Coverage / independent expectation |
| --- | ---: | --- |
| `literal_{1,32}`, `locals_{1,32}` | 4 | Literal arithmetic and never-reassigned local chains; 143 per block, totals 143 / 4576. |
| `compare_{1,32}`, `negate_{1,32}` | 4 | Comparisons yield 3 / 96; restricted negation yields -143 / -4576. |
| `loop_local` | 1 | 32 bounded range yields, fresh ordinary locals per iteration; total 4576. |
| `fraction`, `negative_zero`, `negate_zero`, `positive_zero` | 4 | 3.75, -0, -0, +0 respectively. |
| `overflow`, `underflow_i32`, `negate_min` | 3 | Intermediate and final results leave i32: 2147483649 / -2147483650; minimum negation yields 2147483648. |
| `unknown`, `mutable`, `capture` | 3 | Unknown input yields 8.5, reassigned fractional local yields 3.5, shared capture observes reassignment and yields 144. |
| `multiply`, `divide`, `remainder` | 3 | Unsupported operations yield 63, 3.5, -1. |
| `nan`, `infinity`, `subnormal` | 3 | NaN, positive infinity, and `Number.MIN_VALUE`. |
| `effects`, `trap_order` | 2 | Result -6 with marks `[3, 9]`; bounds trap after mark `[1]`, before RHS mark 2. |

Every module declares `env.mark: number -> number`, unused except by effect cases,
and exports `main(x: number) -> number`. The harness supplies the host import even
when unused. Ordinary calls use `x = 7.5`; `unknown` additionally checks -0,
2147483647, both infinities, NaN and `Number.MIN_VALUE`. Host registration and
measurement introduce no production-output change. Subprocess deadlines are not
guest fuel.

Record independent expected values, effect sequences, and traps before inspecting
optimized output. Use `Object.is` for signed zero and `Number.isNaN` for NaN.
Require valid Wasm plus independent decoded representation checks when the pass
lands; valid Wasm alone does not prove numeric equivalence. Unsupported/unsafe
controls must retain their baseline paths. Differential execution must compare
both `None` and enabled levels against the independent oracle, not only each other.
Default Cargo success does not establish ignored Node execution coverage.

## Measurement and release decision

[Benchmark procedure](benchmarks.md#issue-38-integer-specialization-baseline-and-comparison)
owns commands, timing isolation, environment records, and the evidence handoff.
Keep compilation cost, emitted module size, and guest execution time separate.
The baseline must be recorded before transformations; comparing today's no-op
presets is a harness/parity check, not an optimization speedup.

For the future comparison, measure matched `None` and enabled configurations on
identical corpus inputs, toolchains, runtime, machine, and harness protocol.
Repeat measurements and preserve raw samples, full command output, source/module
hashes, environment, revision and dirty-tree identity. Record implementation
revision differences explicitly; changes to workloads or harnesses require fresh
matched measurements rather than comparison with incompatible historical data.

Shipping requires a demonstrated module-size **or** guest-time benefit, with
compilation trade-offs and all measured regressions reported. More integer
instructions alone is not a benefit. There is no invented percentage threshold;
material regressions need explicit user acceptance. Inconclusive or regression-only
evidence keeps the work experimental pending a decision, not release success or
permission to change the default. Correctness and diagnostic/adapter gates remain
mandatory even when measurements show a benefit.

## Evidence status

The unoptimized baseline was recorded on 2026-10-17: two compiler runs, two guest
runs, 108 verification cases and matching before/after input hashes. See the
[measurement summary](benchmarks/integer-specialization/summary.md),
[environment record](benchmarks/integer-specialization/environment.json), and
[interpretation](benchmarks.md#recorded-baseline--2026-10-17). Timings show substantial
run-to-run variation, not an enabled optimization benefit or release result.
Historical benchmark records retain their original scope and measurement boundaries.
