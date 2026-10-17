# Proven integer specialization: #38 contracts and experimental #39 subset

## Status and authorization

Issue #38 established bounded workloads, reproducible unoptimized measurements,
and the broader proof contract below. User-approved issue #39 implements only
exact parsed signed-i32 literals and proven addition/subtraction subtrees in
`Basic`, `Standard`, and `Aggressive`. This subset is **experimental**: measurements
show a module-size benefit, but noisy timing results include material regression
signals pending explicit user acceptance. [Correctness gates passed](#correctness-validation-2026-10-17);
Review against `09dcb5c` is complete. Neither validation nor
measurements establish release acceptance. `None` remains the default and
byte-compatibility baseline.

The approved #39 subset is a narrow scheduling exception ahead of deferred WP12,
not general WP13 authorization. Local-storage, comparison and unary-negation
specialization remain future work under #38's broader contract, not #39 support.
The [extension plan](was_language_extensions_plan.md#issue-38--proven-integer-specialization-baseline-exception)
reconciles this exception with the ordered work packages. The broader optimizer
candidates and WP13 entry gates remain unchanged.

## Implemented subset (#39)

- Literal eligibility uses the parsed binary64 value: exact signed-i32 integer,
  excluding negative zero. Decimal spelling alone neither qualifies nor excludes it.
- Add/sub specializes only when both children have proofs and conservative bounds
  keep every intermediate result within signed i32 without producing negative zero.
  Lower with `i32.const`, `i32.add` and `i32.sub`, not constant folding.
- Independent eligible subtrees specialize even within an unsupported or unsafe
  parent, converting exactly with `f64.convert_i32_s` at every unspecialized consumer.
  An unsupported parent therefore need not retain byte-identical children.
- All numeric locals remain `f64`, including never-reassigned and loop-local lets.
  Identifier/parameter/iteration reads, calls, loads and captured reads supply unknown
  facts. Comparisons and unary operations retain their existing lowering; so do
  multiplication, division and remainder. A source `-1` uses unary negation and is
  not itself a proven literal subtree, though its literal operand may specialize.
- `Compiler::compile` / `compile_source` select a private numeric policy from the
  preset. The typed-AST optimization boundary still borrows without rewriting.
  Direct `codegen::compile` and `codegen::compile_with_exports` retain baseline policy.
- Private per-body `Facts` in `crates/compiler/src/codegen/representation.rs` caches
  conservative proofs for borrowed immutable expression nodes, including unknown
  results. Node addresses are lookup keys only, never emitted-order authorities;
  each user/helper body owns its cache. No public AST annotation or new IR is added.
- Facts guide representation, not acceptance. Checked emission still validates every
  node, malformed literals/annotations, and unreachable/skipped children in existing
  order. Unknown facts fall back rather than introducing errors or runtime guards.

Language types, public AST, numeric local/storage layouts and all ABI boundaries
remain unchanged. #39 adds no branch/caller/contract refinement, constant folding,
algebraic rewrite, or preliminary baseline compilation.

## Broader first-pass representation contract (#38; future beyond #39)

The table below is the broader design envelope, **not** a list of implemented #39
features. Ordinary-local, comparison and negation specialization need later work.

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

- **Baseline:** `None` stays default and preserves baseline bytes, as do direct
  codegen entry points. #38's recorded cross-preset byte identity is historical.
- **Enabled subset:** `Basic`, `Standard`, and `Aggressive` use the same #39 policy
  and produce identical bytes to each other. They may differ from `None`. Programs
  with no eligible subtree retain existing bytes; unsupported parents alone do not
  establish this condition because their children can specialize independently.
- **Across all levels:** require semantic and diagnostic parity, including error
  variant, message, span, and first-error order, rather than cross-preset byte identity.
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
[`integer_measure.cjs`](../tests/support/integer_measure.cjs), not in the
representation proof analysis. Corpus case names describe workloads, not implemented
specialization: local/comparison/negation cases can shrink through literal children.

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
Require valid Wasm plus independent decoded representation checks; valid Wasm alone
does not prove numeric equivalence. Unsupported/unsafe operations must retain their
baseline paths, even when eligible child subtrees specialize. Differential execution
must compare both `None` and enabled levels against the independent oracle, not only each other.
Default Cargo success does not establish ignored Node execution coverage.

## Measurement and release decision

[Benchmark procedure](benchmarks.md#issue-38-integer-specialization-baseline-and-comparison)
owns commands, timing isolation, environment records, and the evidence handoff.
Keep compilation cost, emitted module size, and guest execution time separate.
The #38 baseline predates transformations; its then-no-op preset comparison is a
historical harness/parity check, not an optimization speedup.

For #39 and later comparisons, measure matched `None` and enabled configurations on
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

The [#39 matched measurement record](benchmarks/integer-add-sub-39/README.md)
measures this working-tree literal/add-sub subset, not the broader #38 design.
24 of 27 modules shrink and none grow; summed independent corpus module sizes
fall from 13,414 to 9,702 bytes (27.67%), not a production-application estimate.
Compilation medians show regressions; guest timings are noisy, with reversals and
material regression signals, not a demonstrated overall speedup. The record reports
108 successful independent workload/preset oracle checks and focused validation,
not complete diagnostics, forged-tree, workspace or current-source CLI/browser gates.
The separate correctness validation below passed; it does not broaden the historical
measurement record's scope. Material regressions still require explicit user
acceptance; this work remains experimental, with no release claim or default change.
Review against `09dcb5c` is complete.

## Correctness validation (2026-10-17)

Final integration correctness gates passed with the commands below, as reported by
the integration owner. The [raw final workspace test output](integer-specialization-validation-2026-10-17.txt)
records 1,261 tests passed, zero failed and zero ignored. Browser bindings were
rebuilt from current source before browser and full ignored-suite execution.

| Exact command (repository root) | Result |
| --- | --- |
| `cargo check --offline --workspace --all-targets` | Passed |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo build --offline -p was-compiler --lib --target wasm32-unknown-unknown` | Passed |
| `wasm-pack build crates/browser --target web --out-dir ../../pkg --out-name was_compiler --mode no-install --no-opt --offline` | Passed; current-source browser package rebuilt |
| `node tests/browser.cjs pkg` | Passed: 25 groups |
| `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` | Passed |
| `cargo test --offline -p was-compiler -- --include-ignored` | Passed |
| `cargo test --offline -p was-compiler-cli -p was-compiler-browser -- --include-ignored` | Passed; together 1,261 tests, zero failed/ignored |

These are correctness gates, not a release decision or acceptance of noisy material
timing regressions. Explicit user acceptance remains pending, and `None` remains
the default. Review against `09dcb5c` is complete.

## Review completion

The [two-axis review against `09dcb5c`](integer-specialization-review-39.md)
records the standards findings, fixes, zero spec findings, final split validation
and timeout/provenance details. Release acceptance remains a separate decision.
