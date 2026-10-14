# WAS language

Terms for the language's defaults, contracts, numeric patterns, and pipelines. Proposed extensions
remain distinct from implemented language support.

## Language

**Pure expression**:
An expression admitted by the WAS default/contract whitelist that performs no writes
or runtime allocation; it may read shared array contents or trap.
_Avoid_: Nontrapping expression, arbitrary side-effect-free expression

**Default expression**:
An expression supplying an omitted argument for an invocation, resolved in the callee's
declaration scope with access only to earlier formal parameters.

**Contract expression**:
A boolean precondition over a named function's formal parameters, evaluated in declaration
order before its body; a false condition causes a trap.

**Interval pattern**:
A numeric match pattern selecting values between constant bounds, including fractional
values. It denotes an interval, not the stepped progression of a range value.
_Avoid_: Range membership pattern, progression pattern

**Named-only pipeline**:
An ordered chain that supplies its input to the first parameter of each named
user-function stage; it does not make functions into values.

**Pipeline stage**:
A function invocation within a pipeline, receiving the preceding input as its first
argument before any written arguments; omitted remaining arguments use defaults.

**Final Void stage**:
The last invocation in a named-only pipeline whose declared result is void,
permitted only where the pipeline's result is discarded.
