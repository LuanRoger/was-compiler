# Proven integer specialization without a new numeric language type

The language retains a single `number` type with IEEE-754 binary64 semantics.
Integer specialization may use signed `i32` internally only where static proofs
establish equivalent observable behavior; unknown or unsupported cases retain
`f64`. Integer-looking syntax alone is not proof. New source integer types and
runtime guarded specialization are outside this decision.

A narrowly scoped exception to the WP13 entry gate permits this numeric work ahead
of WP12, not general optimizer development. The initial scope is numeric expressions
and uncaptured ordinary locals within function bodies. Function signatures, captured
cell storage, array elements, and range records retain their existing contracts.
This trades broader opportunities for a smaller proof surface and preserves host
compatibility.

Correctness gates include independent differential results, effects, traps, and
numeric edge cases. Reproducible execution-time, binary-size, and compilation-cost
measurements are required; more integer instructions alone are not evidence of a
benefit. `OptimizationLevel::None` remains the unoptimized compatibility baseline,
and default enablement requires measurements first.

## First-pass implementation scope

Specialize eligible expression subtrees and uncaptured ordinary locals with no
subsequent writes. Mutable locals and loop-carried state retain `f64`; an ordinary
local declared inside a loop may qualify if it is not reassigned. Supported numeric
operations are exact signed-i32 literals/local reads, addition and subtraction with
proof for every intermediate result, comparisons with proven integer operands, and
negation whose operand excludes zero and signed-i32 minimum. Multiplication,
division, and remainder retain existing floating-point lowering. Integer results
convert to `f64` at unspecialized consumers and existing ABI/storage boundaries.

Keep the public typed AST and language types unchanged. Compiler orchestration
selects the optimization policy; private per-function representation facts guide
checked lowering. Conservative proof analysis introduces no new source errors and
must not bypass backend validation of mutable public typed input, unreachable code,
or skipped children. Do not add another IR, duplicate validator, or preliminary
unoptimized compilation.

`Basic`, `Standard`, and `Aggressive` receive the same initial pass. `None` remains
default and preserves baseline bytes. Across levels require equivalent semantics
and diagnostics, not identical bytes. Within each level preserve library/CLI/browser
byte parity; programs receiving no specialization retain existing bytes.

## Proof and inspection boundaries

Track integrality, conservative signed-i32 bounds, and negative-zero possibility.
Literal eligibility follows its parsed binary64 value, not its spelling. Propagate
facts through supported expressions and eligible local bindings; do not refine
from branches, contracts, or callers. Calls, memory loads, captured reads, and
unsupported expressions supply unknown numeric facts. Eligible subexpressions may
still specialize independently, converting back at unspecialized consumers.
Do not add constant folding or algebraic rewriting.

Inspect actual specialization through existing WAT output and focused independent
decoded instruction/local-layout tests. No new reporting flags, browser methods,
or public representation annotations are part of this first pass.

## Release evidence

Compare baseline and optimized execution against independent expected results,
including signed zero, overflow boundaries, effects, and traps. Validate emitted
Wasm and independently decode instructions and local layouts. Negative controls
must show that unsupported or unsafe cases retain their baseline paths.

Repeat measurements of compilation cost, binary size, and guest execution time on
bounded representative workloads. Shipping requires a demonstrated size or execution
time benefit, not merely integer instruction counts. Do not invent a percentage
threshold before measurements: report trade-offs, and obtain explicit user
acceptance for material regressions. Only regressions or inconclusive benefit keep
the work experimental pending a decision; they do not justify declaring release
success or changing the default.

These decisions were agreed during the design interview on 2026-10-17. The complete
design still requires final user confirmation before compiler implementation.
The existing no-op implementation and its parity requirements remain current
behavior until that implementation is approved.
