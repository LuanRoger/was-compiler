# Represent normalized calls explicitly in the typed tree

WP6 uses a dedicated typed normalized-call sequence rather than representing
named/default binding as an indistinguishable ordinary value block. Existing blocks,
lets and calls can express evaluation order, but cannot identify the formal-to-binding
mapping and generated-binding ownership that checked lowering must verify in public,
mutable typed input.

The sequence owns ordered invocation-local bindings, formal mapping and origin spans,
ending in a canonical full-arity direct call. This adds a focused typed construct and
consumer migration rather than another IR or a second validator. Default references
are remapped by resolved formal identity; ordinary full-positional calls retain their
existing path where normalization is unnecessary, preserving legacy bytes.

Issues #10/#12 implement this accepted decision incrementally for named calls and
trailing pure defaults; WP6's separate release gate #13 remains open. See the
[WP6/WP7 interface decisions](../was_language_extensions_plan.md#wp6wp7-shared-interface-decisions)
and [specification §14.3](../was_language_extensions_spec.md#143-abi-and-local-callable-restrictions).
