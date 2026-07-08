# Explanation: realm tree routing boundaries

> Diataxis: explanation. Conceptual model for realm discovery and strict tree
> routing. For field-level schema validation, read
> [`docs/reference/realm-routing.md`](../reference/realm-routing.md).

The realm routing contract models how future realm controllers may discover each
other and authorize operations across a parent/child tree. It deliberately stops
at metadata today: the repository contains data models, validators, generated schemas,
and tests, but not a live router that moves current VM lifecycle traffic onto
relays or per-realm daemons.

## Why routing is a tree

A tree gives every cross-realm decision a single accountability path. A child can
advertise only itself and descendants below itself to its direct parent. A parent
can route to another branch only by walking up to the nearest common ancestor and
back down through validated child edges. That keeps policy review, revocation,
and audit attached to explicit parent/child relationships instead of ad-hoc mesh
links.

The model rejects sibling advertisements, parent advertisements from a child,
loops, and multi-parent shapes because those make it unclear which controller is
authorized to speak for a realm namespace.

## Discovery is not trust

Discovery input arrives before a peer is authenticated, so the contract treats it
as hostile by default. Queue depth, unverified peer count, pre-auth rate limits,
and replay windows are explicit bounds, and queue overflow drops new input rather
than growing unbounded state.

The discovery metadata intentionally uses coarse ingress classes and redacted
peer refs. It should help operators understand whether input came from a parent,
child, provider, local-root, static config, or unknown source without recording
raw relay endpoints or identity claims as truth.

## Advertisements prove namespace, not reachability

A route advertisement says, "this direct child is allowed to advertise these
descendant prefixes and capabilities until this expiry, signed by this controller
generation." It does not say that an underlay transport is reachable, that a
provider credential exists, or that current `d2b` commands can use the route.

Namespace allocation is the parent-side guardrail. If a child is allocated
`dev.work`, it cannot advertise `work`, `personal`, or a sibling subtree. The
capability ceiling prevents a child from exporting capabilities the parent did
not delegate, even if a downstream provider claims them.

## Shortcuts are accountable exceptions

A direct shortcut can later optimize transport, but only after the normal tree
route is authorized. Its metadata keeps the authorized tree path, source, target,
nearest common ancestor, operation kind, policy rule, and expiry. That means audit
and revocation still point back to the tree decision even if the data path becomes
shorter.

Shortcuts are not generic tunnels. They must not become VPNs, overlays, implicit
SSH fallbacks, raw relay pipes, TCP proxies, file-descriptor tunnels, or provider
opaque channels that bypass realm policy.

## Correlation without payload leakage

Every discovery, admission, advertisement, decision, shortcut, and teardown shape
has a correlation id and may carry a bounded trace context. Those ids join audit
records across realms and transports. They do not justify logging payload bytes,
argv, stdio, provider headers, host paths, endpoint strings, credentials, or
unbounded peer identity strings.

Route telemetry follows the same rule: counters use stable event/reason/realm
class/placement labels, not raw target names or transport addresses.

## Relationship to access and identity

The [realm access resolver](../reference/realm-access-resolver.md) answers,
"which binding should this client use for this target?" The
[identity lifecycle contract](../reference/realm-identity-lifecycle.md) records
controller generations, parent trust anchors, child pins, revocation, teardown,
and recovery metadata. Tree routing composes with both: a future router needs a
resolved realm, valid controller generation, accepted parent/child identity
relationship, route namespace allocation, replay check, capability check, policy
check, and bounded audit record before admitting cross-realm work.

Today those are contracts for future runtime work, not a claim that live relay
sessions, runtime route enforcement, or per-realm `d2b` lifecycle routing are
implemented.
