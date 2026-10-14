# Agent guide

This guide applies to the entire repository. Keep it current when changing the
architecture, supported language, public contracts, or validation workflow.

## Purpose and working rules

`was-compiler` compiles a small, statically typed language to raw WebAssembly using
`wasm-encoder`. The priorities are understandable compiler stages, deterministic
output, explicit host integration, and maintainable, idiomatic Rust. It is not an
execution engine. Host registration declares imports; it does not install callbacks.

- Read the affected implementation and tests before editing. Keep changes focused;
  preserve user work and do not stage or commit unless requested.

- Distinguish behavior-preserving refactors from language, ABI, and optimization
  changes. Do not introduce speculative frameworks, another IR, or dependencies.
- Keep the core library portable to native targets and `wasm32-unknown-unknown`.
  Filesystem access, CLI dependencies, and JavaScript bindings belong in adapters.
- Use offline build commands. If dependencies or tooling are unavailable, report
  the prerequisite rather than silently installing or fetching anything.
- Before parallel extension work, read extension plan §4 for approved development
  waves, shared interface gates and exclusive file ownership. Parallel development
  retains ordered package integration/release gates and sequential compiler planning.
- Treat implementation and tests as evidence of current behavior. Consult
  [README.md](README.md), [grammar.bnf](grammar.bnf), and the documents below.
  The BNF is not a perfect lexer specification: actual strings accept raw Unicode,
  and whitespace recognition is broader. Reconcile differences deliberately.

| Reference | Responsibility |
| --- | --- |
| [Compilation roadmap](docs/compilation-roadmap.md) | Architecture, milestones, acceptance criteria |
| [Memory ABI](docs/memory-abi.md) | Authoritative descriptor, allocation, ownership, and host contract |
| [Limitations](docs/limitations.md) | Implemented restrictions versus possible alternatives |
| [Runtime integration](docs/runtime-integration.md) | CLI, raw execution, imports, explicit marshaling |
| [Browser guide](docs/browser.md) | Compiler-in-Wasm API, packaging, playground integration |
| [Extension specification](docs/was_language_extensions_spec.md) | Extension semantics and implemented-versus-proposed scope |
| [Extension plan](docs/was_language_extensions_plan.md) | Ordered work packages, representation gates, required tests |

## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues. Before tracker operations, read
`docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage labels. Before triaging, read
`docs/agents/triage-labels.md`.

### Domain docs

Use a single-context layout. Before codebase exploration, read
`docs/agents/domain.md`.

## Compiler pipeline and ownership

```text
source -> lexer -> parser AST -> semantic analysis + host signatures
       -> TypedProgram -> optimization (currently no-op)
       -> ABI/module layout -> checked function-body lowering
       -> private ValidatedProgram -> wasm_encoder section assembly -> Wasm bytes
```

The library entry points are `Compiler::analyze`, `Compiler::compile`, and
`Compiler::compile_source`. Analysis produces an owned typed program; compilation
uses its host-declaration snapshot, not the compiler's subsequently edited registry.
`analyze` is independent of optimization settings. Direct `codegen::compile`
bypasses the compiler's optimization/options orchestration.

| Location | Responsibility |
| --- | --- |
| `src/lib.rs`, `src/compiler.rs` | Public API, configuration, pipeline, stage errors |
| `src/tokens.rs`, `src/lexer.rs` | Tokens and UTF-8 byte-offset spans |
| `src/parser.rs`, `src/ast.rs` | Recursive-descent parsing and untyped syntax |
| `src/semantic.rs`, `src/typed_ast.rs` | Types, scopes, loop targets, return analysis, resolved semantic IDs |
| `src/semantic/defaults.rs`, `src/semantic/pure_expression.rs`, `src/semantic/call_binding.rs` | Declaration-checked defaults/contracts, shared whitelist, and hygienic named/default binding |
| `src/number_literal.rs` | Shared literal syntax/finite-representability rules with stage-specific errors |
| `src/match_patterns.rs` | Shared semantic/backend numeric-pattern, domain and exhaustiveness checks |
| `src/flow.rs` | Private completion sets shared by semantic/backend flow summaries |
| `src/host.rs`, `src/exports.rs` | Ordered host declarations, private external-type eligibility, and owned export selection |
| `src/optimization.rs` | Borrowed `Cow<TypedProgram>` no-op; future transformation boundary |
| `src/codegen/abi.rs` | Internal language-to-Wasm signatures, separate external signature checks, and named memory-layout constants |
| `src/codegen/layout.rs`, `src/codegen/indices.rs` | Signatures, callees, exports, distinct Wasm index types |
| `src/codegen/walker.rs` | Shared structural traversal for separate planning policies |
| `src/codegen/locals.rs`, `src/codegen/temporaries.rs` | Local-slot and temporary planning |
| `src/codegen/function.rs` | `BodyEmitter`, scoped initialized bindings, private emission context |
| `src/codegen/expression.rs`, `src/codegen/statement.rs` | Expression and statement lowering |
| `src/codegen/place.rs` | Shared checked local/index places, ordinary and compound assignment |
| `src/codegen/control_flow.rs`, `src/codegen/call.rs`, `src/codegen/values.rs` | Structured control flow, calls, literal values |
| `src/codegen/memory.rs`, `src/codegen/numeric.rs` | Memory planning/helpers and optional floating remainder helper |
| `src/codegen/range.rs` | Private range discovery, construction, record-validation, and exact membership helpers |
| `src/codegen/match_.rs` | Private checked match lowering, scrutinee cache, scoped arms and ordered comparison regions |
| `tests/match_frontend.rs`, `tests/match_lowering.rs` | Match syntax/inference and forged-tree/decoded lowering contracts |
| `tests/match_runtime.rs`, `tests/match_adapters.rs` | Bounded match effects and CLI/current-source browser compatibility |
| `src/codegen/validation.rs`, `src/codegen/mod.rs` | Checked internal boundary and section assembly |
| `src/diagnostic.rs` | Shared native/browser diagnostic rendering |
| `src/main.rs`, `src/cli.rs` | Thin native entry point and clap/file/output adapter |
| `browser/src/lib.rs` | Separate wasm-bindgen compiler adapter using the same core pipeline |
| `runtime/`, `examples/browser/` | Optional Node execution helpers and browser playground |
| `tests/`, `src/codegen/tests/`, `src/codegen/memory_tests/` | Integration contracts and focused backend suites |

`src-python/` contains separate Python parser/REPL code. It is outside the Cargo
pipeline and is not authoritative for the Rust compiler's language behavior.
`target/` and `pkg/` are generated output, not implementation sources.

### Preserve the internal boundaries

- The planning walker is sequential and structural, not semantic analysis or code
  emission. Keep exhaustive node traversal centralized and policies separate.
  Locals traverse expressions to collect nested declarations, without validating
  expression contents; remainder discovery stops at the first `%` expression
  or `%=` statement before visiting its children. Successful block-exit hooks are not unwind cleanup.
- `ValidatedProgram` privately borrows the original typed tree, completes layout
  first, then checks/lowers every user function in source order and caches encoded
  bodies. Module assembly consumes that checked state. Do not clone the AST or add
  a second independently implemented validator to accomplish the same checks.
- Public typed nodes, IDs, and annotations are mutable and can be forged. Retain
  backend checks; do not assume every caller went through semantic analysis.
  The internal boundary is not a public checked-AST API or a guarantee that every
  arbitrary malformed tree produces a recoverable error.
- Keep `CompilerOptions` small and `Copy`; owned `ExportOptions` stays separate.
  Optimization levels `None`, `Basic`, `Standard`, and `Aggressive` currently have
  identical output and diagnostics. Do not add optimization as incidental cleanup.

## Current language support

- WP6 implements user-function named/mixed-prefix calls and trailing pure defaults;
  #13's release gates passed with [recorded evidence](docs/was_language_extensions_plan.md#issue-13-wp6-release-validation).
  WP7 implements acyclic chain/diamond result inference and its named/default
  interactions (#11/#14/#15); [#16 release gates passed](docs/was_language_extensions_plan.md#issue-16-wp7-release-validation),
  including current-source native/browser ABI and diagnostic coverage. Every recursive
  component member still needs an explicit result in the initial release. WP8 implements
  whitelist `requires` contracts; [release gates passed](docs/was_language_extensions_plan.md#wp8-release-validation).
  WP9 match statements/values are implemented with frontend/backend integrated;
  [release gates passed](docs/was_language_extensions_plan.md#wp9-release-validation).
  WP10 named-only pipelines implement positional/named/default stage binding
  and final-Void discard support (#27);
    [release gates passed](docs/was_language_extensions_plan.md#wp10-release-validation) on 2026-10-14.
  WP11 implements capture-free local callables (#29) and immediate-creator shared
  parameter/let captures (#30), plus captured numeric compounds (#31).
  [#30 subset validation passed](docs/local-callable-interface.md#issue-30-validation-2026-10-14)
  before #31; [#31 validation passed](docs/local-callable-interface.md#issue-31-validation-2026-10-14).
  Nested forwarding, context-requiring dependencies, captured `for` bindings,
  full WP11 release and WP12 remain deferred. WP13
  optimization requires separate authorization.
  Preserve `target/wp7-inference` when cleaning generated output. Before changing defaults, call binding, or
  normalized lowering, read [the handoff](docs/wp6-wp7-development.md) and
  [named-call interface](docs/named-call-interface.md) for origins, ownership,
  diagnostic staging, whitelist, and earlier-formal read rules.

- Types: `number`, `bool`, `string`, `number[]`, `range`, and `void`. Parameters
  require explicit types. Omitted function results infer concrete types from
  reachable explicit returns, or void without value-return syntax. Recursive
  components require all results annotated, even across annotated barriers.
  Acyclic inferred-to-inferred calls use dependency scheduling with source-order
  ties among ready functions. Never-only value-return syntax requires an annotation.
  All unreachable/skipped children remain checked against final concrete results.
  Before changing result inference, read [the #14 contract](docs/result-inference-development.md#issue-14-acyclic-chains-and-diamonds)
  for graph separation, diagnostic staging, and concrete/source-order boundaries. Locals accept `let name = initializer;` or
  `let name: type = initializer;`; every declaration requires an initializer.
  Value-producing expressions infer concrete existing types; `[]` infers `number[]`
  because only numeric arrays exist. An inferred `Never` initializer is rejected;
  an explicitly typed divergent initializer is accepted. No coercions are added.
- Local inference adds no source `i32` type, promotions, implicit coercions,
  parameter inference, or whole-program type unification. `number` remains `f64`.
- Named functions support forward, nested, self-recursive, mutual, and registered
  host calls. Statements include `let`, variable/index assignment, expression
  statements, `return`, `if`/`else`, `while`, range/array `for`, and `loop`. Bare
  `break;` exits while/for and statement loops; `break value;` exits value loops.
  Continue reevaluates a while condition or advances a for before retesting.
  Statement-only `+= -= *= /= %=` accept `number` variables and numeric-array elements, not whole arrays.
- Expressions support literals, parentheses, unary `-`/`!`, arithmetic
  `+ - * / %`, numeric comparisons, boolean equality and short-circuit `&&`/`||`,
  string equality, numeric-array equality, indexing, and built-in `len(array)`.
  Tail-valued blocks and `if`/`else if` expressions support all concrete result
  types. Value-context if requires else and exact branch joins, allowing `Never`.
  Statement-if may omit else and discards branch tails. WP9 adds scoped match
  statements/values; read its frozen contract below before changing them.
- WP5 implements range construction/internal calls, exact numeric membership,
  and range/array statement iteration:
  `start..end` / `start..=end`, optional contextual `by step`, default `+1`.
  Precedence is below arithmetic and above comparisons, non-associative. Range
  values work with locals/assignment, internal signatures, blocks/if/value loops,
  and exact/Never joins; equality is rejected. `number in range` yields `bool` at
  comparison precedence with existing left associativity; `in` is reserved.
  Reserved `for name in iterable { body }` accepts `range` or `number[]` with
  Never compatibility and introduces a read-only `number` binding; an inner
  shadowing `let` remains mutable. Issue #6 completes adapter/interaction implementation;
  all WP5 release gates passed, with exact evidence in
  [extension plan §5](docs/was_language_extensions_plan.md#5-documentation-and-validation-workflow).
  Before changing ranges or for, read extension spec §§5–6 and the
  [private record contract](docs/memory-abi.md#private-range-record).
- Strings are immutable raw UTF-8 values. Numeric arrays have fixed lengths and
  mutable shared storage; every evaluated array literal allocates fresh storage.
- `//` line comments and non-nesting `/* ... */` block comments are supported
  between tokens anywhere whitespace can appear; delimiters inside strings remain text.
- Statement/value `loop` supports all concrete result types through value breaks;
  no reachable exit yields Never. Explicit divergence is allowed in nonvoid
  functions. No `foreach`, loop labels, value breaks in while/for, string
  escapes, exponent literals,
  `.5`, `1.`, string concatenation/indexing/`len`, or general postfix
  chaining. Source indexing starts from an identifier.
- No other array element types, modules, first-class functions, generics, free, or garbage
  collection. Consult the limitations document before claiming support.
- Source parsing requires at least one function; the lower-level typed API can
  represent an empty program. An empty `void` body is valid.

## Compatibility contracts

### Local callables and shared captures (WP11 subsets, #29–#31)

- Before changing closure syntax, inference, capture ownership/storage, lexical calls,
  helper planning or diagnostics, read [the subset and public migration](docs/local-callable-interface.md)
  and [spec §10](docs/was_language_extensions_spec.md#10-feature-specification-anonymous-functions-and-local-closures).
- Dedicated typed declarations own helper bodies; lexical calls use program-wide
  source-preorder callable IDs. Return/loop/local/temporary contexts are owner-local.
  Module discovery enters bodies; creator-local planning excludes nested helpers.
- Check/cache every borrowed helper body through `ValidatedProgram`, after layout
  and creator checks. Emit direct unexported helpers after runtime helpers. Earlier
  context-free dependencies remain lexical permissions, not callable values.
- Context-free declarations execute no body, allocate no environment and add no
  hidden argument or callable-only memory. Preserve ordinary handle validation.
- #30 captures immediate-creator parameters/ordinary lets of all concrete types.
  Use owner-qualified `TypedExprKind::Captured` for helper reads/places/array bases;
  creator accesses remain `Identifier`. Preserve original type, initialization,
  shadowing and canonical owner/declaration ordering, independent of forged sparse IDs.
- Allocate captured parameter cells in formal order after validation/contracts,
  captured let cells after completing initialization, and nonempty environments
  only when declarations execute. Use the [private uniform eight-byte layout](docs/memory-abi.md#private-closure-storage-30),
  monotonic allocation and hidden first `i32` only for capturing helpers. Allocation
  can grow memory/trap without rollback; hosts refresh views after possible growth.
- #31 supports all five numeric compounds on captured scalar/indexed places and
  creator-side promoted bindings, including before the capturing declaration.
  Use the existing checked place path: retain the scalar cell address and old `f64`
  on the stack; save indexed element address/old value in existing scratch before
  RHS effects. Ordinary indexed assignments retain their checked pre-RHS address.
  Preserve numeric/mutability checks, trap order and exact `%=` behavior.
- Nested helpers may capture their immediate creator's own bindings. Reject ancestor
  forwarding, external context-requiring callable dependencies and captured `for` bindings with
  [stable subset reasons](docs/local-callable-interface.md#implemented-shared-capture-subset-30).
  Full WP11 release remains deferred. [#30 evidence](docs/local-callable-interface.md#issue-30-validation-2026-10-14)
  covers shared storage before #31, not captured compounds; [#31 validation passed](docs/local-callable-interface.md#issue-31-validation-2026-10-14).
  Historical #29 results cover only capture-free callables.
- Keep `tests/closures_frontend.rs`, `tests/closures_lowering.rs`,
  `tests/closures_runtime.rs`, and `tests/closures_adapters.rs` at the confirmed
  compiler/typed-input/adapter seams. Runtime cases remain ignored by default.

### Named-only pipelines (WP10 implemented, #27 release gates passed)

- Before changing pipeline binding, discard propagation, diagnostics or lowering,
  read [the normative rules](docs/was_language_extensions_spec.md#12-feature-specification-pipeline-operator-)
  and [public migration](docs/named-call-interface.md#wp10-pipeline-migration-issue-25).
  The plan's #25/#26 validation records are historical subset evidence, not #27 release evidence.
- Raw pipelines retain input/prefix spans and ordered named stages for inference
  dependency discovery, whitelist rejection and narrow diagnostics. Typed stages
  nest existing normalized direct calls; `PipelineInput` must be unique, first,
  mapped to formal 0, and cannot read sequence-owned bindings. Written explicit
  indices still start at zero. Preserve ordinary named/default calls and independent
  structural/planning policies.
- Stages reuse ordinary positional-prefix/named/default binding with
  formal 0 already occupied, even when defaulted; naming it is a duplicate. Bare
  and empty stages both use remaining defaults. Keep declaration-scope remapping,
  earlier-formal reads, written argument spans and declaration-default spans.
  Preserve caller control context, once-only input/written/default order, parameter
  validation and ordered contracts. Pipelines stay outside default/contract whitelists.
- Final declared-Void stages require a discarded result: direct expression statements
  and propagated discarded block tails/if branches/match arms; parentheses are transparent.
  Operands, call arguments, pipeline inputs and value-producing contexts receive no
  permission from an outer discard. Nonfinal declared Void is rejected even after Never.
  Preserve structural joins for expression if/match. `InvalidPipeline` messages are
  `a nonfinal stage cannot return void` and `a final void stage requires a discarded result`.
- Discard context is source-semantic, with no typed wrapper or public discard flag.
  It is not wholly reconstructible from forged typed input; retain independent backend
  injection ownership, type and flow checks. Ordinary calls, defaults and ABI are unchanged.
- `tests/pipelines_frontend.rs`, `tests/pipelines_lowering.rs`,
  `tests/pipelines_runtime.rs`, and `tests/pipelines_adapters.rs` own source,
  forged-tree/decoded, bounded-effects and native/current-source browser coverage.
  Keep runtime/browser cases ignored by default. WP10 is implemented;
  [#27 release validation passed](docs/was_language_extensions_plan.md#wp10-release-validation),
  including current-source rebuilt-browser and full ignored runtime coverage.

These are deliberate semantics, not implementation accidents. Preserve them in
refactors; specify and test any intentional behavior change explicitly.

### Function preconditions (WP8)

- Before changing contracts or their shared whitelist, read extension spec §13 and
  [WP8's frozen contracts](docs/was_language_extensions_plan.md#wp8--whitelist-contracts-spec-13).
- Raw/typed functions carry ordered `contracts` records with `value` and expression-only
  byte `span`; existing constructors supply an empty vector. `requires` is reserved
  for source/host-language names, while host module/field names remain unrestricted.
- Declaration checking handles each function's parameter types/defaults then contracts
  before inference/body checks. Contracts require bool, read all formals, and preserve
  the shared whitelist and skipped-child checks. They never alter result inference or
  definite return, even when constant false.
- All callee parameters are validated after caller defaults and before ordered contracts;
  false traps before later clauses/body. This entry path covers exports, internal calls,
  and recursion. Raw exports stay full-arity. Keep checks at every optimization level.
- The structural walker visits contracts before bodies. Preserve separate planners and
  checked lowering's parameter-only read/type checks for mutable public typed trees.
  `tests/contracts_frontend.rs`, `tests/contracts_lowering.rs`,
  `tests/contracts_runtime.rs`, and `tests/contracts_adapters.rs` cover these boundaries;
  runtime/browser cases retain the default ignored policy.

### Match statements and values (WP9)

- Before changing match syntax, patterns, flow, inference or lowering, read
  [spec §8.2–§8.4](docs/was_language_extensions_spec.md#82-match-expression-syntax)
  and [WP9's frozen interview](docs/was_language_extensions_plan.md#wp9--match-statementsvalues-spec-8-implemented).
  Frontend/backend are integrated and [release gates passed](docs/was_language_extensions_plan.md#wp9-release-validation).
- Match requires nonempty comma-separated `pattern => expression` arms, with an
  optional trailing comma even for block arms. Each arm is scoped independently;
  declarations never leak between arms. Transfers need a block RHS and retain
  enclosing function/nearest-loop targets.
- Completing scrutinees are number/bool/string only. Patterns are `_`, exact scalar
  literals, numeric comparisons and constant intervals. Numeric constants are
  finite decimal literals with optional unary minus, not general expressions.
  Fractional endpoints are allowed without safe-integer restrictions. Intervals
  use bound comparisons, not range progressions; reject `by`. Reversed and
  equal-exclusive intervals never match; signed zeros compare equal and NaN
  reaches `_`. String matching uses content equality.
- Both contexts require exhaustive coverage: number/string need final `_`; bool
  needs both literals, either literal then `_`, or `_` alone. Wildcard is unique
  and last. Duplicates/overlaps remain accepted without new warnings. Cache the
  scrutinee once; execute only the first matching arm.
- Check every arm conservatively, including constant, duplicate, covered and
  unreachable cases. Value arms structurally join exact concrete types or all
  Void with Never compatibility; statement arms discard heterogeneous results.
  Never scrutinees still require valid patterns/coverage/structural joins; their
  non-wildcard patterns establish one domain, wildcard-only needs none. Compose
  flow after the scrutinee, so no arm supplies reachable flow after Never. With
  a completing scrutinee every arm contributes to inference/transfer summaries.
- Bare match follows bare-if statement/tail classification: statement bodies
  discard results, final bare matches without `;` in value blocks are tails,
  nonfinal/semicolon-terminated matches are statements. Parenthesize for arithmetic
  continuation there. Preserve no-newline-heuristic/no-implicit-return behavior.
- Reserve exact `match` and `_` for source/host-language names, retaining `_temp`
  and arbitrary nonempty host module/field names. This breaks prior exact-name
  uses deliberately; it adds no ignored bindings or placeholders. Match remains
  outside the default/contract pure whitelist, including forged backend trees.
- Share pattern checks through `src/match_patterns.rs`; keep checked lowering in
  private `src/codegen/match_.rs`. The walker visits scrutinee then scoped arms;
  retain separate locals/scratch/memory/range/remainder policies and deterministic
  pattern-string discovery. Preserve pattern-only and arm-through-RHS byte spans,
  checked annotations/flow, label identities and scoped initialization restoration.
- `tests/match_frontend.rs` owns syntax/spans/reservation/domain/coverage/scope and
  inference/structural-join tests; `tests/match_lowering.rs` owns forged trees,
  validator and independent decoded cache/result-region/branch-depth/planner checks.
  `tests/match_runtime.rs` owns bounded effects, overlaps, strings, fractional/NaN
  cases and transfers; `tests/match_adapters.rs` owns CLI bytes/diagnostics,
  reservations and rebuilt-browser compatibility. Preserve their default ignored
  runtime/browser policy; release evidence comes from the explicit runs linked
  above, not default Cargo success.

### Lexical trivia and source spans

- `//` comments run to LF, CR, or EOF. `/* ... */` comments may span lines and end
  at the **first** `*/`; nested opening delimiters are ordinary comment text.
- Skip comments in the lexer between tokens wherever whitespace can appear, never
  through source preprocessing. Preserve original UTF-8 byte spans. Delimiters
  inside string literals remain text; comments separate tokens and never join
  identifier fragments or operator bytes.
- An unterminated block comment emits `Invalid` spanning the opening slash to EOF,
  followed by `Eof`. Report it via existing parser errors, without a new error
  stage. Comments change no AST, codegen, or ABI contract.

### Local initializer inference

- Analyze the initializer before introducing the binding. A same-named reference
  resolves only to an existing outer binding; otherwise it is unknown. Allow
  nested-scope shadowing while preserving same-scope duplicate checks and existing
  first-error ordering. After initialization, the binding's type is fixed.
- An inferred void call initializer yields the existing
  `SemanticError::InvalidVoidType` for the local. Explicit annotations retain
  exact type checking; do not add coercions or a new error stage.
- Public untyped `StatementKind::Let.ty` is `Option<Type>`: `Some(type)` for
  annotated declarations, `None` for inferred declarations. Typed
  `TypedStatementKind::Let.ty` remains concrete `Type`. Resolve inference in
  semantic analysis; do not change the walker, backend, or ABI for this feature.

### Expression values and divergence

- Preserve `Block = Vec<Statement>` and `TypedBlock = Vec<TypedStatement>` for
  function and statement bodies. `ValueBlock` / `TypedValueBlock` wrap statements,
  an optional boxed tail, and a span; `Expr` / `TypedExprKind` add `Block` and `If`.
- `TypedExpr.ty` is `ExprResult::{Value(Type), Void, Never}`. Bindings, parameters,
  and function signatures retain concrete `Type`. `Type::Void.into()` is `Void`;
  `Value(Type::Void)` is malformed. Public category equality is identity, not
  Never compatibility. There is no `Type::Never` or source spelling for it.
- A tail without a semicolon supplies a value; a semicolon discards it. Function
  and while bodies remain strict for ordinary expression tails, and a standalone
  block expression statement needs a semicolon. No implicit function return.
- Bare statement-if ends at its final arm, even before `-1;` with no whitespace.
  In value blocks, a final bare if is a tail; a nonfinal bare if is a statement.
  Parenthesize a whole if for arithmetic continuation in these positions. Never
  use newline-sensitive parsing. Retain legacy statement shapes, spans, and bytes.
- `Never` means no normal completion, not a storage/ABI type or a proof that an
  arbitrary call/trapping operation diverges. Join it with the other branch's
  category; otherwise branch results match exactly. Void is discardable, not a
  local value. Inferred Never initialization reports `CannotInferNeverType`.
- Semantic expression analysis propagates transient `(TypedExpr, Flow)`; checked
  lowering reconstructs flow while emitting. Do not add public flow annotations,
  rewalk each subtree, or trust forged result categories. Check all children even
  after a transfer and in skipped logical operands. A divergent logical RHS alone
  does not make the whole logical expression Never because of short-circuiting.
- Use MVP single-result block/if regions: f64 for numbers, i32 for bool/handles,
  empty for Void/Never. No artificial result, store, or drop on Never paths;
  restore stack polymorphism after a region with no normal completion.
- The walker visits value-block statements then tail within block hooks. Locals
  now traverse expressions to find nested declarations; reserve an outer let's
  slot before initializer declarations, preserving legacy declaration preorder.
  Memory, scratch and remainder policies remain separate and exhaustive.

### Statement and value loops

- Raw/typed statement and expression enums add `Loop { body }` (typed nodes also
  carry `id`). Keep bare Break variants; raw `BreakValue(Expr)` and typed
  `BreakValue { target, value }` are separate. Loop bodies remain strict statement
  vectors; only break values supply expression-loop results.
- Prefix bare loops in function/statement bodies, including `loop {};`, are
  statement contexts. A final bare loop without a semicolon in a value block is
  a value tail; nonfinal/semicolon-terminated loops are statements. Parenthesized
  loops are expressions, including discarded values. No newline heuristics.
- Unify only reachable value breaks targeting that loop, with exact concrete
  types. Static/backend checks reject bare breaks in value loops, value breaks
  in statement/while/for loops, and Void RHSs, even when unreachable. Loop-result
  annotations are checked against reconstructed flow, not trusted.
- Lower value loops to MVP typed outer blocks and empty backedge loops. Save no
  artificial result local. The declared annotation chooses the region result;
  an unreachable break with an incompatible raw payload is checked/emitted with
  Unreachable instead of an invalid typed branch. Its constraint still rejects
  compilation if reachable. Preserve live stack prefixes and label identities.
- The walker traverses loop bodies within block hooks and value-break RHSs as
  expressions. Keep memory/local/scratch/remainder policies separate, including
  all unreachable/skipped children. Do not execute unbounded examples in tests;
  subprocess deadlines are not guest fuel or process-tree cancellation.

### Range construction, membership, and internal boundaries

- Public `Type` adds `Range`; raw `Expr::Range` and `TypedExprKind::Range` carry
  boxed `start`/`end`, optional boxed `step`, and `inclusive: bool`. Numeric operands
  use existing Never compatibility; traverse/check all children, including skipped
  and unreachable ones. The walker visits start, end, then explicit step.
- Evaluate all normally completing operands once, left-to-right, before any range
  validation, including invalid, empty, and discarded constructions. Executed
  construction requires finite integral inputs within ±(2^53−1), nonzero step
  (reject ±0), and canonical integer-zero endpoints. Invalid constants trap at
  runtime, not as a new static error.
- Internal range parameters/results lower to `i32`; validate private record bounds
  and fields at parameter entry. Reject range signatures in every host registration
  and forged snapshot; reject only selected user range exports. Keep default
  export-all errors and explicit `Only`/`None` selection, never silent omission.
- Membership requires exact `number`/`range` operands with Never compatibility;
  check all children, including unreachable/skipped code. Evaluate both once,
  left-to-right. Validate the consumed record before the candidate guard, so a
  malformed record traps even for an invalid candidate. Nonfinite, fractional,
  or out-of-safe-domain candidates return false; both signed zeros are accepted.
- Membership uses signed `i64` interval checks and `(candidate - start) % step == 0`
  via `I64RemS`, not floating remainder. Safe-domain differences have magnitude
  at most `2*(2^53−1) < 2^54`; validated nonzero steps keep arithmetic/remainder
  safe. See the memory ABI/spec for the bounds proof.
- Copies share immutable bump-allocated records, not public descriptors. Append
  `Construct`, then `Validate` range helpers after memory and optional remainder
  helpers; append optional `Contains` only when membership occurs anywhere in the
  typed AST. Construction-only bytes and existing helper indices remain unchanged;
  membership alone does not require the floating remainder helper.

### Range and numeric-array statement for loops

- Raw statements add `For { name, iterable, body }`; typed statements add
  `For { id, variable, name, iterable: Box<TypedExpr>, body }`. IDs participate in
  existing per-function loop/binding checks. For is a void statement, never a value
  expression; its body is a strict statement vector. Bare breaks are permitted,
  value breaks are rejected even when unreachable.
- Accept only `range` or `number[]` iterables with Never compatibility. Issue #5
  reuses the existing parser and raw/typed For shapes.
- Evaluate the iterable once in the enclosing context before activating the
  loop or binding. Iterable break/continue targets the outer nearest loop, or is
  invalid without one; preserve return/divergence without inventing fallthrough.
  Check the body even if the iterable cannot complete. A completing iterable can
  describe an empty range or array, so body return alone does not establish
  definite return.
- Iteration bindings are read-only for ordinary and compound assignment. Reconstruct
  checked-lowering write restrictions from For provenance, not new public mutable
  metadata. Distinct inner shadowing lets retain ordinary mutable behavior.
- Array iteration caches its handle and `u32` length once after full `ValidateArray`,
  even for empty arrays. Use unsigned index tests and load each element through
  existing checked `ArrayAddress` then `f64` load on its iteration, never snapshot
  elements or cache a raw payload pointer. Alias mutations remain visible; source
  reassignment does not change the captured handle. Continue exits the body latch
  before index advancement/retest. ABI/helper order is unchanged; array-only for
  requires no range helpers.
- For ranges, validate the consumed record and cache `i64` current/end/step and
  `i32` inclusivity scratch. Test signed endpoints before exact `f64` conversion/yield (zero is +0).
  Continue exits a body latch block, then `I64Add` advances and the loop retests;
  resolve branch depths from label identity, including nested expression regions.
- With `M = 2^53−1`, every yielded current and nonzero step have magnitude ≤ M,
  so `|current + step| <= 2M < 2^54` safely fits i64. The next value may leave the
  safe domain, but the endpoint test rejects it before conversion/yield and exits
  normally; no subsequent addition uses that out-of-domain value. See the memory ABI
  for the full proof. For adds no range helpers or public ABI changes.
- The structural walker visits iterable then body within body block hooks. Keep
  local/memory/temporary/remainder policies separate, with no-reuse scratch slots
  and checks for forged trees and all unreachable/skipped children.
  `tests/for_frontend.rs` covers syntax/types/scopes and transfer rules;
  `tests/for_lowering.rs` covers forged trees and decoded planner/lowering contracts;
  `tests/for_loops.rs` covers deterministic layout, imports/exports, and bounded
  runtime behavior at all optimization levels. `tests/array_for_loops.rs` is the
  array-for suite. Issue #6 adds CLI/browser and cross-feature coverage; adapters
  give actionable private-range host errors while retaining external-signature
  rejection. For cross-feature test coverage, read the
  [WP5 checkpoint](docs/was_language_extensions_plan.md#wp5--safe-ranges-rangearray-for-membership-spec-56).
  All WP5 release gates passed; exact evidence, including the full ignored run,
  is in extension plan §5. Node execution cases retain the ignored policy by default;
  default Cargo success alone does not establish runtime coverage.

### Evaluation, scopes, and module layout

- Evaluate operands, call arguments, and array elements exactly once, left-to-right.
  Indexed assignment validates the target and bounds **before evaluating the RHS**.
  Compound assignment additionally reads the old value before RHS effects and
  writes to the original checked place. Indexed compounds use saved `i32` address
  and `f64` old-value scratch slots; local compounds retain the old value on the
  operand stack. `%=` uses the exact existing remainder helper.
- Logical operators short-circuit at runtime, but both operands, unreachable
  statements, and unselected function bodies still receive static/backend checks.
- Transfers target the nearest active while/for/loop within the function. Bare
  breaks require a statement loop; value breaks require a value loop. Continue
  uses the nearest loop's continuation path. Keywords, including `loop` and `for`,
  are reserved for source and host language names, never imposed on host module/field
  names. No labels.
  Typed while/for/loop nodes carry per-function `LoopId`s; transfers carry resolved
  targets. Sparse IDs are allowed; duplicate, inactive, unknown,
  outer-instead-of-inner, and cross-function targets are rejected.
- `Flow` separates fallthrough, return, break, continue, and explicit divergence.
  Reachable value-break type constraints travel in private flow summaries and are
  consumed at the nearest loop boundary; never collect constraints in mutable
  context without sequence reachability or rewalk subtrees. Unreachable breaks
  must be contextually/type valid but do not constrain the loop result. Void
  break values are invalid; a Never RHS produces no value or actual break.
- Sequence only fallthrough paths in summaries but check/emit every child. Nonvoid
  functions allow return or explicit divergence, never normal fallthrough.
  Ordinary while remains conservative even with literal true; a syntactic loop
  without reachable exit yields Never. Activate while context before its condition;
  condition blocks can transfer to that loop. A noncompleting condition consumes
  its own break/continue paths without inventing completion. Summaries include
  expression effects without pruning checks.
- `BodyEmitter` tracks unique labels for every structured `if`, `block`, and `loop`,
  including short-circuit expressions. Resolve `LabelDepth` from active target
  identity and nesting, never fixed loop depths. Restore labels/loop contexts and
  initialized bindings on errors without rewinding scratch slots or identity counters.
- Variable IDs are per-function and may be sparse. Local slots place parameters
  first, then declarations/iteration bindings, then typed temporaries (array handles,
  indexed compound addresses/old values, and for induction/bounds scratch). Preserve
  shadowing, scoped initialization restoration, and the current no-reuse temporary policy.
- Internal signature mapping does not establish host/export eligibility. Share the
  private external-type policy with host registration; layout checks every host
  snapshot and only selected user signatures for external eligibility. Map every
  internal user signature and check every body regardless of export selection.
- Emit imports in registration order and user functions in source order. Deduplicate
  signatures without confusing type indices with function indices. Use the
  `FunctionIndex`, `TypeIndex`, `LocalIndex`, and `LabelDepth` newtypes internally.
- Keep output deterministic. Hash maps/sets are lookup tools, not emitted-order
  authorities. Preserve static-string first-discovery order and helper ordering.
- Export selection changes only the public surface: no body/import pruning or
  index shifts. Required memory exports remain even with no user-function exports.
- All registered imports must be supplied by the embedding, including unused ones.
  There is no implicit `print`, WASI, or automatically invoked guest start function.

### Numbers and equality

- `number` and array elements are IEEE-754 `f64`, even for integer-looking literals.
  Overflowing literals are errors; underflow is allowed. Arithmetic can produce
  NaN, infinities, and signed zero. Do not assume finite results or exact integers.
- `%` is truncating floating remainder, matching JavaScript `%`/`fmod`, not
  mathematical modulo or nearest-quotient IEEE remainder. Wasm has no `f64.rem`.
  The optional guest helper uses exact scaled subtraction and needs no implicit
  host import or memory. Never replace it with `a - trunc(a / b) * b`.
- Remainder preserves the dividend's signed zero; NaN operands, an infinite
  dividend, or a zero divisor produce NaN. Finite `%` infinite returns the dividend.
- Array equality compares lengths and exact elementwise `f64` values: `+0 == -0`,
  but NaN is unequal even when comparing an array handle to itself. No pointer or
  byte-equality shortcut. `!=` is the complement of `==`.
- Evaluate both array operands before reading compared elements; RHS mutations
  must be visible. Validate **both complete descriptors before early exits**.
  String equality compares UTF-8 bytes, without normalization or case folding.

### Memory and host ABI

- Centralize production constants/signatures in `src/codegen/abi.rs` and helper
  ordering in `Helper::ALL`; do not duplicate magic offsets in emitters.
- Raw types: `number = f64`; `bool = i32`; strings/arrays = `i32` handles despite
  being distinct language types. Normalize host boolean boundaries to `0`/`1`.
- A descriptor is eight bytes, eight-byte aligned, with little-endian `u32` data
  pointer at offset `0` and length at offset `4`. String length counts bytes;
  array length counts eight-byte `f64` elements with eight-byte-aligned payloads.
  Addresses below `8` are invalid. Valid host payloads need not be contiguous with
  their descriptors. Handles are instance-local and assignments can alias arrays.
- The bump allocator rounds to eight-byte alignment and reserves at least eight
  bytes. There is no freeing. Memory32 grows in 65,536-byte pages with a maximum
  of 65,535 pages; use widened unsigned range checks and explicit overflow checks.
- Preserve memory helper order: `Alloc`, `ValidateString`, `ValidateArray`,
  `ArrayAddress`, `ArrayLen`, `StringEqual`, `ArrayEqual`. The optional remainder
  helper follows user functions and any memory helpers. Scalar-only programs must
  not gain memory/global/data sections just to support `%`.
- Indices must be finite, integral, nonnegative, and in bounds; negative zero
  addresses element zero. Invalid indices/descriptors and allocation failure trap.
- Hosts must preserve descriptor metadata, valid UTF-8, and string immutability.
  Trusted hosts can mutate array elements. Recreate JavaScript views after possible
  memory growth. Guest bounds checks do not isolate malicious hosts or validate UTF-8.
- The compiler emits raw Wasm with no automatic JSON interface metadata. Optional
  runtime conversions are explicit copies, not part of the compiler ABI.
  There is no compiler-provided execution fuel, cancellation, or transactional rollback.

### Diagnostics and CLI

- Preserve error variant, message, span, and first-error ordering for unchanged
  behavior. Keep exhaustive `span() -> Option<Span>` extraction beside stage-error
  definitions and delegate through `CompileError`; do not silently discard locations.
- Spans are UTF-8 byte offsets. Expressions have no independent spans; nested
  expression errors generally use the enclosing statement. Return raw offsets
  from accessors; only the renderer clamps invalid ranges for presentation.
- Preserve character-based columns, four-space excerpt tabs, CRLF/EOF handling,
  and consistent native/browser diagnostics. Precise expression spans and recovery
  are separate features, not formatting refactors.
- The CLI requires a file, never compiles a fallback example, and does not read
  stdin. Source `-` is a literal filename; exact output `-` means text stdout.
  Default output is plain WAT; only exact lowercase `.wasm` infers binary.
  `--emit` overrides inference, and binary requires a filesystem output path.
- Help exits `0`, usage errors `2`, and compilation/registration/I/O failures `1`.
  Keep CLI binary output identical to library bytes for matching inputs/options.
- Preserve canonical same-input refusal, no automatic parent-directory creation,
  sibling `create_new`/write/sync/rename output, temporary cleanup, and unchanged
  destinations after compilation failure. Do not overstate race or durability guarantees.
- Rust host registration accepts arbitrary nonempty module/field names. CLI host
  syntax is delimiter-limited; do not impose its restrictions on the library API.

## Implementing a feature

1. Establish syntax, typing, evaluation order, failure behavior, and ABI impact.
   Read neighboring tests and the limitations/roadmap before choosing a design.
2. Update tokens/lexer, parser, `grammar.bnf`, and untyped AST where applicable.
   Keep precedence, associativity, assignment targets, and malformed-input handling explicit.
3. Extend semantic analysis and typed AST with exact type/scope/return rules and
   stable IDs. Update exhaustive matches; do not bypass the type system in codegen.
4. Extend the shared walker for new structural children, then adjust relevant
   memory, local, temporary, and numeric planning policies separately.
5. Update ABI/layout/helper signatures and typed indices if needed. Lower through
   the existing emitter modules, retaining checks for forged typed input and stack balance.
6. Add stage errors and span accessors where needed. Propagate configuration through
   the library and relevant CLI/browser adapters without duplicating compiler logic.
7. Add tests at every affected boundary, including rejection and observable effects.
   Update README and relevant docs to distinguish implemented behavior from proposals.
8. Run focused checks first, then broader validation appropriate to the change.
   Report actual commands/results and any unavailable validation. Update this guide
   when the architecture, features, or workflows change.

## Rust coding guide

- Use Rust 2024 and existing project idioms. Prefer small cohesive functions,
  descriptive names, early returns, exhaustive enum matches, and explicit ownership.
  Add blank lines between declarations and logical blocks; avoid dense method chains.
- Keep visibility minimal (`pub(super)`/private for backend internals). Prefer enums
  and index/ID newtypes over boolean mode flags, strings, or interchangeable integers.
  Unwrap typed indices with `.raw()` at encoder/comparison boundaries, not throughout planning.
- Borrow `&str`, slices, and references when ownership is unnecessary. Own values
  when they must outlive source/registry input. Clone deliberately, never just to
  evade borrowing; do not clone whole ASTs or large collections in hot paths.
  Use `Cow` only when borrowed-versus-owned behavior is meaningful.
- Use iterators when they clarify a transformation and loops when they clarify
  state, ordering, or errors. Reserve collection capacity when size is cheaply
  known; avoid repeated allocation, string formatting, and lookup scans in hot paths.
  Do not merge distinct planning policies merely to eliminate a traversal.
- Propagate recoverable errors with `Result`, typed errors, and `?`. Production
  `expect`/`assert` must express a justified internal invariant, not malformed source
  or routine I/O. Do not add broad panic catching or blanket lint suppressions.
- Use checked arithmetic/conversions for sizes, offsets, indices, and memory ranges.
  Distinguish byte alignment from Wasm's log2 alignment immediate. Explicit bit
  reinterpretation is valid when intentional; lossy `as` casts are not validation.
- Prefer safe Rust and standard-library/existing dependencies. New unsafe code,
  concurrency, global caches, or packages require a concrete need and documented
  tradeoffs. Keep planning sequential unless parallelism is explicitly requested.
- Optimize from measurements, not assumptions. Preserve IEEE-754 behavior,
  deterministic order, short-circuiting, side effects, and trap order. No algebraic
  rewrites, constant folding, or dead-code removal inside a nominal refactor.
- Comments should explain non-obvious intent, invariants, or tradeoffs, not narrate
  code. Document public contracts where useful. Do not reformat unrelated files.

## Mandatory testing

**Every feature and regression fix must include tests.** Refactors must preserve
existing tests and add focused coverage for newly introduced boundaries when needed.

- Test frontend acceptance/rejection, types/scopes, planner behavior, and lowering
  for every affected stage. Include malformed source and forged typed-AST cases.
- For emitted modules, use `wasmparser::Validator` plus independent decoded
  instruction/layout assertions. Valid Wasm alone does not prove correct semantics.
- Execute generated Wasm for observable behavior where appropriate: host effects,
  short-circuiting, evaluation/trap order, mutation/aliasing, loops, and returns.
  Check unexported, unreachable, and runtime-skipped code for static validation.
- Numeric tests cover NaN, infinities, signed zero, subnormals, and extremes as
  applicable. In JavaScript use `Number.isNaN` and `Object.is` for special values;
  use independent expected results/reference behavior, not the production algorithm.
- Preserve import snapshots, deterministic ordering, export-selection behavior,
  and optimization-level parity. Adapter changes need CLI/browser contract tests.
- Use `tests/support/process.rs` for bounded subprocess tests: it drains pipes
  concurrently, applies deadlines, and kills/reaps the direct child. It is not a
  process-tree controller or guest fuel mechanism. Keep specialized decoders local;
  avoid building a generic testing framework.
- Preserve the existing ignored-test policy for Node/browser dependencies. A passing
  default Cargo suite is not evidence that ignored runtime cases passed.
- Golden fixtures under `tests/fixtures/abi`, `tests/fixtures/emitter`, and
  `tests/fixtures/diagnostics` encode compatibility expectations. Do not regenerate
  them just to make tests pass. Intentional changes require an explicit contract
  decision; retain independent expected bytes/offsets instead of deriving all
  expectations from production constants. Preserve diagnostic fixture newline bytes.

### Validation commands

Run from the repository root. Start with relevant test filters; use these for
broader checks. Bound long-running commands in agent tools.

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

For core/browser changes, rebuild `pkg/` before browser tests or the full ignored
suite; stale generated bindings do not validate current source. Node.js 24+,
`wasm32-unknown-unknown`, wasm-pack, compatible wasm-bindgen tooling, and cached
Rust dependencies are prerequisites. `--mode no-install` avoids tooling installation.
There is no root `browser` feature; the dedicated workspace crate is the adapter.

```sh
wasm-pack build browser --target web --out-dir ../pkg --out-name was_compiler --mode no-install --no-opt --offline
node tests/browser.cjs pkg
cargo test --offline -- --include-ignored
cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings
```

For CLI/runtime changes, a useful smoke test returns `65`:

```sh
cargo run --offline -- examples/array-sum.was -o target/array-sum.wasm --export main
node runtime/run.cjs target/array-sum.wasm
```

Format only edited Rust files, supplying their actual paths:

```text
rustfmt --edition 2024 --config skip_children=true <edited Rust files>
```

Do not run workspace-wide formatting that changes unrelated files. Finish with
`git --no-pager diff --check` and review the diff/status. For documentation-only
changes, check links/paths and whitespace; a code test run is normally unnecessary.
Never claim a test passed unless it ran successfully; report missing tooling,
ignored coverage, and unrelated failures without modifying unrelated code.
