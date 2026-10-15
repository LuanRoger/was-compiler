# Named/default call interface

The user approved the WP6/WP7 shared design and parser/semantic, public typed-tree
compilation, and source-to-Wasm/runtime test seams. This document distinguishes
the current #10/#12 named/default contract from the historical #10 named-only
gate, when defaults and omitted result annotations were disabled.
Issue #12's default policy and the original isolated #11 development are recorded
as historical worktree-development evidence in [the follow-up handoff](wp6-wp7-development.md).

## Current WP6 contract (issues #10/#12)

WP6 user-function named/mixed-prefix calls and trailing pure defaults are
implemented on main. [#13 release validation passed](was_language_extensions_plan.md#issue-13-wp6-release-validation),
including adapter coverage, freshly rebuilt browser bindings, and the full ignored
suite. Issue #11 integrates initial result inference with these interfaces;
Issue #14 completes acyclic inferred-to-inferred chains/diamonds with source-order
ties among ready functions; see [its contract](result-inference-development.md#issue-14-acyclic-chains-and-diamonds).
Owned concrete signatures and source-order output remain unchanged.
[#15 combined-feature acceptance](result-inference-development.md#issue-15-combined-nameddefault-and-inference-acceptance)
covers named/default normalization with inferred chains/diamonds, diagnostic staging,
and forged nested sequences. [#16 WP7 release validation passed](was_language_extensions_plan.md#issue-16-wp7-release-validation),
including native and current-source browser ABI/diagnostic acceptance. Every recursive
component member still needs an explicit result. WP8 whitelist preconditions are
implemented with [release gates passed](was_language_extensions_plan.md#wp8-release-validation).
WP9 match statements/values are implemented with [release gates passed](was_language_extensions_plan.md#wp9-release-validation),
including native/current-source rebuilt-browser and full ignored runtime coverage.
Match retains named/default normalization and raw full-arity ABI; it remains
outside the shared pure-expression whitelist, including forged typed defaults.
WP10 named-only pipelines are implemented, including named/default stage binding
and #27 final-Void discard support;
[release gates passed](was_language_extensions_plan.md#wp10-release-validation)
on 2026-10-14. WP11's #29–#33 local callable/shared-capture subsets are implemented,
including captured numeric compounds (#31), nested parameter/let and live callable-context
forwarding (#32), and [read-only iteration captures (#33)](local-callable-interface.md#implemented-read-only-iteration-captures-33).
[#31 validation passed](local-callable-interface.md#issue-31-validation-2026-10-14);
[#33 validation passed](local-callable-interface.md#issue-33-validation-2026-10-15).
Nested/transitive read-only iteration captures compose with captured compounds and
invocation-local lifetimes; [#35 WP11 release gates passed](local-callable-interface.md#issue-35-wp11-release-validation-2026-10-15).
See [the interface](local-callable-interface.md) for current support and historical
subset evidence. WP12 remains deferred. WP13 requires separate
authorization; later packages
reuse the approved interfaces only after their ordered integration/release gates.

### Public AST migration

- Raw calls now use `Vec<ast::CallArgument>`, not `Vec<Expr>`. Wrap positional
  operands with `name: None`, `value`, and the written argument's byte `span`;
  named operands use `name: Some(formal_name)`.
- Raw `ast::Param` requires `default: Option<DefaultExpression>`; existing
  constructors use `None`. `DefaultExpression { value: Expr, span: Span }` keeps
  the declaration expression's origin. Raw `Function.return_type` is now
  `Option<Type>`: existing constructors wrap annotations in `Some(type)`;
  `None` requests inference, including acyclic chains/diamonds. Typed results remain
    concrete `Type`; consumers need no unresolved-type representation. No callable or
    default metadata is added to raw exports, and full arity/result ABI is unchanged.
- Typed parameters/signatures stay concrete and full-arity with no default
  metadata. Ordinary typed `Call` still takes `Vec<TypedExpr>`. Exhaustive typed
  expression consumers must handle `NormalizedCall(TypedNormalizedCall)` and
  visit its ordered binding values, then its final call.

### WP10 pipeline migration (issue #25)

Raw expression consumers must handle `Expr::Pipeline { input: Box<Expr>,
input_span: Span, stages: Vec<PipelineStage> }` and visit input then stage arguments.
`PipelineStage { name: String, args: Vec<CallArgument>, span: Span }` retains the
written target/arguments. Parser-produced pipelines have at least one stage; bare
and empty-parenthesis stages have empty argument vectors. Input spans include
parentheses; stage spans exclude the operator/trivia and include closing parentheses.

Typed consumers add the unit origin `CallBindingOrigin::PipelineInput`; there is
no typed pipeline node. Each stage becomes an existing `TypedNormalizedCall`, with
the preceding input/normalized stage stored once as its first binding and mapped
to formal 0. Written explicit indices remain zero-based, starting after injection.
Injection spans cover the preceding pipeline prefix. The final call stays direct,
canonical and full-arity; walker/planning policies retain their existing traversal.
Checked lowering verifies unique first injection, formal-0 mapping, ownership,
IDs, scope, exact types/results, Never flow, and existing default-whitelist rules.
Injection initializers, like explicit initializers, cannot read owned bindings.
Void is not an injectable value.

#26 extends #25's value-producing user stages with the ordinary named/mixed-prefix
and trailing-default binder. Injection occupies formal 0 even when defaulted;
written positionals start at formal 1, and naming formal 0 is a duplicate. Bare and
empty-parenthesis stages both use omitted remaining defaults. Structural checks
precede injected-type and written-expression checks; defaults then instantiate in
formal order with declaration spans and resolved earlier-formal remapping.
Injection supplies an earlier formal without exposing generated bindings to source
lookup. Written explicit indices remain zero-based and argument spans unchanged.

#27 completes source support for final declared-Void stages only when the result
is discarded. Direct expression statements supply discard context, which propagates
through discarded block tails, if branches and match arms; parentheses are
transparent. It does not propagate into operands, call arguments, pipeline inputs,
initializers, returns or value breaks. Independent expression statements inside
such contexts still discard their own results. Expression-form if/match retain
existing structural joins, including exact Void/value distinctions and Never
compatibility; outer discard does not bypass those checks.

Every nonfinal declared-Void stage is rejected, even after Never. Source errors use
`SemanticError::InvalidPipeline` with the stage span and the messages
`a nonfinal stage cannot return void` or
`a final void stage requires a discarded result`. Skipped/unreachable children
remain checked, with ordinary binding/type diagnostics and their ordering preserved.

No typed wrapper or public discard flag is added. Existing normalized-call support
for Void direct calls in forged typed input is unchanged; source-semantic discard
is not wholly reconstructible from that input. Backend injection ownership, type,
result and flow checks remain independent, not complete source revalidation.
Stages remain user-only; ordinary named/default calls and the full-arity ABI are
unchanged. WP10 implementation is complete, and
[#27 release gates passed](was_language_extensions_plan.md#wp10-release-validation),
including freshly rebuilt browser bindings and the full ignored runtime suite.
The plan's [#25](was_language_extensions_plan.md#issue-25-subset-validation-2026-10-14)
and [#26](was_language_extensions_plan.md#issue-26-subset-validation-2026-10-14)
records validate historical subsets, not the current final-Void release.

### WP8 contract constructor migration

Raw `ast::Function` also requires `contracts: Vec<ast::ContractExpression>`;
typed `TypedFunction` requires `contracts: Vec<TypedContract>`. Existing constructors
supply `vec![]`. Each ordered record owns `value` and an expression-only UTF-8 byte
`span`, including parentheses but excluding `requires` and surrounding trivia.
Typed contract reads retain callee formal identities, not invocation-local binding IDs.

Contracts run at callee entry after explicit arguments in written order, omitted
defaults in formal order, and validation of every callee parameter. False clauses
trap using ordinary Wasm `unreachable` before later clauses and body effects; they
provide no inference refinement or rich runtime metadata. Raw exports remain
full-arity and do not synthesize defaults. See [spec §13.1](was_language_extensions_spec.md#131-syntax).

### Binding, scope, and checked lowering

The records below remain unchanged from #10, but `Default { formal }` is now
accepted. Explicit origins are contiguous zero-based written-argument indices;
all explicit bindings precede defaults. Default origins index their actual
formal, match the formal mapping, and increase strictly. The mapping is a
bijection over owned bindings; the final call reads them in formal order.

Explicit initializers cannot read any binding owned by their enclosing sequence.
Default initializers may read only owned earlier-formal bindings, already bound
by an explicit argument or an earlier default. References are remapped by resolved
formal identity, never by caller names or copied callee-local IDs. Generated IDs
come from the caller's per-function allocator and do not enter source lookup.
All bindings leave scope on sequence exit; normalization adds no function or
loop context. The final call has the signature's result; the enclosing sequence
may be Never when explicit argument evaluation cannot complete.

Checked lowering verifies origins, order, IDs, mapping, types, default whitelist
and earlier-formal reads, including skipped/unreachable children. Public metadata
is mutable: these checks do not prove that a forged default came from a source
declaration or constitute complete source-semantic revalidation.

### Defaults, diagnostics, and ABI

Every declared default is checked before function bodies, even when unused or
explicitly supplied. Defaults are trailing-only, exact-typed, and restricted to
scalar literals, earlier-formal reads, existing type-valid unary/arithmetic/
comparison/equality/logical operations (including string/array equality), and
positional builtin `len(array)`. Reject indexing, array/range construction,
membership, blocks/if/loops/match, writes, and user/host calls, even in skipped operands.
An earlier handle read may copy an array/string/private range handle; it does not
allocate or snapshot its contents. Pure defaults may observe shared array contents
or trap. See [spec §14.2](was_language_extensions_spec.md#142-default-expression-restrictions-and-scope).

Resolve lexical targets first, reject named host/builtin calls, scan written
structural errors, check missing required formals, then analyze explicit expressions
in written order and instantiate omitted defaults in formal order. Defaults retain
declaration spans; normalized explicit expressions retain argument spans. Ordinary
full-positional calls retain their direct path, legacy diagnostics and bytes even
when their target declares defaults. Default literal syntax/overflow is checked
semantically; ordinary body literal staging remains unchanged.

Defaults run per source invocation after all normally completing explicit arguments.
Scalars are captured values; handles remain shared. Transfers/traps prevent later
evaluation and preserve caller context. Raw exports still require every machine
argument: neither Node nor browser calls synthesize defaults, and no default
metadata is emitted. Private range host/selected-export rejection is unchanged.

## Historical #10 gate

The remaining sections record the narrower named-only interface and its evidence
at #10, before defaults were enabled. Their default rejection and future metadata
wording are historical, superseded by the current contract above and #12 handoff.

### Concrete records and identities

- `ast::CallArgument { name: Option<String>, value: Expr, span: Span }` replaces
  raw call operands. Spans cover each entire written argument, including its name.
- `TypedExprKind::NormalizedCall(TypedNormalizedCall)` owns
  `bindings: Vec<TypedCallBinding>`, `formal_to_binding: Vec<VariableId>`, and
  `call: Box<TypedExpr>`. The last expression is a canonical ordinary user `Call`.
- `TypedCallBinding { id: VariableId, ty: Type, value: TypedExpr,
  origin: CallBindingOrigin, span: Span }` records written-order storage.
  `CallBindingOrigin::{Explicit { argument: usize }, Default { formal: usize }}`
  distinguishes origins; this slice rejects Default bindings.
- Generated IDs come from the caller's existing per-function VariableId allocator,
  are unique among all declarations, and are not introduced into source name lookup.
  Reserve a binding ID before analyzing its initializer. Initializers cannot read
  any generated binding of their enclosing sequence. Bindings become readable only
  for the final call, are read-only there, and leave scope on sequence exit.
- The formal mapping is a bijection onto this sequence's bindings. Final arguments
  are typed Identifier reads of those IDs in formal order, with exact formal types.
  The final target/result must agree with the checked user signature; its own
  result is the signature result, while the enclosing sequence may be Never if
  its ordered initializers cannot complete. No new loop/function context is created.

## Declaration and diagnostic handoff

The existing private FunctionSignature gains formal names for user functions;
non-user targets have no formal-name contract. Resolve lexical shadowing first.
Named calls scan written structural errors, then missing formals, then expressions.
Semantic variants: NamedArgumentsNotSupported, UnknownArgument, DuplicateArgument,
TooManyPositionalArguments, PositionalAfterNamed, MissingArgument (each carries
name/span, with formal names where relevant). Ordinary positional calls retain
ArgumentCountMismatch and legacy enclosing-statement expression spans.
Backend uses InvalidNormalizedCall { reason: &'static str, span: Span } for sequence
structure/ownership errors and existing errors for malformed child expressions.

For later tickets, raw parameter metadata is an optional declaration-origin
`DefaultExpression { value: Expr, span: Span }`; raw result metadata is Option<Type>.
These shapes are frozen design, not fields enabled by #10. Declaration-checked
pure defaults will remap resolved earlier-formal identities to invocation bindings,
never names or copied callee IDs. Return/dependency policy consumes the existing
private Flow completion summary and concrete Type/ExprResult checker boundary;
no public flow or unresolved-type annotation is introduced here.

## Exclusive writers

| Owner | Files |
| --- | --- |
| Integrator | ast.rs, typed_ast.rs, parser.rs, semantic.rs, grammar.bnf, codegen/mod.rs, frontend test migrations, tests/named_arguments.rs, documentation |
| Backend agent | src/codegen/** except mod.rs; tests/named_call_lowering.rs |
| Runtime test agent | tests/named_call_runtime.rs |

No default/inference policy agent is dispatched for this narrower ticket. Compiler
planning remains sequential. Each writer tests their own slice red before green.
Legacy fixtures are not regenerated.

## Validation evidence

Issue #10's incremental slice passed these offline gates on 2026-10-12:

- `cargo check --offline --all-targets`
- `cargo test --offline --test named_arguments` — 9 frontend tests passed;
  the first reordered-call test failed at the named colon before implementation.
- `cargo test --offline --test named_call_lowering` — 11 public typed-tree and
  decoded-Wasm tests passed.
- `cargo test --offline --test named_call_runtime -- --include-ignored` — 5 tests
  passed, including 4 normally ignored bounded Node tests at all optimization levels.
  Source runtime cases failed at named syntax before frontend integration.
- `cargo test --offline` — complete default suite passed; Node/browser cases
  retained their normal ignored policy.
- `cargo clippy --offline --all-targets -- -D warnings` — passed.
- `wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline` — rebuilt current source.
- `node tests/browser.cjs pkg` — passed against rebuilt bindings.
- `cargo test --offline -- --include-ignored` — complete runtime/browser suite passed.
- `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` — passed.
- Scoped Rust formatting and `git --no-pager diff --check` — passed.

Parallel standards/spec review found no documented breaches or correctness/spec
findings. One low-priority duplicated-slot-reservation heuristic was left unchanged;
separate planner policies remain intentional. These results do not complete WP6
(defaults) or WP7 (result inference).
