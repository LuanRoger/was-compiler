## Parent

Part of #28 — Implement WP11 local shared-binding closures. Follow the complete parent contract and the callable interfaces established by the first slice.

## What to build

Let local closures capture creator parameters and ordinary let bindings by reference to original shared cells. Outer and inner reassignment, multiple closures sharing a binding, shadowing, and numeric-array reference changes must work across source analysis, checked lowering, and execution for every existing concrete value type.

## Acceptance criteria

- [ ] Analyze captures by owner-qualified original binding identity for number, bool, string, range, and numeric arrays. Multiple closures share one cell per live binding; later shadowing never retargets captures.
- [ ] Rewrite outer/inner reads and ordinary reassignment to the same cell. Preserve array-handle reassignment, alias mutation, expression evaluation order, exact types, and initialized-binding checks.
- [ ] Allocate captured parameter cells in formal order after every explicit parameter validation and ordered contract; contracts read incoming parameters before promotion. Allocate captured lets after completing initializers, with no cell allocation after a noncompleting initializer.
- [ ] Eager promotion can allocate even when a later capturing declaration is skipped. Allocate an environment only when its declaration executes, without executing its body. Allocation may trap/grow memory without rolling back earlier effects.
- [ ] Use private eight-byte-aligned eight-byte cells and pointer slots, f64/i32 storage by language type, canonical original-owner/declaration capture order, checked arithmetic, and hidden first i32 environment parameters only where context is needed. Keep all production layout constants centralized privately.
- [ ] Activate existing memory helpers/exports/collision checks when captures require memory. Preserve existing helper ordering, user/import IDs, external signature rules, and deterministic output; add no public environment descriptors or helper exports.
- [ ] Extend existing checked lowering to verify capture completeness, canonical order, uniqueness, original owner/type/mutability, lexical availability/initialization, and access types. Reject forged capture metadata rather than trusting public annotations or repairing it silently.
- [ ] Explicitly retain staged limitations for nested capture forwarding, context-requiring callable dependencies, captured compound places, and captured iteration bindings until their respective slices land. Do not weaken final parent semantics or claim release readiness.
- [ ] Add frontend acceptance/rejection, forged-tree checks, wasmparser validation, independent cell/environment/signature decoding, and ignored bounded runtime tests for all concrete captured types, multiple closures, outer/inner mutation, shadowing, parameter/initializer effects, array rebinding/aliasing, and declaration-without-body-execution.
- [ ] Verify allocation and entry-validation/contract precedence using decoded ordering and observable effects/traps where feasible; compare all optimization levels, deterministic bytes, and focused CLI/core parity.
- [ ] Document monotonic space growth, allocation trap/growth points, and memory view refresh. Run focused offline validation and report missing prerequisites without installing dependencies.
