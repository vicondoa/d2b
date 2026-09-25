# Dependency injection

The depth for the getting-a-dependency-in section of `SKILL.md`: the escalation
ladder, one rung per section, each with what it costs and what makes you climb,
and the two rules at the bottom.

## A concrete type

Pass the thing. It costs nothing — the signature names the real type, the
dispatch is as fast as there is, and the caller learns one name. Most
"injection" exists because someone anticipated a second implementation that
never arrived; stay on this rung until one does.

## A wrapper struct

One field, one type, behaviour attached — enough to swap the inside without
touching a signature, and it costs no generic parameter. Climb here when the
inside must change — a backend, a configuration, a policy — while callers keep
naming one thing.

## A generic parameter

Static dispatch, monomorphized, no runtime cost — and it spreads: `Client<S>`
means every type holding a `Client` is generic too, and that infects the public
API of everyone downstream. Climb here only when the inside is a different kind
of thing, not just a different instance, and pay the infection on every type
that holds it.

## `dyn Trait`

One vtable indirection, one type regardless of how many implementations, and the
generic infection stops. The price is object safety and the loss of generic
methods. Climb here when a caller must hold heterogeneous implementations — a
registry, a plugin list — and the generic parameter has spread too far to
unscale.

## Climb only when forced

Climb only when the current rung cannot express the requirement, and never climb
for a test-only need — that case is ADR 0006 and belongs to `/rust-testing`.
Every rung above the concrete type is a permanent part of the public API, so pay
it for a requirement the surface really has, not for one only the test harness
has.

## No preludes in libraries

Two glob-imported preludes collide, and a prelude that would make a crate easier
to understand is a signal the module structure needs work rather than a shortcut
worth shipping.
