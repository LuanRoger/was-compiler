# Issue #10 interface gate

The user approved the WP6/WP7 shared design and parser/semantic, public typed-tree
compilation, and source-to-Wasm/runtime test seams. This is the named-argument
slice only; defaults and omitted result annotations were disabled at that gate.
Issue #12's enabled default policy and isolated #11 development are recorded in
[the follow-up handoff](wp6-wp7-development.md); the #10 evidence below is historical.

## Concrete records and identities

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
