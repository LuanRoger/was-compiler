# WAS language

Terms for the language's collections, defaults, contracts, numeric patterns, pipelines,
and local closures. Proposed extensions remain distinct from implemented language support.

## Language

**Number**:
The language's single numeric type, with IEEE-754 binary64 semantics, including
fractional values, signed zero, infinities, and NaN. Integer-valued numbers do not
form a separate language type.
_Avoid_: Inferred integer type, integer-looking type

**Array**:
A fixed-length, mutable collection whose elements share one type. Assignment and
parameter passing share the collection rather than copying its elements.
_Avoid_: List, growable list

**Growable list**:
A proposed collection whose length can change after creation, distinct from an array.
Growable lists are not currently supported.

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

**Local callable binding**:
A lexical name for a closure that can be invoked directly but cannot be aliased,
reassigned, passed, returned, or otherwise used as a value.
_Avoid_: Function-valued local, first-class function

**Shared-binding capture**:
A closure's reference to an original lexical binding, preserving its type and
mutability and observing reassignment from either side.
_Avoid_: Snapshot capture, capture by value

**Lexical callable dependency**:
A closure's dependence on an earlier visible local callable for direct invocation,
including its live context when required. It is not a callable alias or value capture.
