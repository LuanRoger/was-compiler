# WAS language

Terms for the language's defaults, contracts, and numeric patterns. Proposed extensions
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
