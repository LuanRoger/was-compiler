# WAS Language Extensions — Unified Design and Implementation Specification

**Status:** Design baseline with the 13 approved decisions and implementation clarifications (WP1–WP4 and partial WP5 implemented: compound assignments, while transfers, block/if values, internal Never, statement/value loops, and range construction/internal calls, numeric membership, and range/array statement for; adapter/interaction release gates pending and remaining extensions proposed)
**Version:** 0.3 — 9 October 2026 (implementation clarification update)  
**Target:** Core WebAssembly, initially the MVP-compatible instruction set already used by `was-compiler`, with optional optimizations gated by Wasm features.  
**Repository:** https://github.com/LuanRoger/was-compiler

> **Normative language.** **MUST**, **SHOULD**, and **MAY** describe requirements, strong recommendations, and optional implementation choices. This document establishes a coherent *proposed* semantic model. Decisions tagged **Open decision** need confirmation before being frozen as language guarantees. Compound assignments (§4/WP1), bare `while` transfers (§7/WP2), block/if values with internal Never (§§3.2, 8.1/WP3), and statement/value loops (§7/WP4) are implemented. Range construction, internal calls, exact numeric membership (§5/partial WP5), and range/array statement `for` (§6/issues #4–#5) are also implemented; remaining adapter/interaction release gates are pending. Other extension examples remain proposed `.was` syntax, not claims about the present compiler.

The [concrete implementation plan](was_language_extensions_plan.md) maps this
specification to ordered work packages, repository files, acceptance tests, and
validation gates. This revision retains the approved v0.2 direction, including
shared mutable captures and local-only closures; it clarifies implementation
safety and corrects the current-language baseline. Compound assignments (§4/WP1)
and bare `while` transfers (§7/WP2), followed by block/if values and internal Never
(§§3.2, 8.1/WP3) and statement/value loops (§7/WP4), are implemented;
WP5's construction/internal-call, numeric membership, range-for, and array-for checkpoints are implemented;
the rest of WP5 and WP6 onward remain proposed.

## 1. Goals and non-goals

### 1.1 Goals

1. Improve developer ergonomics without exposing Core Wasm's stack machine, block depths, tables, or linear-memory representation.
2. Make expressions, iteration, functional composition, and API contracts first-class language concepts.
3. Preserve the current `number` abstraction: a semantically IEEE-754 binary64 value. `i32`/`i64` are potential *internal representations*, never inferred source-language types.
4. Define evaluation order, errors, scoping, and observable results precisely enough for independent parser, analyzer, interpreter/reference-model, and backend implementations.
5. Enable effective Core Wasm lowering: structured control flow, direct known-target calls for locally bound closures, stack-valued branches, and optional static analysis. Indirect closure calls are deferred.
6. Allow incremental implementation: basic control flow must not require a closure runtime, and early pipelines use statically resolved named functions before local closures are supported.
7. Preserve existing host ABI contracts unless an explicit versioned extension is introduced.

### 1.2 Non-goals

- Introducing public `i32`, `f64`, generics, classes, or a general-purpose trait system.
- Implementing a garbage collector as a prerequisite for the first six features.
- Automatically specializing every numeric expression to `i32`.
- Requiring Wasm GC, reference-types/function-references, exceptions, tail calls, WASI, or the Component Model for core semantics.
- Promising that pipelines/closures are allocation-free or SIMD-vectorized in every program.
- General first-class functions: callable parameters, callable return values, arrays of functions, and arbitrary indirect callable dispatch are deferred. Local closure bindings are a restricted feature, not general first-class functions.
- Changing current array assignment/aliasing semantics.

## 2. Existing language assumptions and compatibility

The current compiler supports `fn` with required parameter/return annotations,
annotated `let name: type = value;` and inferred `let name = value;` declarations,
reassignment, numeric compound assignments, `if`, `while`, bare `break;` /
`continue;`, range/array statement `for`, `return`, statement/value `loop`, value breaks in loop expressions,
tail-valued blocks, value-context `if` / `else if`, named calls, and `number`,
`bool`, `string`, `number[]`, `range`, and
`void`. Every local requires an initializer and retains a fixed type. `//` line comments and non-nesting `/* ... */` block comments are implemented.
Use the local implementation and tests, alongside README and grammar, as evidence
of the existing-language baseline rather than assuming a public copy is current.

The compiler emits Core Wasm via `wasm-encoder`. `number` lowers to IEEE-754 `f64`,
including NaN, infinities, and signed zero. Strings and arrays use `i32` descriptor
handles; array assignment aliases storage, indices are checked, and host functions
use the documented raw ABI. Binary `%` is implemented as truncating floating
remainder (JavaScript `%`/`fmod`) through an optional exact guest helper, with no
implicit host import or scalar-only memory requirement. Its existing numerical
and runtime tests are regression requirements for `%=`.

Export selection is also implemented: the CLI provides `--export` and
`--no-exports`, the library provides `ExportOptions`, and the browser adapter
provides `setExports`. All current optimization levels remain no-ops with identical
output and diagnostics; the extensions below do not imply optimization passes.

**Compatibility requirements:**

- Existing valid programs MUST remain valid wherever possible, except new keywords intentionally reserve previously usable names. `break`, `continue`, `loop`, `in`, and `for` are now reserved source/host-language names; existing bindings/functions using them must be renamed. Standalone `_` bindings are intentionally reserved when their extension arrives. Host import module/field names remain unaffected.
- `let` MUST remain assignable under this extension set (no implicit switch to Rust-like immutable `let`).
- Any optimization MUST preserve observable `f64` numerical semantics unless a separately documented opt-in mode is introduced.
- Host imports/exports MUST retain their current raw ABI. Functions with `range` parameters/results cannot be raw-exported or imported until a host ABI is defined; callable types in function signatures are not supported in v1.
- Array iteration MUST respect existing mutable array aliasing behavior.
- Existing left-to-right call-argument evaluation order MUST remain intact.
- Keep the existing default **export-all** policy. Selecting an internal function
  with an unsupported raw host signature (notably `range`) MUST produce a
  compilation error suggesting existing export selection such as `--export main`;
  the compiler MUST NOT silently omit it. Unselected functions still receive
  analysis and backend checks and remain available to internal calls. Export ABI
  rejection belongs to compilation/layout, not to analysis of a valid internal
  typed program; callers may analyze first and choose exports afterward.

### 2.1 Source types and deliberately restricted callable bindings

Keep `number`, `bool`, `string`, `number[]`, and `void`, and add `range` as a first-class source value type. `range` uses safe integral numeric endpoints and steps; range elements still have type `number`.

In v1, a closure is allowed **only as a locally named callable binding**, e.g. `let f = (x: number) => x + captured;`. Its compiler-known signature is attached to the resolved binding rather than exposed as a generally assignable `fn(...) -> ...` source type. The closure can be invoked at that binding's lexical call sites, but MUST NOT be passed as an argument, returned from a function, inserted into an array, exported, or reassigned as a general callable value. Named functions can be invoked and used as pipeline stages but MUST NOT be assigned as values (`let f = named_function;` is not supported).

A named function declaration MUST reject `range` parameters/results when that function is selected for a raw Wasm export. Such signatures are valid **internally**. Callable parameters or result annotations are rejected at source type checking (and thus cannot be exported either). `range` is spelled in lowercase to match the existing source type naming convention; this choice is frozen for v1.

## 3. Cross-cutting semantics

### 3.1 Evaluation order

Unless explicitly stated otherwise:

- Expression operands, call arguments, range endpoints, and array literals evaluate **left to right**.
- Each source expression is evaluated at most once per evaluation of its containing construct, unless the programmer explicitly places it in a loop or closure body.
- `&&` and `||` remain short-circuiting.
- A compound assignment evaluates its target location exactly once, then reads the old value, evaluates the RHS, computes, and writes.
- A `for` statement evaluates its iterable exactly once on entry, in the enclosing context before activating the new loop or introducing its iteration binding.
- Pipeline stages evaluate in order; the previous stage's result is evaluated once and passed forward.
- Explicit call arguments are evaluated in written source order; omitted defaults are evaluated afterward, in formal declaration order. Resolution of names in defaults is lexical at the function declaration, never dynamic at the call site.

### 3.2 Values, control flow, and divergence

**Implementation status:** WP3 implements block/if results and semantic categories
`Value(T)`, `Void`, and `Never`; WP4 adds statement/value loops. Match remains
proposed. Never means no normal completion, not necessarily nontermination:
explicit return, break, or continue can terminate a block's normal path, and a
syntactic loop without a reachable exit yields Never. A possible runtime trap or
arbitrary call is not inferred as Never. Track transfer targets separately from value
types. Never has no source spelling, storage, or ABI type. **Do not** require full
termination proofs or omit required static checks based on reachability.

The implemented public `TypedExpr.ty` is `ExprResult::{Value(Type), Void, Never}`;
`Type::Void.into()` is Void, and `Value(Type::Void)` is malformed. Bindings,
parameters and function results remain concrete Type. Public category equality is
identity, not Never compatibility. Keep `Block` and `TypedBlock` statement-vector
aliases; raw/typed value-block wrappers carry statements, an optional boxed tail,
and a span. Individual expressions still have no independent spans. Semantic
expression analysis propagates transient flow; checked lowering reconstructs it
without public flow annotations, another IR, or an independent second validator.

A block can contain zero or more semicolon-terminated statements and optionally one trailing expression without a semicolon. A trailing expression supplies its block value. Existing statement blocks without a trailing expression continue to have type `void`.

```
let result: number = {
    let x: number = 10;
    x * 2
};
```

- A trailing semicolon discards the expression result. Named-function bodies do
  not implicitly return tails; function and while bodies require semicolons for
  ordinary expression statements. Standalone block expression statements require
  semicolons too. Bare statement-if may omit else and discards branch tails.
- In an expression block a final bare if is a tail; a nonfinal bare if is a
  statement. Bare statement-if ends at its final arm, even before adjacent `-1;`
  or `--1;`. Parenthesize the whole if for arithmetic continuation in these
  statement/tail-classification positions. Whitespace and newlines never decide
  whether a token belongs to the if.
- An expression expected to produce a value MUST have a non-void type on all
  normally completing paths; Never branches are allowed. In implemented syntax,
  `let value = { return 1; };` in a number-returning function cannot infer a storage
  type, whereas `let value: number = { return 1; };` is accepted without a reachable
  store. Void results cannot initialize locals. Implemented `loop {}` follows the
  same rule: `let value = loop {};` fails inference, while an explicitly annotated
  divergent initializer is accepted and has no reachable store.
- Nonvoid functions accept return or explicit divergence, never normal completion
  without a return. `fn forever() -> number { loop {} }` is valid. This does not
  establish that arbitrary calls, trapping operations, or ordinary while bodies
  guarantee noncompletion; ordinary while remains conservative.
- A divergent RHS of `&&` / `||` alone does not make the whole expression Never:
  the short-circuit path may complete normally. Check all operands regardless.
- While loop context is active during its condition, so condition blocks can
  break/continue the nearest loop. An unconditional explicit return in a while
  condition guarantees return before the body. A literal-true condition plus a
  returning body alone still does not satisfy definite return.
- A block expression must not silently read an uninitialized result. Unreachable statements, skipped short-circuit operands, unselected `match` arms, and bodies of otherwise uncalled functions MUST still undergo name resolution, type checking, and contextual validity checks. Runtime evaluation order and static validity are separate concerns.

### 3.3 Scopes and names

- Every loop iteration introduces a fresh logical iteration binding.
- The implemented range `for` binding is read-only within the loop body; ordinary
  and compound writes are rejected, even when unreachable. A distinct inner
  shadowing `let` remains mutable under existing rules. Checked lowering derives
  write restrictions from For provenance, without new public mutable-binding metadata.
- Functions, match arms, lambda parameters, and blocks each create lexical scopes.
- Every named or anonymous function establishes its own return and loop-control
  context. A `return` in a closure exits that closure, not its enclosing function;
  `break` and `continue` MUST NOT target a loop across a function/closure boundary.
- Shadowing follows existing lexical-binding rules; closures capture the *resolved binding*, never a later same-named variable.
- Compiler-generated variables MUST use unique symbol IDs and must not become user-accessible names.
- A standalone `_` is reserved for placeholder expressions and `match` wildcards, not a binding name or ignored-binding pattern. `_temp` and other underscore-prefixed identifiers remain ordinary names.
- `by` is a contextual token recognized only following a range expression, not a globally reserved keyword; `by` remains a valid variable or parameter identifier.

### 3.4 Diagnostics

Compiler diagnostics MUST distinguish parse errors, type errors, invalid
control-flow targets, and unsupported ABI crossings. Runtime contract and range
failures MUST preserve their specified trap behavior. The initial Core Wasm
implementation may expose generic traps; portable rich messages or distinct
runtime failure categories require an explicitly designed debug mapping or error
mechanism and are not guaranteed by `unreachable` alone. Every synthetic/desugared
node SHOULD retain an origin span so compiler errors and any future mapping refer
to source syntax rather than generated scaffolding.

## 4. Feature specification: compound assignment

**Implementation status:** All five operators are implemented for numeric local
and indexed places, with frontend, checked-backend, and integration tests. WP3
block effects now test scalar RHS mutation and array-binding rebinding while
preserving the original place and old value. Shared captures await closure work.

### 4.1 Syntax

```
count += 1;
total -= discount;
scale *= factor;
ratio /= divisor;
values[next_index()] += compute();
```

Initially implement **all five**: `+=`, `-=`, `*=`, `/=`, and `%=` for `number` variables and indexed elements of `number[]`. `%=` MUST delegate to the exact existing `%` language semantics (including division by zero, NaN, signed zero, and sign conventions); Core Wasm has no `f64.rem` instruction, so use the same correct lowering/helper as binary `%`.

### 4.2 Semantics

`TARGET OP= RHS` is equivalent in resulting value to `TARGET = TARGET OP RHS`, **but not to a naive textual rewrite that evaluates TARGET twice**.

1. Resolve `TARGET` as an assignable place (`identifier` or `array[index]`).
2. Evaluate the receiver/base expression once, if any.
3. Evaluate the index expression once, if any.
4. Perform the same bounds/trap checks as ordinary element access.
5. Read the existing value.
6. Evaluate `RHS` once.
7. Perform the numeric operation with established `number` semantics.
8. Store the result in the same place.

The operation is a statement in v1, not an expression. Compound assignment to `range`, `string`, `bool`, `void`, or local callable bindings is not supported.

### 4.3 Compiler implementation

Use a `Place` abstraction (`Local`, `ArrayElement {array_temp, index_temp}`, later more). Lower to a typed `ReadModifyWrite {place, op, rhs}` operation or equivalent statement sequence with synthetic locals. Reuse the backend's checked array read/store logic; do not duplicate expression evaluation. Potential Wasm optimization: `local.tee` where safe, reuse address computation, eliminate redundant bounds checks only with a sound proof.

### 4.4 Acceptance tests

- `x += 1` preserves the type `number`.
- `a[f()] += g()` calls `f` and `g` exactly once each, in that order, even with host-observable side effects.
- Bounds failure does not evaluate `g()` if the target read would trap (based on the specified evaluation sequence).
- An RHS that mutates the same array element does not replace the previously read
  operand: starting from `a[0] = 10`, if `g()` sets `a[0] = 100` and returns `2`,
  `a[0] += g()` stores `12`, not `102`. Keep the original target location as well
  as its old value if later closure support lets the RHS rebind the array variable.
- `2147483647 + 1` retains `f64` semantics.
- `x %= y` agrees with `x = x % y` for `number` values, including special IEEE-754 cases.

## 5. Feature specification: first-class ranges

**Implemented scope:** WP5's first checkpoint covers construction, immutable
private records, locals/assignment, and internal parameters/results. Range values
also work in blocks, ifs, and value loops with exact joins and Never compatibility.
Exact numeric membership (`in`) is implemented; range equality is rejected.
Range statement `for` with exact progression advancement and numeric-array
iteration are implemented. Adapter/interaction release gates remain pending,
so both remain partial WP5.

### 5.1 Syntax

```
let a: range = 0..10;       // 0 through 9
let b: range = 0..=10;      // 0 through 10
let c: range = 10..0 by -2; // 10, 8, 6, 4, 2
let d: range = 0..10 by 3;  // yields 0, 3, 6, 9 in a range for
```

Implemented numeric membership syntax:

```
let contains: bool = 6 in (0..10 by 2); // true
```

`..` excludes the end; `..=` includes the end. Contextual `by` supplies a nonzero
explicit step and binds to the range construct, not arbitrary arithmetic. The default step is `+1`. `in`
tests exact progression membership. Construction with `..`, `..=`, optional `by`,
numeric membership, and range-for iteration are implemented; §6 also implements
numeric-array iteration without changing the range representation.

### 5.2 Type and validation

A `range` represents a finite, ordered progression of **safe integral `number`
values**, not all possible `f64` real values. Construction evaluates start, end,
and explicit step once, left-to-right, all before validation, even if an earlier operand is invalid. At construction,
including for empty or discarded ranges, endpoints and step MUST be finite
integers with magnitude at most
`2^53 - 1`. Endpoint zero is valid; both `+0` and `-0` steps are invalid. Normalize
`-0` endpoints to integer zero, and expose a yielded zero as canonical `+0`.
This range-specific normalization does not change ordinary `number` semantics.

Invalid values MUST trap when range construction executes, even if no iteration
or membership test follows. Invalid constant ranges have the same runtime rule;
constant folding MUST NOT turn them into compilation errors or suppress preceding
side effects. It MAY emit an equivalent trap at the construction's original
execution point while preserving endpoint/step evaluation and earlier effects.
Invalid source syntax or operand types remain static errors. Reachable iteration
MUST NOT silently stall due to floating-point rounding; rich runtime diagnostics
remain subject to the mapping/error-mechanism limitation in §3.4.

The default step is `+1`; `10..0` with no explicit step is empty. A negative step requires `by -N` or other explicit negative-step form. For `0..=0`, there is exactly one element. Empty ranges are legal. `in` tests whether a number is one of the progression's emitted values; `5 in (0..10 by 2)` is `false`. NaN is never in a range.

**Important design choice:** the safe-integer restriction makes first-class ranges well-defined while retaining binary64 `number` semantics. Do not silently convert arbitrary decimals or infinities to integers. The source `range` abstraction need not be stored as a contiguous array.

### 5.3 Grammar and precedence

- `..` and `..=` are lower precedence than arithmetic, higher than comparisons (including `in`), and non-associative.
- `by` attaches only to a range expression.
- Implemented `in` has comparison-level precedence with existing left associativity.
  It requires exact `number` left / `range` right operands and returns `bool` on
  normal completion, with existing Never compatibility. Both operands evaluate
  once, left-to-right; all children remain statically/backend checked, including
  skipped or unreachable code. `in` is reserved for source and host-language names,
  not host module/field names. No new chaining restriction is introduced; chained
  forms parse left-associatively and must satisfy the existing type rules.
- Lexer MUST prioritize `..=` over `..`, and detect decimal literals without swallowing `0..10` as `0.`.
- Chained forms like `0..10..20` are syntax errors unless parentheses explicitly resolve them (even then, endpoints must be numeric).

### 5.4 Representation and performance

WP5's implemented construction/internal-call representation is a single `i32`
handle to an immutable private, eight-byte-aligned 32-byte record, allocated with
the existing bump allocator.
Validated signed `i64` start/end/step occupy byte offsets 0/8/16. A `u32`
inclusivity flag at offset 24 is exactly 0 or 1; four bytes of reserved padding at
offset 28 are initialized to zero. Copies share the immutable record. This is not
the public string/array descriptor ABI and adds no host range-marshaling API.
See [ADR-0001](adr/0001-private-range-representation.md) for the rationale and the
[private record contract](memory-abi.md#private-range-record) for implemented checks.

Internal functions may accept and return range handles. Reject range parameters
or results in every host import, including unused imports and forged host
snapshots. Reject user range signatures only when selected for export; default
export-all fails with guidance to use explicit selection. `None` or compatible
`Only` selection permits internal range functions without pruning their bodies or
skipping validation. Check private record bounds and fields where consumed and
at internal parameter boundaries. Private `Construct`, then `Validate` helpers
append after memory and optional remainder helpers. Optional `Contains` with
signature `(f64, i32) -> i32` follows `Validate` only when membership occurs anywhere
in the typed AST, including unexported, unreachable, or runtime-skipped code.
Construction-only bytes, feature-free bytes, and existing helper indices remain
unchanged. Membership alone needs no floating remainder helper.

Range `==` and `!=` are static errors, including for forged typed input.
Membership does not define handle identity, record equality, or sequence equality.

**Implemented range-for advancement:** Validate the consumed record once and cache
signed `i64` current/end/step plus `i32` inclusivity scratch. Test signed endpoints
before every exact `f64` conversion/yield, including after `I64Add` advancement.
Positive steps use `<`/`<=` and negative steps use `>`/`>=`, according to inclusivity.
A failed check exits immediately; zero yields canonical `+0`. Iteration never
materializes an array. No new range helper is added; `Construct`, `Validate`, and
optional membership-only `Contains` retain their ordering and discovery rules.

**Advancement bounds proof:** Let `M = 2^53−1`. Every yielded current and the
validated nonzero step have magnitude ≤ M, so `next = current + step` satisfies
`|next| <= 2M < 2^54`, safely within signed `i64`. Next may leave the safe-integer
domain, but it MUST be endpoint-tested before conversion/yield. Positive advancement
past M exceeds the safe end and fails `<`/`<=`; negative advancement below −M fails
`>`/`>=`. The loop exits on the first failed check, so no subsequent addition uses
an out-of-domain current. Valid advancement exhausts normally, without overflow,
rounding stalls, traps, or out-of-range yields. Every passing current lies between
the safe start/end and converts exactly to `f64`.

Allocation-free scalar representations of unescaped ranges remain deferred to WP13,
subject to evidence and semantic proofs; caching iteration fields is not scalar
replacement of the immutable allocated range value.

**Implemented exact numeric membership:** After both operands evaluate, validate
consumed record bounds and canonical fields **before** the candidate guard.
Malformed records MUST trap even for invalid candidates. For valid records,
nonfinite, fractional, or out-of-safe-domain candidates MUST return false, not trap;
both signed zeros are accepted and convert to integer zero. For candidate `n`,
start `a`, end `b`, and step `s`, positive steps require `n >= a` and `n < b`
(or `n <= b` for inclusive ends); negative steps require `n <= a` and `n > b`
(or `n >= b` for inclusive ends). Also require `(n - a) rem_s s == 0`, emitted
with signed `I64Sub`/`I64RemS`. Zero remainder handles both step signs without
modulus normalization or floating-point remainder. This tests the described
progression without advancing or iterating it and allocates nothing.

**Membership bounds proof:** Let `M = 2^53−1`. Validated record fields and a guarded candidate
satisfy `|n|, |a|, |b|, |s| <= M`, with `s != 0`. Conversion to signed `i64` is
exact, and `|n - a| <= 2M = 2*(2^53−1) < 2^54`, so subtraction cannot overflow
signed `i64`. The divisor is nonzero and neither operand can be `i64::MIN`,
excluding division-by-zero and the `MIN / -1` corner; signed remainder is safe.
This proof covers membership; advancement has its separate proof above. No optimization
may change behavior for invalid candidates, negative steps, boundary inclusivity,
or record-validation-before-candidate ordering.

### 5.5 Acceptance tests

**Implemented checkpoint coverage:** `tests/range_frontend.rs`, `tests/ranges.rs`,
and `src/codegen/range_tests.rs` cover syntax/types, exact/Never joins, construction
validation/effects, independent record bytes and decoded Wasm, internal signatures,
host/selected-export rejection, equality rejection, and forged typed input. Runtime
cases retain the ignored Node-test policy; default tests alone do not establish
runtime coverage.

**Implemented membership coverage:** `tests/range_frontend.rs` and
`tests/ranges.rs` cover reserved names, comparison precedence/left associativity,
exact/Never operand typing, checking both children, and decoded integer congruence
without floating remainder. Node execution retains the ignored-test policy;
default Cargo success alone does not establish runtime coverage.

**Implemented range-for coverage (issue #4):** The suites in §6.3 cover yielded
values, advancement/exhaustion, effects, transfer rules, forged input, and decoded
lowering at all optimization levels. Runtime execution uses ignored, bounded Node
cases; default Cargo success alone does not establish runtime coverage. Numeric-array
iteration is implemented under issue #5; broader WP5 adapter/interaction release
gates remain pending. Current CLI/browser compilation supports both iterable types
through the shared core.

`0..3` yields 0,1,2; `0..=3` yields 0,1,2,3; `3..0 by -1` yields 3,2,1;
`5..5` is empty; `5..=5` has one item. Fractional/NaN/infinite/out-of-domain
endpoints or steps and both signed-zero steps trap, including empty or unused
range constructions. Zero endpoints are valid; `-0..=0` yields exactly one `+0`,
verified with `Object.is`. Side-effectful endpoints/steps evaluate once, left to
right. Test exact membership near both safe-integer limits, normal exhaustion
without a trap, and preserved preceding host effects for invalid constant ranges
at every optimization level. Pin record bytes independently, cover internal range
parameters/results and selected-export/host-import rejection, and reject range
`==`/`!=` in source and forged typed trees.

## 6. Feature specification: `for` loops and direct array iteration

**Implemented scope (issues #4–#5):** Statement `for` over ranges with exact `i64`
advancement and over `number[]` with unsigned index iteration and live element
loads. Both use a continue latch. Adapter/cross-feature release gates remain
pending; range and array iteration are still **partial WP5**.

### 6.1 Syntax

```
for i in 0..10 {
    print(i);
}

let bounds: range = 2..8;
for i in bounds {
    print(i);
}
```

Implemented numeric-array syntax:

```
let values: number[] = [10, 20, 30];
for value in values {
    print(value);
}
```

Implemented syntax is `for name in iterable { body }`, accepting `range` or
`number[]` with existing Never compatibility, not `number`, `bool`, `string`, or
Void. `for` is reserved for source and host-language names, never host module/field
names. The body remains a strict statement vector, not a value block; for is never
a value expression, including when last in a value block. Issue #5 reuses the
existing parser and raw/typed For shapes. Strings and arbitrary user iterators are
out of scope for v1; no implicit iterator protocol is introduced.

### 6.2 Semantics

- The iterable expression is evaluated once before the first iteration, in the enclosing context before activating the new `for` or introducing its iteration binding. Break/continue there target an existing enclosing loop under its usual rules, or are invalid without one. Unlike a while condition, the iterable cannot transfer to the new loop. Return/divergence during iterable evaluation is preserved; it does not create a normally completing `for` path.
- The range/array loop variable is a body-local, read-only `number`. Ordinary
  and compound assignment to it is rejected even in unreachable code. A distinct
  inner shadowing let is mutable; the iterable sees only enclosing bindings.
- The range yields arithmetic-progression elements per §5.
- Array iteration captures its handle and unsigned `u32` length once after full
  `ValidateArray`, including for empty arrays. Elements are loaded on each iteration,
  not snapshotted: alias mutations to unvisited elements are visible. Reassigning
  the source variable does not replace the captured handle. Current arrays have
  fixed length; hosts must preserve descriptor metadata under the existing ABI.
- `break;` exits the nearest active loop. `continue;` starts its next iteration
  (for a range this performs the step; array iteration advances its index).
  `return` exits the function.
- A loop body may shadow its iteration variable with a distinct inner binding.
- The `for` loop itself is a void statement, not a value expression. `break VALUE;`
  targeting for is forbidden even when unreachable or when its RHS is Never;
  nested value loops retain their own value-break rules.
- Check the body even if the iterable cannot complete, and preserve iterable
  transfer/return/divergence paths without inventing fallthrough. A normally
  completing iterable may be empty, so a returning body alone does not prove
  definite return.

### 6.3 Lowering

Implemented range lowering (the iterable runs before the new loop/binding is active):

```
let iterable = eval_range_once();
validate_record(iterable);
let current: i64 = iterable.start;
let end: i64 = iterable.end;
let step: i64 = iterable.step;
let inclusive: i32 = iterable.inclusive;
block break_target {
    loop backedge_target {
        if !signed_endpoint_check(current, end, step, inclusive) { break break_target; }
        let i: f64 = exact_conversion(current);
        block continue_target {
            BODY // continue exits this block and reaches advancement
        }
        current = I64Add(current, step);
        branch backedge_target;
    }
}
```

Implemented array lowering (issue #5; iterable evaluation precedes loop/binding activation):

```
let array_ref: i32 = eval_array_once();
ValidateArray(array_ref); // full descriptor/payload validation, even when empty
let count: u32 = load_descriptor_length(array_ref);
let index: u32 = 0;
block break_target {
    loop backedge_target {
        if unsigned_ge(index, count) { break break_target; }
        let address = ArrayAddress(array_ref, exact_f64_conversion(index));
        let value: f64 = load_f64(address);
        block continue_target {
            BODY // continue exits this block and reaches advancement
        }
        index = index + 1;
        branch backedge_target;
    }
}
```

Range lowering uses exact `i64` current/end/step and `i32` inclusivity scratch,
with signed bounds checks before every `f64` conversion/yield; §5.4 proves safe
advancement and normal exhaustion even when next leaves the safe domain. A body
latch block makes continue reach advancement and retest instead of skipping them.
Resolve all branch depths from label identity, including nested value ifs/blocks.
Array lowering uses `i32` handle/length/index scratch with unsigned comparisons.
Each yield calls existing checked `ArrayAddress` before its `f64` load; it does not
cache a raw payload pointer across body effects or eliminate per-element checks.
No ABI or helper-order change is introduced: array-only iteration needs no range
helpers, and range iteration adds no helper or floating remainder need.

Issue #4's public typed statements add
`For { id, variable, name, iterable: Box<TypedExpr>, body }`. LoopId and VariableId
follow existing per-function uniqueness, scope, initialization, and active-target
checks, including forged typed input. Reconstruct read-only write restrictions
from For provenance, without public mutable-binding metadata or a second validator.
Issue #5 extends iterable typing/lowering without changing parser or AST shapes.
The structural walker visits iterable then body within body block hooks; retain
separate local/memory/temporary/remainder policies and no-reuse scratch slots.

**Implemented test coverage:**

- `tests/for_frontend.rs` covers syntax/reserved names and source spans, iterable
  typing, statement versus expression contexts, read-only writes and mutable
  shadowing, binding scope, nearest-active/iterable transfer targets, unreachable
  value-break rejection, conservative empty-iterable flow, and analyzer reuse after errors.
- `tests/for_lowering.rs` covers forged types/results/IDs/targets, read-only writes
  and exact error spans, initialization/scope restoration, scratch types/preorder,
  consumed-record validation/caching before loop regions, and independent decoded
  conversion/latch depths with `wasmparser::Validator`. Rejections preserve the
  typed input across export selections and all optimization levels.
- `tests/for_loops.rs` covers deterministic output at all optimization levels, owned
  import snapshots, export indices/body preservation, helper counts and distinct
  scratch slots. Ignored bounded Node cases compare exact yields/exhaustion and
  membership against an independent BigInt progression reference near ±safe limits,
  with large steps and canonical +0. They also check once-only left-to-right effects,
  nested body/iterable transfers, consumed-record validation even for empty ranges,
  and field caching before body effects. Node cases are ignored by default;
  default Cargo success alone does not establish runtime coverage.
- `tests/array_for_loops.rs` is the numeric-array iteration suite; the existing
  `tests/for_frontend.rs` and `tests/for_lowering.rs` cover shared frontend and
  checked-lowering boundaries. Test presence is not a claim that runtime validation
  or adapter/interaction release gates passed.

Current CLI/browser adapters compile range and array for through the shared core;
broader WP5 adapter/interaction release gates remain pending. Rebuild browser
bindings before adapter validation; stale bindings do not validate current source.

**Lowering warning:** `continue` must target the increment/latch path rather than jump around the induction-variable update. Implement an explicit latch block or equivalent structured region; naive `br loop` from the middle of the body can produce an infinite loop.

### 6.4 Optimization opportunities

These are future WP13 opportunities, not new optimization passes for issues #4–#5.
Once-only iterable evaluation, cached range fields, and captured array handle/length
are already semantics, not optional optimizations.

- For arrays with a verified fixed length, bounds-check elimination MAY occur for the compiler-generated index; mutable aliasing by itself does not change length, but memory corruption or host-memory mutation assumptions must be documented.
- Range fields are already cached; optional literal-step specialization and proposed
  array offset strength reduction require separate correctness/measurement gates.
- Preserve implemented exact `i64` range induction and test-before-conversion
  semantics; do not replace them with rounding-prone `f64` advancement.
- WASM SIMD or loop fusion are future optimizations, not guarantees introduced by `for` syntax.

## 7. Feature specification: `break`, `continue`, and infinite `loop`

**Implementation status:** WP2–WP4 implement while transfers, transfers through
expression regions/conditions, and statement/value loops with value breaks.
Typed while/for/loop nodes carry per-function LoopIds; transfers carry resolved
innermost active targets. Range/array for is implemented as specified in §6;
labels remain proposed. A transfer is not a function
return, but a loop without reachable exit explicitly diverges. Nonvoid functions
allow returns or explicit divergence, never normal fallthrough. Ordinary while
remains conservative even with literal true; a noncompleting condition consumes
its own break/continue outcomes without inventing normal completion.

### 7.1 Syntax

```
while condition {
    if should_stop() { break; }
    if should_skip() { continue; }
}

loop {
    if finished() { break; }
}

let first: number = loop {
    let value: number = next();
    if value > 0 {
        break value;
    }
};
```

### 7.2 Control-flow rules

- Unlabeled `break`/`continue` target the **innermost enclosing loop**, regardless of nested `if`, `match`, and ordinary blocks.
- `break;` is valid in `while`, `for`, and statement-position `loop`.
- `continue;` is valid in any loop; it skips to the loop's continuation/latch point.
- `break EXPR;` is valid only for a `loop` expression. Every *reachable* `break` targeting that loop expression MUST produce a compatible non-`void` value; a bare `break;` in that loop is a type error.
- `loop { ... }` can be used as a statement or a value-producing expression.
  Prefix bare loops in function/statement bodies, including `loop {};`, are
  statements. In a value block, a final bare loop without a semicolon is its tail;
  nonfinal/semicolon-terminated bare loops are statements. Parenthesized loops
  are expressions, including discarded values. No newline heuristics apply.
  Loop bodies contain strict statements; only value breaks supply results.
- Bare breaks in value loops, value breaks in statement/while/for loops, and Void
  break RHSs are invalid even when unreachable. Never RHSs supply no value or
  actual break. Unreachable but otherwise valid break values do not constrain
  the result; exact concrete result joins are checked at the loop boundary.
- The type of a value-producing `loop` is the unified type of its reachable breaks. A syntactic `loop` with no reachable `break` exiting that loop has type `Never`; the analysis need not prove termination of arbitrary conditionals or functions. `Never` can flow into a concretely annotated declaration. Unreachable source is still type-checked.
- A `break VALUE;` in a nested loop affects only that nested loop; its value does not type the outer loop.
- Loop labels, labeled breaks, and `break VALUE` in `for`/`while` are deferred.

### 7.3 Wasm lowering

Use a stack of compiler `LoopContext` objects containing typed break target, continue target, result type, and nesting-depth bookkeeping. Never hardcode `br 0`/`br 1`: nesting `if`, `block`, or nested `loop` changes relative label depths.

For a value-producing `loop`, an outer `block (result T)` can receive the break value; `break VALUE` compiles `VALUE` then branches to the result-bearing outer block. With MVP-compatible Wasm, `f64` and `i32` results are straightforward;
Existing string/array results use their i32 descriptor handle. Implemented `range`
results use their private i32 record handle. Local
closure bindings are not transferable values and cannot be loop results. The empty body without breaks must not produce an arbitrary value on fallthrough.

The implemented raw/typed statement and expression enums add Loop nodes, with
body vectors and typed IDs. Bare Break variants remain; raw `BreakValue(Expr)`
and typed `BreakValue { target, value }` are separate. Update public exhaustive
matches. Private Flow carries reachable value-break type constraints and explicit
divergence through sequence/branch composition, consuming them at loop boundaries.
Do not collect unreachable constraints opportunistically or rewalk subtrees.

Declared annotations choose MVP outer-block results and are verified against
reconstructed flow. A checked unreachable break with an incompatible raw Wasm
payload emits Unreachable instead of an invalid typed branch; its type constraint
still rejects compilation if reachable. Preserve scopes, unique label identities,
live operand prefixes and stack polymorphism, without an artificial result local.
The walker traverses loop bodies within block hooks and value-break RHSs as
expressions, leaving memory/local/scratch/remainder policies separate.

### 7.4 Acceptance tests

- Nested loops pick the correct targets. WP4 tests cover all existing concrete
  results, unreachable valid mismatches, forged kinds/results/IDs, divergent
  initialization and nonvoid completion, plus independent decoded MVP regions
  and branches with Validator checks. Bounded Node tests cover effects, aliases,
  transfers and live stack prefixes. Infinite examples are compile-only tests;
  subprocess deadlines are not guest fuel or process-tree cancellation.
- `continue` in `for` advances the induction variable.
- `break 10;` in `while` is rejected.
- Mixing `break 10;` and `break true;` from the same value loop is rejected.
- Control flow inside `match` and `if` nested in a loop uses correct Wasm branch depths.

## 8. Feature specification: `if` and `match` as expressions

### 8.1 `if` expression

**Implementation status:** Implemented in WP3 for all existing concrete result
kinds and Void/Never joins. `match` (§8.2 onward) remains proposed.

```
let sign: number = if value >= 0 { 1 } else { -1 };

let result: number = if ready {
    compute()
} else {
    fallback()
};
```

- An `if` used in **value context** MUST have an `else` on every reachable path; branch results unify to exactly one type, except that `Never` is compatible with the other branch.
- `if` used purely in statement context MAY omit `else` and behaves as before.
- Conditions MUST be `bool`; numeric truthiness remains disallowed.
- The trailing expression of a branch supplies the branch value; `return`, `break`, and `continue` may replace a value if they terminate that path.
- Lower through MVP single-result `if` / `block`: f64 for number, i32 for bool and
  string/array descriptor handles, empty for Void/Never. Never paths must not
  fabricate a result, store, or drop; preserve stack polymorphism after a region
  with no normal completion. No new ABI or multi-value feature is required.
- Two normally completing Void arms produce Void, discardable but not a local
  value; otherwise normally completing arms match exactly. A Never arm joins
  its counterpart, and two Never arms produce Never.

### 8.2 `match` expression syntax

```
let category: string = match score {
    >= 90   => "A",
    >= 80   => "B",
    >= 70   => "C",
    _       => "F",
};

let description: string = match value {
    0         => "zero",
    1..=9     => "one digit",
    < 0       => "negative",
    _         => "other",
};

let choice: number = match ready {
    true  => 10,
    false => 0,
};
```

v1 patterns: wildcard `_`, exact constant literals, relational patterns (`<`, `<=`, `>`, `>=` followed by a compile-time constant number), compile-time-constant range patterns, and boolean patterns. String exact-literal matching MAY use the existing content-equality implementation. General destructuring, binding patterns, or-patterns, and guards are deferred.

### 8.3 Semantics

- Evaluate the scrutinee once; examine arms top to bottom; evaluate only the selected arm's expression/block.
- Relational patterns compare scrutinee against their constant with IEEE-754 `f64` comparisons. NaN does not satisfy equality or relational numeric patterns and can fall through to `_`.
- A match in value context MUST be exhaustive. The initial analyzer MAY require a final `_` arm for `number`/`string` scrutinees; `bool` can be exhaustively covered by `true` and `false`.
- Each reachable arm MUST have a compatible result type. Duplicate or provably unreachable arms SHOULD be warned about (not silently rejected until pattern coverage analysis is reliable).
- `match` as a statement is allowed; non-exhaustive statement matches SHOULD still be rejected or require `_` for simplicity and predictability.
- The range pattern `1..=9` means numeric interval matching, **not** the iteration/progression-membership meaning of first-class `range` values with `by`. It matches values satisfying interval bounds, including fractional numbers unless the language explicitly restricts it later. This context-sensitive distinction MUST be documented. To test progression membership use `value in range` in an `if` expression.

### 8.4 Wasm optimization opportunities

- Prefer direct numeric comparisons/branch chains for sparse or relational cases.
- If a collection of exact numeric literal cases is proved integral, safe, and sufficiently dense, MAY lower to `br_table` after validated conversion; maintain a fallback for NaN/nonintegral values.
- Compile string patterns via content comparison, not descriptor identity.
- Block-result typing allows fewer mutable locals for straightforward branch values.
- Constant propagation may prune unreachable arms; ordered pattern semantics must remain unchanged.

## 9. Feature specification: inferred function return types

### 9.1 Syntax

```
fn square(x: number) {
    return x * x;
}

fn description(x: number) {
    if x > 0 { return "positive"; }
    return "non-positive";
}

fn log_value(x: number) {
    print(x);
}

fn explicit(x: number) -> number {
    return x;
}
```

Only the **return type annotation** is optional: parameter annotations remain required in v1. An omitted return type is *inferred*, not implicitly `void`. A body with no returned value is inferred `void`.

### 9.2 Inference algorithm

1. Build a symbol table containing all function signatures, marking omitted results as inference variables.
2. Analyze bodies and gather return constraints from every reachable `return EXPR;` and `return;`. Named function bodies require explicit `return` to produce a non-void result; a non-void tail expression without `;` directly at named-function scope (e.g. `fn answer() { 42 }`) is a compile-time error. Nested expression blocks and anonymous-function bodies MAY use tail expressions.
3. Unify result types. Mixed `return;` and `return VALUE;` in one reachable function body are an error.
4. Perform the same definite-return check currently used for explicitly typed non-`void` functions.
5. Solve dependencies among functions in call-graph order. Mutually recursive strongly connected components whose return types cannot be uniquely resolved from nonrecursive constraints MUST require explicit annotations.
6. Emit clear diagnostics instead of guessing `number` from a numeric literal or selecting `void` from an incomplete path.

**Examples:**

```
fn a(n: number) {
    if n <= 0 { return 0; }
    return b(n - 1);
}

fn b(n: number) {
    return a(n);
}
```

This recursive component has an anchor (`return 0`) but still requires a proper constraint solver; implementation MAY conservatively demand explicit annotations for all recursive or mutually recursive functions in the first release. That is a staged limitation, not a language-level rule forever.

### 9.3 Wasm ABI and optimization

Return inference is complete **before module type section emission**. It does not produce runtime polymorphism or multiple Wasm signatures by itself. The result ABI is exactly the ABI that the inferred source type would have had if written explicitly. Caching inferred signatures stabilizes forward-call resolution. Never change an exported function's ABI opportunistically based on call-site arguments.

## 10. Feature specification: anonymous functions and local closures

### 10.1 Syntax and restricted v1 use

```
let double = (x: number) => x * 2;
let factor: number = 3;
let multiply = (x: number) => x * factor;

factor = 5;
let result: number = multiply(10); // 50
```

An anonymous function uses `(typed_parameters) => expression` or `(typed_parameters) => { statements; tail_expression }`. Local closure parameters are explicitly typed in v1. Its result type is inferred from expression bodies or compatible explicit `return` statements / tail expressions. Zero-argument closures are supported (`() => { ... }`). A locally declared closure binding is a **named lexical callable**, not a first-class value in the public type system.

Allowed: direct lexical invocation `multiply(10)`, immediate invocation of a syntactically known closure if implemented, closure stages in pipelines after the closure phase, and closures referring to other lexical values where no callable escapes.

Rejected in v1: `fn apply(f: fn(number) -> number)`, `return multiply;`, `return (x: number) => x + 1;`, `let f = named_function;`, `let g = multiply;`, assigning a different closure to `multiply`, storing a closure in an array, exporting/importing a callable, or passing a closure as any call argument. The general `fn(...) -> ...` source type syntax is deferred.

### 10.2 Confirmed shared-binding capture semantics

**Decision: captures observe and can mutate the original lexical binding.** This overrides the former capture-by-value proposal.

- `number`, `bool`, `string`, `range`, and `number[]` bindings are captured by reference to their **variable cell**, not as immutable snapshots. Outer reassignment is visible inside the closure; closure reassignment is visible in outer code.
- If a captured `number[]` binding is reassigned to another array, later closure reads observe the new reference. Element writes on either side obey existing shared-array alias semantics.
- Closures capture the binding resolved in lexical scope; shadowing a name later does not retarget the capture.
- Multiple closures capturing the same binding share one cell. Nested closures may use the same cell when valid under the local-only restrictions.
- Captures preserve the resolved binding's type **and mutability**. Mutable `let`
  bindings may be reassigned or updated with `count += 1;` from a closure; the
  effects are visible in outer code. Read-only `for` iteration bindings remain
  read-only when captured, including through nested closures. No implicit
  Rust-style immutable `let` semantics are introduced.
- Closures MUST NOT escape their enclosing **function invocation** in this release;
  that restriction does not weaken permitted shared mutation or lexical visibility.
- A closure has its own function-control boundary: `return` exits its body;
  `break` and `continue` can target only loops inside that body. Merely capturing
  a value from an enclosing loop does not make the loop a legal transfer target.

```
fn example() -> number {
    let count: number = 0;
    let increment = () => { count += 1; return count; };
    increment();
    increment();
    return count; // 2
}
```

### 10.3 Storage, lifetime, and implementation

WebAssembly locals are private to each function invocation. A generated lambda helper therefore cannot directly access its creator's Wasm locals. Perform free-variable/capture analysis, promote captured mutable bindings to shared **cells** (e.g. in a closure environment region of linear memory), and rewrite both outer and inner accesses to load/store the same cell. Non-captured locals stay in Wasm locals.

For a compiler-known local closure, generate a static helper function plus a known environment pointer, then emit a **direct `call`** passing the pointer. This does not require general `call_indirect`, a table, public callable types, or first-class function values. A local closure binding can be compiler metadata rather than a heap-stored universal callable descriptor. If a closure captures nothing, omit the environment argument via a distinct helper signature where convenient.

Initially allocate environments and boxed cells with the existing monotonic bump
allocator. They remain allocated until the instance is discarded; local-only
closures do not imply reclamation or memory-bounded repeated invocation. Each
recursive or host-reentrant invocation MUST have distinct capture storage, while
closures capturing the same live binding instance share its cell. Non-escaping
closures may cease to be usable at scope exit, but their cells are not freed by
this initial implementation.

**Never reset the general heap pointer to reclaim a closure frame.** Arrays or
strings allocated by the guest or a host during the invocation may be returned,
retained by the host, or otherwise remain live after it. Rewinding their shared
allocator would allow valid persistent storage to be overwritten. A future frame
allocator or arena MUST use a separate allocation discipline or a proven lifetime
strategy that cannot invalidate those values. It must handle nested calls,
recursion, host reentrancy, and traps; Wasm traps do not automatically run cleanup.
**Never** reuse a cell while an eligible in-scope closure can still access it.

Optimization requires proof: inline local closure bodies, forward a known address, eliminate a cell if all reads/writes can be safely unified in one lowered function, or scalar-replace storage only while preserving shared-binding reads/writes and observable call ordering.

### 10.4 Diagnostics and future expansion

Reject passing, returning, exporting, aliasing, or treating closure bindings as general expression values. Report a clear diagnostic suggesting an ordinary named function or a local invocation. General callable parameters/results and escaping closures are deferred to a separate future specification, as are dynamic dispatch and general closure heap-lifetime management.

## 11. Feature specification: placeholder functions

### 11.1 Syntax

```
let double = _ * 2;
let add = _ + _;
let over_ten = _ > 10;
let squared = (x: number) => x * x;
```

Each unbound `_` in a placeholder-function expression denotes a **distinct parameter** in left-to-right source order. `_ + _` therefore represents binary addition, not the square of one argument. `_temp` remains a valid ordinary identifier, but standalone `_` cannot be declared as a binding name.

### 11.2 Abstraction boundaries

Parenthesized placeholder expressions are explicit regions: `(_ * 2)`. In unambiguous local initializers, `let double = _ * 2;` may omit outer parentheses. In a pipeline stage, `value |> (_ * 2)` invokes the synthesized local lambda immediately. In a `match` pattern, `_` is a wildcard, never a placeholder.

Placeholder abstraction MUST produce the **same compiler-known local closure representation** as §10, including shared reference capture of external bindings. It MUST NOT introduce a different closure runtime or bypass the restriction on passing/returning callable values. Examples such as `filter(_ > 10)` and `map(_ * 2)` are **not valid normal function arguments in v1** and must not be presented as supported unless separately introduced as compiler-known constructs.

### 11.3 Typing

Infer placeholder parameter types from uniquely constraining operations (e.g. `_ * 2` implies `number`, `_ > 10` implies `number`). A placeholder occurrence without sufficient context (`_ == _`) is an ambiguity error; use explicitly typed lambda parameters instead. The placeholder pass MUST run before final closure capture analysis, and anonymous parameter symbols MUST be generated hygienically.

## 12. Feature specification: pipeline operator `|>`

### 12.1 Two staged entry points, one semantic rule

The initial pipeline implementation precedes closures and supports named functions only:

```
let result = 10 |> double |> square;
let result = 10 |> add(5);       // add(10, 5)
let result = 10 |> clamp(max: 100);
```

After local closures (§10) are available, additionally support a known lexical closure binding or an immediately expressed placeholder stage:

```
let factor: number = 3;
let scale = (x: number) => x * factor;
let result = 10 |> scale;
factor = 4;
let other = 10 |> (_ * factor);
```

A pipeline stage is either `value |> named_function_or_local_closure` (call with the value) or `value |> callable_name(args...)` (insert the value as the **first positional argument**). Immediate syntactic lambda/placeholder stages are permitted only after closure support. The pipeline operator does **not** make a function a first-class value and does not permit closures as ordinary function arguments. `values |> filter(_ > 10)` is rejected in v1 unless `filter` is separately defined as an intrinsic language construct with a dedicated lowering.

### 12.2 Order, binding, and errors

- `|>` is left-associative, below logical OR and above assignment statements (which remain statements, not expressions).
- Evaluate the input once, then process stages left-to-right. Within a call stage, the injected input logically binds to parameter 0; evaluate explicit arguments exactly once in written order; then omitted defaults in formal order.
- `x |> f(a)` means `f(x, a)`, **never** `f(a)(x)` or an intermediate partial function.
- `x |> f` requires a statically resolved named function or supported local closure accepting one argument.
- A `void`-producing stage is valid only as the final stage in statement context.
- Named arguments on later parameters work normally: `value |> clamp(min: 0, max: 100)`.
- `f(_, 10)` is **not** a pipeline insertion syntax; unbound `_` in an ordinary argument would attempt to pass a closure and is rejected in v1.

### 12.3 Compiler lowering and optimization

Resolve the stage to a known declaration or local closure binding. Lower to a direct call with compiler-generated evaluation-order temporaries; preserve the same injection, evaluation order, and full-arity ABI from the earliest named-only implementation. After local closures arrive, direct-call the static helper with shared-capture environment as needed. General `CallIndirect` infrastructure is not required. Cross-stage inlining and elimination of intermediate temporaries are optional if observationally equivalent. Higher-order `filter`/`map` fusion is deferred until an explicit collection API is designed.

## 13. Feature specification: function preconditions/contracts

### 13.1 Syntax

```
fn divide(a: number, b: number) -> number
    requires b != 0
{
    return a / b;
}

fn normalize(x: number) -> number
    requires x >= 0
    requires x <= 100
{
    return x / 100;
}
```

`requires` attaches to a named function declaration after its optional return type, before `{`. Contract expressions are typed `bool`. Each condition is checked in declaration order.

### 13.2 Semantics

- Preconditions are evaluated **after argument binding and default-argument evaluation**, but before execution of the function body.
- Every normal call path to the function must enforce its preconditions unless the compiler proves they hold.
- False conditions produce a **contract failure trap**. The initial Core Wasm backend may use `unreachable` and emit optional debug metadata containing the contract's source span and textual condition; a plain Wasm trap cannot portably deliver a rich error message by itself.
- Contract expressions must be side-effect-free in v1: permit parameter reads, approved arithmetic/comparison/logical operations, and an allowlist of compiler-known pure builtins. Reject **all user-function calls and host-function calls** in contracts in v1, even when they appear harmless. Reject assignments. User-function purity analysis is a future feature.
- Contracts do not automatically prove the function's return value or eliminate all runtime safety checks.
- Named functions exported through the host retain runtime precondition checks; closures are not exported or passed as values in v1.

**Security/correctness:** never remove a trap solely because the user wrote a `requires` clause. The clause is an assertion to verify, not a trusted assumption that excuses unsound codegen.

### 13.3 Static reasoning and lowering

Initially: at function entry, emit `if (!condition) unreachable`. Later: a local refinement/range-analysis pass can propagate facts known after a successful check; eliminate redundant checks only if the proof includes all side-effectful intervening operations and mutation/aliasing rules. Constant true contracts can disappear; constant false contracts can produce a compile-time warning or an unconditional trap, while preserving defined call behavior.

## 14. Feature specification: named and default parameters

### 14.1 Syntax

```
fn clamp(value: number, min: number = 0, max: number = 100) -> number {
    return if value < min { min } else if value > max { max } else { value };
}

let a = clamp(120);
let b = clamp(120, max: 200);
let c = clamp(value: 50, min: 10, max: 80);
```

Defaults are trailing parameters only in v1. All parameters still have explicit types. Once a call contains a named argument, subsequent arguments MUST be named. Positional arguments fill parameters from the start in declaration order; named arguments bind the remaining parameters by name. Duplicate, unknown, or missing required arguments are static errors. A pipeline-injected argument fills the first parameter before the explicit stage arguments.

### 14.2 Default expression restrictions and scope

Default expressions may reference earlier parameters, literals, and **allowlisted compiler-known pure builtins**; they MAY NOT reference later parameters, user-function calls, or any host-function calls. Every name in a default is resolved lexically in the **callee declaration scope**, not against names visible at a call site. Example:

```
fn increment(value: number, by: number = value / 10) -> number {
    return value + by;
}
```

Defaults are evaluated **for each invocation**, not once at declaration time. Explicit arguments evaluate in **written order**, with their values stored in temporaries. Then omitted defaults evaluate in **formal parameter order** with earlier argument values available. Host-observable side effects are forbidden in default expressions in v1, simplifying consistent evaluation and optimization. This still permits runtime-dependent defaults referring to earlier parameters.

### 14.3 ABI and local callable restrictions

The compiler MUST preserve one canonical full-arity machine signature per named function. Bind written positional/named arguments to formal parameters before emitting a call. Evaluate explicit arguments once in source order, then omitted defaults in formal declaration order, then execute contracts and the body.

```
clamp(120, max: 200)
  => let $value = 120;
     let $max = 200;
     let $min = 0;
     clamp($value, $min, $max)
```

Local closure bindings can be called with their explicit positional signature; named/default parameters on closures are deferred unless separately specified. Named functions cannot be assigned as values (`let f = clamp;` is invalid in v1). Raw Wasm exports use their full-arity ABI; a separate host binding generator MAY someday supply convenience defaults. A `range` parameter/result requires a defined host ABI and thus currently blocks raw export selection.

## 15. Lexing, parsing, and precedence

### 15.1 New tokens

Reserved keywords: `for`, `in`, `loop`, `break`, `continue`, `match`, `requires`. `by` is **contextual only** after a range expression, not a globally reserved keyword. Existing keywords remain.  
Operators/tokens: `+=`, `-=`, `*=`, `/=`, `%=`, `..`, `..=`, `=>`, `|>`, `:` in named arguments (already present elsewhere), and `_` as a context-dependent placeholder/wildcard token.

`_` on its own is reserved in expressions/patterns **and rejected in binding positions**; ordinary identifiers such as `_temp` remain valid.

Lexer precedence must recognize `..=` before `..`, `=>` before `=`, `|>` before `|`, and compound assignment before its component punctuation. Numeric tokenization must distinguish `1.25` from `1..25`.

### 15.2 Illustrative EBNF fragment

The grammar below is **an integration sketch**, not a drop-in replacement for the current `grammar.bnf`. It omits established lexer rules, annotations, and trivial productions. `Expr` is a Pratt-/precedence-parsed expression and `Block` has an optional tail expression.

```ebnf
FunctionDecl    := 'fn' IDENT '(' ParamList? ')' ('->' Type)? RequiresClause* Block ;
Param           := IDENT ':' Type ('=' Expr)? ;
RequiresClause  := 'requires' Expr ;
Type            := 'number' | 'bool' | 'string' | 'void' | 'range'
                 | 'number' '[' ']' ; // callable types deferred

Statement       := LetStmt | AssignStmt | CompoundAssignStmt
                 | WhileStmt | ForStmt | BreakStmt | ContinueStmt
                 | ReturnStmt | Expr ';' | ExprBlockStatement ;
ForStmt         := 'for' IDENT 'in' Expr Block ;
WhileStmt       := 'while' Expr Block ;
BreakStmt       := 'break' Expr? ';' ;
ContinueStmt    := 'continue' ';' ;
CompoundAssignStmt := Place ('+=' | '-=' | '*=' | '/=' | '%=') Expr ';' ;

Block           := '{' Statement* TailExpr? '}' ;
TailExpr        := Expr ; // no terminating semicolon
IfExpr          := 'if' Expr Block ('else' (Block | IfExpr))? ;
LoopExpr        := 'loop' Block ;
MatchExpr       := 'match' Expr '{' MatchArm (',' MatchArm)* ','? '}' ;
MatchArm        := Pattern '=>' (Expr | Block) ;
Pattern         := '_' | Literal | RelOp ConstantExpr | ConstantRange ;

RangeExpr       := ExprWithoutRange ('..' | '..=') ExprWithoutRange ('by' ExprWithoutRange)? ;
LambdaExpr      := '(' TypedParamList? ')' '=>' (Expr | Block) ; // legal only as locally bound closure or immediate stage
PipelineExpr    := Expr ('|>' PipelineStage)* ;
PipelineStage   := NamedFunctionOrLocalClosure | CallToKnownFunction | ParenthesizedLocalLambdaStage ;
CallArgument    := (IDENT ':')? Expr ;
```

**Ambiguity note:** `if`/`loop` can appear as expressions and standalone statements. Parsing a source `if condition { ... }` as an `if` expression is fine if context/semicolon handling permits the previous statement form. For ambiguous adjacent expression blocks, use explicit grammar termination and parser tests rather than ad hoc newline-sensitive rules.

### 15.3 Recommended precedence, loose to tight

| Precedence tier | Operators | Associativity/notes |
|---|---|---|
| 1 | Assignment and compound assignment | Statements only in v1 |
| 2 | `|>` | Left |
| 3 | `||` | Left, short circuit |
| 4 | `&&` | Left, short circuit |
| 5 | `==`, `!=` | Non-chaining recommended |
| 6 | `<`, `<=`, `>`, `>=`, `in` | Existing left associativity retained, including implemented `in` |
| 7 | `..`, `..=` and attached `by` | Non-associative range construction |
| 8 | `+`, `-` | Left |
| 9 | `*`, `/`, `%` | Left; `%` retains implemented truncating floating remainder |
| 10 | unary `!`, unary `-` | Right |
| 11 | calls, indexing, parenthesized primaries | Highest |

This table is a *parser proposal*, not a universal claim about how other languages associate these operators. Care is needed to permit `for i in 0..10 { ... }` without consuming `{` as an expression call or block ambiguity. Parentheses are always available to disambiguate.

## 16. AST, typed analysis, and desugaring architecture

### 16.1 Raw AST additions

Issue #4 adds raw `StatementKind::For { name, iterable, body }` and typed
`TypedStatementKind::For { id, variable, name, iterable: Box<TypedExpr>, body }`.
For is statement-only; no raw/typed For expression variant or public mutable-binding
metadata is added. The original typed node is lowered directly through the existing
checked boundary, not desugared into another public tree or IR. Issue #5 adds
numeric-array iteration without changing these parser/AST shapes.

WP3's implemented raw/typed expression enums add `Block` and `If`. `ValueBlock`
and `TypedValueBlock` hold statements, an optional boxed tail, and span, while
function/statement bodies retain their vector aliases. The planning walker
visits statements then tail within block hooks. Locals now traverse expressions
for nested declarations without validating expression contents, reserving the
outer let's slot before initializer declarations. Memory, scratch and remainder
planning remain separate policies. The following broader shapes are illustrative
future extensions, not a replacement AST or a new core IR.

Suggested nodes (names illustrative; fit existing code style):

```
Stmt::CompoundAssign { place, op, value, span }
Stmt::Break { value: Option<Expr>, span }
Stmt::Continue { span }
Expr::Loop { body, span }
Expr::If { condition, then_block, else_branch, span }
Expr::Match { scrutinee, arms, span }
Expr::Range { start, end, inclusive, step, span }
Expr::Lambda { params, body, span }
Expr::Placeholder { span }
Expr::Pipeline { input, stages, span }
Expr::Call { func: String, args: Vec<CallArg>, span } // known name; not an expression callee
TypeAst::Range
LocalClosureBinding { symbol, params, inferred_result, captures } // not a general source TypeAst
FunctionDecl { result_annotation: Option<TypeAst>, contracts, ... }
Parameter { name, ty, default: Option<Expr>, ... }
```

Keep ordinary named-function calls using their existing callee representation if convenient. Extend call resolution to compiler-known **local closure symbols** and syntactically immediate lambda stages. **Do not** add general expression callees or `make_adder(10)(5)` semantics in v1; those require first-class functions.

### 16.2 Semantic passes (recommended)

1. **Lex/parse:** recognize the surface syntax and retain precise spans.
2. **Declaration collection:** register all named functions and parameters/defaults;
   allocate unresolved return type variables. Analyze valid internal signatures
   independently of export selection; compilation/layout later applies the current
   owned `ExportOptions` policy.
3. **Symbol and lexical-scope resolution:** bind reads/writes, distinguish `_` expression placeholder from `_` match wildcard.
4. **Initial type/constraint inference:** type plain expressions, functions, defaults, ranges, and match arms.
5. **Function-result inference:** solve call-graph/SCC dependencies and definite returns.
6. **Placeholder abstraction:** turn placeholder regions into typed *local* lambdas; diagnose unresolved/ambiguous parameter types and forbidden passing/returning.
7. **Local closure/capture analysis:** collect free-variable binding IDs, plan shared mutable cells, validate that all callable use remains lexically local and non-escaping.
8. **Contract validation:** type `requires`, restrict effects, attach contract metadata.
9. **Control-flow validation:** loop stack, break target/result unification, unreachable paths, `Never` handling.
10. **Proposed normalization:** normalize pipelines (named-function-only first),
    default/named arguments, and optionally match without introducing another IR.
    Implemented range For and compound assignments retain their existing typed nodes
    and checked lowering; this pass sketch does not require desugaring them.
11. **Representation/lowering analysis:** descriptor layout, closure conversion, numeric safety, local slots, effect/escape facts.
12. **Wasm layout, emission, and validation:** check declared host import and selected
    export signatures, then assemble types, any required memory, functions, exports,
    code, and optional separately designed debug/source mappings. Tables/elements
    and indirect callable dispatch are deferred; local closure helpers use direct
    calls and remain inside the existing checked-program boundary.

Some passes may need iterating to a fixed point (notably contextual lambda typing with function inference). Avoid circular dependence by separating *declaration signatures* from *body expression inference* and representing unresolved types explicitly until resolved.

### 16.3 Suggested typed core

Keep a compact internal representation that captures *semantics*, not syntax:

- `Core::CallDirect` (including known local closure helpers); defer `Core::CallIndirect`
- `Core::CaptureCellAlloc`, `Core::CaptureCellRead`, `Core::CaptureCellWrite`, `Core::CallLocalClosure`
- `Core::IfValue`, `Core::BlockValue`, `Core::MatchDecision` (or `IfValue` chain)
- `Core::Loop {break_target, continue_target, result_ty}`
- `Core::ReadPlace`, `Core::WritePlace`
- `Core::Trap {kind, source_span}`
- `Core::RangeValue` and `Core::RangeIter`
- Typed temporaries and explicit evaluation-order sequencing

A separate fully general SSA IR is **not required** at this stage. A normalized Typed AST plus structured control-flow lowering is enough for correct implementation. If closure and control-flow optimization eventually become complex, a basic-block IR may become worthwhile.

## 17. WebAssembly compilation architecture

### 17.1 Direct structured control flow

Map `if` values to `if (result T)`, `match` to optimized conditionals or `br_table` when proven safe, and loops to `block`+`loop` combinations. Use a control-context stack to resolve relative branch depths. Preserve reachability information to avoid emitting invalid Wasm stacks after unconditional transfers.

### 17.2 Value representation and ABI

| Source value | Initial internal strategy | Raw export/import status |
|---|---|---|
| `number` | `f64` | Existing ABI |
| `bool` | canonical `i32` | Existing ABI |
| `string` | `i32` descriptor handle | Existing ABI |
| `number[]` | `i32` descriptor handle | Existing ABI |
| `range` | immutable private allocated record, `i32` handle; for caches `i64` fields | All host imports and selected user range exports rejected |
| local closure binding | compiler-known helper and per-invocation shared capture environment | Not a source-level callable ABI value; cannot be imported/exported/passed/returned |

Range values retain their immutable allocated record; for caches validated fields
in scratch locals without introducing another range-value representation.
Allocation-free scalar replacement remains WP13 work. For a lexical local closure, the compiler can directly invoke its known helper and pass a pointer to shared capture cells. Neither a Wasm function table nor `call_indirect` nor the Component Model is required for v1.

### 17.3 Known-target closure invocation (no function table required)

- A locally bound closure has a statically known generated function and capture layout.
- Generate direct Wasm `call` sites to this helper, optionally adding a hidden environment pointer argument for captured cells.
- On creation and invocation, preserve shared-binding visibility and mutation exactly; no copy-on-capture of scalar bindings.
- A general function table, `call_indirect`, callable value ABI, or runtime dispatch is **not** required until first-class functions are explicitly added in a later version.

### 17.4 Memory management tradeoffs

Shared cells must remain valid while any eligible local closure can access them.
The initial implementation uses monotonic allocation and performs no reclamation;
see §10.3. Although a callable cannot escape, guest/host arrays and strings can
outlive the invocation, so the general heap pointer MUST NOT be reset at return or
trap. Future reclamation requires isolated frame storage or an equivalent proven
lifetime strategy, including recursion and reentrancy behavior. Linear memory does
not automatically reclaim cells, and traps do not provide unwind cleanup.
Capture-cell elimination/inlining remains optional and must preserve shared
mutations, side effects, and any observable allocation/trap behavior.

### 17.5 Optimizations enabled by the features

| Optimization | Feature(s) | Preconditions / warning |
|---|---|---|
| Direct known-target closure calls | Lambdas, placeholders, pipelines | Preserve shared capture-cell mutation; no indirect call to eliminate in v1 |
| Closure environment scalar replacement | Closures | No escape and alias/effect analysis proves all shared reads/writes preserved |
| Loop bounds hoisting | `for`, ranges, arrays | Iterable expression evaluated once; array length stable |
| Bounds-check elimination | Array `for` | Proven index in fixed-length bounds; preserve traps for non-loop accesses |
| Loop induction-variable optimization | Ranges | Safe-integer proof; no change to `f64` semantics |
| Branch-result stack usage | `if`, `match`, value `loop` | Compatible Wasm result representations |
| `br_table` dispatch | `match` | Dense integral exact-match cases; validate scrutinee first |
| Contract check elimination | `requires` | Prove condition true on this path; don't trust mere declaration |
| Range propagation after contracts | `requires`, ranges | Sound range analysis with invalidation after mutation/effects |
| Named/default call normalization | Default parameters | Preserve argument evaluation order and function ABI |
| Pipeline inlining/fusion | Pipelines and known collection functions | Side effects, traps, aliasing, allocation observability preserved |
| Memory traffic reduction | Compound assignments | Evaluate lvalue once, reuse address/check result |

**Binaryen (`wasm-opt`)** MAY be applied after Wasm emission for general-purpose optimizations. It cannot recover all lost high-level information (effects, contracts, closure captures, named-argument semantics), so semantic normalization and targeted analysis remain the compiler's responsibility. Always benchmark both original and optimized modules; no speedup is guaranteed.

## 18. Interaction scenarios and expected results

### 18.1 Pipeline + placeholder + mutable shared-binding closure capture

```
fn main() -> number {
    let factor: number = 3;
    let multiply = (x: number) => x * factor;
    let first: number = 10 |> multiply; // 30
    factor = 4;
    let second: number = 10 |> (_ * factor); // 40
    return first + second; // 70
}
```

The named lexical closure binding captures the **variable cell**, so changing `factor` changes later calls. The inline placeholder stage is locally invoked, not passed as a parameter. Both lower to known-target calls; no first-class function ABI is needed.

### 18.2 Range + match expression + `continue`

```
fn count_positive() -> number {
    let count: number = 0;
    for i in -2..=2 {
        let label: string = match i {
            < 0 => "negative",
            0   => "zero",
            _   => "positive",
        };
        if label == "negative" { continue; }
        count += 1;
    }
    return count;
}
```

Expected result: `3` (`0`, `1`, `2`). Parentheses may be necessary around negative range endpoints if the final parser chooses a different precedence; the illustrative grammar places unary above range.

### 18.3 Loop expression + function result inference

```
fn first_positive(values: number[]) {
    let i: number = 0;
    let result: number = loop {
        if i >= len(values) { break 0; }
        if values[i] > 0 { break values[i]; }
        i += 1;
    };
    return result;
}
```

Expected inferred return: `number`. The `loop` result is `number`. Array access stays bounds-checked; a smart optimizer may remove checks that are provably implied by `i < len(values)`.

### 18.4 Defaults + contract + named calls + pipeline

```
fn scale(value: number, factor: number = 2) -> number
    requires factor > 0
{
    return value * factor;
}

let a = scale(10);                // 20
let b = scale(value: 10, factor: 3); // 30
let c = 10 |> scale(factor: 4);   // 40
```

`scale(10, factor: 0)` traps on failed contract. Raw Wasm export `scale` still has a full-arity signature and is called with both machine arguments.

### 18.5 Two local closures sharing one mutable binding

```
fn main() -> number {
    let count: number = 0;
    let inc = () => { count += 1; return count; };
    let add_two = () => { count += 2; return count; };
    inc();
    add_two();
    return inc(); // 4
}
```

Both local closures access the **same cell** for `count`. Returning a closure from `main`, passing `inc` as an argument, or copying it into another value binding is not supported.

## 19. Required diagnostics and negative examples

| Program construct | Diagnostic |
|---|---|
| `break;` outside a loop | Break has no enclosing loop |
| `continue;` outside a loop | Continue has no enclosing loop |
| `break 1;` inside `while` or range/array `for`, even unreachable | Value break is only supported by `loop` expressions |
| `for x in 42 {}` (also bool, string, or Void iterables) | Iterable must be `range` or `number[]`, with Never compatibility |
| Ordinary or compound assignment to a for iteration binding, even unreachable | Iteration binding is read-only; a distinct inner shadowing let is mutable |
| `let x = if flag { 1 };` | Value-producing `if` requires an `else` |
| `match n { 1 => 10 }` used as a value | Non-exhaustive match; add `_` |
| `for x in 1.5..10 {}` | Runtime range-construction trap when executed, including for constant endpoints |
| `for x in 0..10 by 0 {}` | Zero step is invalid |
| `let f = _ == _;` with no type context | Cannot infer placeholder parameter types |
| `let f = existing_closure;` | Callable aliasing as a first-class value is deferred |
| `fn apply(f: fn(number) -> number) { ... }` | Callable parameters are not supported in v1 |
| `fn make() { return () => 1; }` | Returning a closure is not supported in v1 |
| `let x = loop {};` | Cannot infer a concrete storage type from a `Never` initializer |
| `fn answer() { 42 }` | Named-function tail expression must be terminated or returned explicitly |
| `let _ = 1;` | Standalone `_` cannot be a binding name |
| `f(_ + 1)` as a normal call argument | Passing placeholder closures as parameters is not supported |
| `fn f(x: number = 1, y: number) ...` | Required parameter follows a defaulted parameter |
| `f(value: 1, value: 2)` | Duplicate named argument |
| `f(y: 2, 1)` | Positional argument after named argument |
| `fn f(x: number) requires print(x) { ... }` | Impure contract expression |
| Inferred function returns `number` on one path, `string` on another | Conflicting return constraints |
| Selecting a `range`-signature function for raw export | Unsupported range host ABI; select compatible exports explicitly |
| `a %= 2` on a nonnumeric place | Compound remainder assignment requires a `number` variable or numeric array element |
| Closure writes a captured `for` iteration binding | Captured binding is read-only |
| Closure uses `break`/`continue` to target an enclosing function's loop | Control transfer cannot cross a function boundary |

## 20. Test strategy

### 20.1 Lexer and parser golden tests

Test every new token at adjacent punctuation boundaries, especially `1..10`, `1.25`, `..=`, `=>`, `|>`, nested calls, named arguments, optional function arrows, no-semicolon tail expressions, and `_` as identifier prefix versus placeholder/wildcard. Round-trip formatting is optional, but AST span consistency SHOULD be tested.

### 20.2 Semantic/unit tests

Test type unification, forward calls, recursive SCCs, mixed return types, `Never`, match exhaustiveness, compound-assignment lvalue evaluation count, shared-binding mutable capture, shadowing, array-binding reassignment visibility, invalid `break`/`continue`, pipeline-stage argument insertion, lexical default resolution, and approved pure-contract checking. Type-check unreachable and unselected source; do not evaluate it at runtime.

### 20.3 Wasm validation

Every successful compiled module MUST validate using the project's Wasm validator (or `wasm-tools validate`/equivalent). Include deeply nested control structures to catch invalid relative branch depths and stack types. Exercise known-target local closure helpers, nested and recursive *enclosing function* calls, mutable capture cells, and zero-capture closures. Test that rejected escaping callable patterns never reach codegen. Test export selection: default export-all rejects unsupported `range` signatures; explicit `--export main` includes only compatible exports.

### 20.4 Differential and property-based tests

Where possible, run a small reference evaluator or an unoptimized codegen baseline against optimized output. Compare:

- Scalar outputs including `-0`, NaN, infinities, and floating-point edge cases.
- Mutation/aliasing results for arrays.
- Side-effect order using registered host callbacks and observable logs.
- Trap/no-trap behavior and source spans.
- Range membership versus actual iteration, inclusive/exclusive endpoints, positive/negative steps.
- Capture-cell lifetime and expected memory growth under repeated local closure
  creation and recursive/reentrant calls; no reclamation is promised initially.
- Guest/host arrays and strings returned or retained from an invocation that
  allocated capture cells remain valid after subsequent calls and memory growth.
- Trap followed by another invocation does not corrupt live values; failed guest
  execution has no automatic rollback or allocator reset.

Do **not** compare NaN with ordinary `==`; use bit-preserving tests when appropriate, while recognizing that NaN payload canonicalization can vary across engines/optimizations. For floating-point algebraic optimizations, preserve the intended IEEE semantics.

### 20.5 Microbenchmarks

Benchmark the original source lowered to Core Wasm, the same artifact post-`wasm-opt`, and an equivalent Rust native program where it is a fair comparison. Separate compilation time, Wasm instantiation, repeated function call overhead, and algorithm steady-state time. Representative workloads:

1. Counter loops using `while` versus `for 0..N`.
2. Array accumulation using index loops versus direct iteration.
3. Deep nested `if`/`match` branching.
4. Direct named call versus non-capturing local lambda versus shared-capture local lambda.
5. Pipeline of known functions versus manual nested calls.
6. Contracts trivially true, runtime-checked, and statically proven.
7. Closure creation in a hot loop, measuring memory growth.

Avoid claiming a speedup without measurements; inlining and allocation costs depend on the engine and optimization profile.

## 21. Approved implementation order and dependency graph

The first five milestones are approved **in this exact order**, building only the infrastructure needed at each step rather than doing a foundational rewrite up front.

1. **Compound assignments:** all five operators including `%=`; introduce typed `Place`/single evaluation.
2. **`break` and `continue` in `while`:** introduce a minimal stack of loop targets and correct branch-depth calculation.
3. **Tail-valued blocks and `if` expressions (implemented WP3):** context-sensitive
   block results and Never; retain explicit named-function return and check all
   unreachable/skipped children.
4. **Statement/value `loop` (implemented WP4):** break-value typing, reachable
   result constraints and explicit divergence with the existing loop-target stack.
5. **Ranges and `for` (partial WP5 implemented):** eager range validation, internal
   calls, exact membership, range iteration with exact `i64` advancement, and
   `number[]` iteration with captured handle/length and live element loads. Both
   use a continue latch; adapter/interaction release gates remain pending.

Following these milestones, incremental later work (ordering within a group MAY be adjusted according to dependencies):

6. Named/default parameters with declaration-site lexical resolution and full-arity call normalization.
7. Inferred function return types with acyclic inference first, then recursive dependency handling; reject named-function implicit nonvoid tails.
8. `requires` with allowlisted pure builtins, runtime traps, and later sound range refinement.
9. `match` statements/expressions and patterns.
10. **Named-function pipelines** using the final first-argument injection/evaluation order from day one, **before** closures.
11. Locally bound anonymous closures with shared mutable capture cells and direct helper calls, **not** first-class callable values.
12. Placeholders desugared to the same local closure representation; extend pipelines to known local closures/inline stages.
13. Measure and conditionally implement loop/index optimizations, closure inlining/scalar replacement, direct-call improvements, contract refinements, and selected pipeline fusion *only if* an appropriate intrinsic API is defined.

**Dependency sketch:**

```
Place (evaluate once) -> += -= *= /= %=
Loop targets -> while break/continue -> statement/value loop -> for ranges/arrays
Block results + Never -> if expressions -> value loop + return inference
Range validation + i64 progression -> range values -> for -> membership optimizations
Known named-call binding -> named pipelines -> named/default argument integration
Local lambda parsing -> shared capture cells -> local invocation -> placeholder sugar -> extended pipelines
Contracts -> entry checks -> optional sound numeric/range refinement
```

The old proposal's generalized expression-callee calls, universal callable types, Wasm tables, `call_indirect`, escaping closures, and interprocedural closure specialization are **not prerequisites** and are deferred.

## 22. Resolved design decisions (v0.2 approvals with v0.3 clarifications)

1. **Compound assignments — YES:** implement `+=`, `-=`, `*=`, `/=`, `%=` on `number` bindings and `number[]` elements; `%` must have coherent existing binary64 remainder behavior.
2. **Exports — YES:** retain export-all by default; a selected raw export with a `range` parameter/result is an error until a host ABI is defined. Provide opt-in selection such as `--export main`, never silently omit exports. General callable parameters/results are not permitted even internally for v1.
3. **Range validation — YES:** eager finite safe-integer validation at construction
   even for empty ranges. Zero endpoints are allowed and normalized to canonical
   `+0`; both signed-zero steps are rejected. Invalid constant ranges retain the
   runtime trap rule and preceding effects, never an optimization-dependent
   compilation error.
4. **Range arithmetic — YES:** internal exact `i64` progressions and membership,
   exposing each element as `number` (`f64`) with canonical `+0`. Advancing beyond
   the endpoint finishes normally; no overflow/trap from a valid advance.
5. **Divergent initializers — YES:** inferred `let x = loop {};` is rejected for lack of a concrete type; annotated `let x: number = loop {};` is accepted, while following code remains statically checked.
6. **Unreachable code — YES:** type-check unreachable statements, skipped operands, unselected arms, and all function bodies; runtime non-execution remains intact.
7. **`by` — YES:** contextual keyword restricted to range stepping; available as a parameter and ordinary identifier elsewhere.
8. **Standalone `_` — OPTION A:** reserved for placeholder and wildcard patterns; not an ignored or normal binding name. `_temp` is unaffected. Intentional compatibility break.
9. **Contracts/defaults — YES:** allowlist only approved operators and compiler-known pure builtins, not user or host calls; defaults resolve names in declaration scope.
10. **Named-function tails — YES:** explicit `return` required for a nonvoid function result; reject a nonvoid tail expression at the function-body level without semicolon. Nested expression blocks and lambdas may return tails.
11. **Closure captures — USER OVERRIDE RETAINED:** shared binding by reference,
    with permitted outer/inner reassignment visible both ways; no capture-by-value.
    Captures retain original type and mutability, including read-only iteration
    bindings. Closures have independent return/loop contexts. Keep local-only
    callables: no callable params/results, passing, returning, aliasing, closure
    arrays, or general indirect dispatch. Initial cells use monotonic allocation
    without reclamation or general-heap rewinding; future frame storage must
    preserve persistent guest/host values and recursive/reentrant invocations.
12. **Implementation order — YES:** compound assignments → break/continue in `while` → block and `if` values → statement/value `loop` → ranges and `for`. Add minimum necessary infrastructure incrementally.
13. **Pipelines — YES:** named-function stages before closures; extend to local closures and placeholders later without changing first-argument injection or source-order evaluation.

These are normative decisions for this design revision, not an assertion that the
repository implements the extensions. Remaining implementation-level freedom
(private layouts and control-flow representation) MUST preserve these guarantees
and existing public CLI/library/browser contracts. In particular, implemented
export-selection flag spelling is not an invitation for an incidental CLI change.

## 23. Definition of done

A feature is complete only when all of the following hold:

- Surface grammar and tokenization are documented and tested.
- Type rules, evaluation order, scope, runtime behavior, and errors are specified.
- Raw AST and Typed AST paths preserve source spans and resolve bindings hygienically.
- The compiler generates validated Core Wasm without breaking the existing host ABI.
- Negative tests cover invalid types, controls, and boundary conditions.
- Side-effect and trap ordering are covered by execution tests.
- Existing examples and tests remain valid.
- Any claimed optimization has a correctness justification and benchmark evidence.
- Documentation distinguishes what is a source-language guarantee from what is merely one backend representation.

## Appendix A — Integrated sample

> Illustrative `.was` program using the selected feature families while obeying **local-only closures**. The sample assumes `print` and `len` are available as named imports or builtins, but does not assume higher-order `map` or `filter` functions exist. Named functions retain explicit `return` statements.

```
fn classify(value: number) {
    return match value {
        < 0 => "negative",
        0   => "zero",
        _   => "positive",
    };
}

fn bounded_scale(value: number, factor: number = 2) -> number
    requires factor > 0
{
    return value * factor;
}

fn aggregate(values: number[]) {
    let total: number = 0;
    for value in values {
        if value < 0 { continue; }
        total += value;
    }
    return total;
}

fn first_positive(values: number[]) {
    let index: number = 0;
    return loop {
        if index >= len(values) { break 0; }
        if values[index] > 0 { break values[index]; }
        index += 1;
    };
}

fn main() -> number {
    let values: number[] = [-2, -1, 0, 1, 2, 3];
    let multiplier: number = 3;
    let positive = (_ > 0); // local placeholder closure
    let transform = (x: number) => x * multiplier;

    for i in 0..10 by 2 {
        if i == 4 { continue; }
        if i == 8 { break; }
    }

    let base: number = aggregate(values);
    let scaled: number = base |> bounded_scale(factor: 2);
    let changed: number = scaled |> transform;
    multiplier = 4;
    let result: number = changed |> (_ + multiplier);
    let answer: number = if positive(result) { result } else { first_positive(values) };
    let label: string = classify(answer);
    let selected: bool = 4 in (0..=8 by 2);
    return if selected { answer } else { 0 };
}
```

**Export compatibility:** if a different internal function takes or returns a `range`, default export-all compilation rejects its unsupported raw ABI until the module is compiled with an explicit export list such as `--export main`.

## Appendix B — Source alignment

The existing-language baseline is the local implementation and tests, supported
by [README](../README.md), [grammar](../grammar.bnf), [agent guide](../AGENTS.md),
and [current roadmap](compilation-roadmap.md). Public project references are useful
context but may lag those files:

- https://github.com/LuanRoger/was-compiler
- https://github.com/LuanRoger/was-compiler/blob/main/grammar.bnf

The new syntax, feature interactions, and implementation plans are proposals authored for this specification.
