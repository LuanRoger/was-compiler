# Issue #39 review against 09dcb5c

The user confirmed `09dcb5c0ede3fe1740b35b0967fb66b80bfe0b05` as the fixed
point. Two independent, read-only reviewers inspected the staged implementation
against that commit before commit creation. The user's pre-existing `GLOSSARY.md`
and `docs/adr/0007-proven-integer-specialization.md` changes were excluded.

## Standards

The initial review found one documented testing-standard violation: the match,
pipeline and contract lowering suites applied independent decoded assertions only
to baseline output, while enabled presets received validation/determinism checks.
The review also identified a naming concern: `emit_numeric_operand` handled all
binary operand types, despite its numeric-only name.

Both findings were addressed:

- `match_lowering.rs`, `pipelines_lowering.rs` and `contracts_lowering.rs` now
  apply their independent decoded assertions to actual output at every preset.
  Explicit baseline/enabled expectations retain numeric conversions, f64 locals,
  contract ordering, direct-call bindings, match regions and helper layout checks.
- The emitter helper is now `emit_expr_as`, with a comment explaining that the
  requested numeric representation leaves nonnumeric result representations alone.

Focused lowering tests passed after these changes. The fixes were checked by the
integrating agent; no second independent review is claimed.

## Spec

The independent specification review identified zero actionable findings against
#39's literal/add-sub-only scope. Broader local/comparison/unary specialization
remains future work. The review did not independently rerun validation or timings.

## Final validation and provenance

After the review fixes, native Clippy, the core Wasm build, current-source browser
rebuild, all 25 browser smoke groups and browser-target Clippy passed. The combined
validation/test commands exceeded the 120-second tool bound; package-separated
reruns completed successfully:

```sh
cargo test --offline -p was-compiler -- --include-ignored
cargo test --offline -p was-compiler-cli -p was-compiler-browser -- --include-ignored
```

Together these ran all 1,261 workspace tests, with zero failures and zero ignored.
[Raw output](integer-specialization-validation-2026-10-17.txt) records both successful
commands. The [post-review manifest](benchmarks/integer-add-sub-39/post-review-source-state.json)
identifies final source hashes separately from the unchanged historical timing
measurements. The raw historical diff is stored losslessly as gzip to preserve its
original whitespace without introducing whitespace errors into the source diff;
the manifest records its decoded and archived hashes.

Review completion and correctness validation do not constitute release acceptance
of the documented material timing-regression signals. The subset remains
experimental and `None` remains the default.
