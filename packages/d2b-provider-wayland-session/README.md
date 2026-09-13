# `d2b-provider-wayland-session`

This is the crate root for the `display-wayland.d2bus.org.WaylandSession` ResourceType, one of the six
resource types of the interaction family. It owns the session's driver, its spec decoder, its child-intent port, and the driver declaration the v3 resource plane registers the type by; the driver verbs come from the interaction family's shared engine in `d2b-provider-wayland-policy`.

## Provider identity

| Field | Value |
| --- | --- |
| Provider identity | `wayland-session` (resource-type owner) |
| ResourceType | `display-wayland.d2bus.org.WaylandSession` |
| Package | `packages/d2b-provider-wayland-session/` |
| Driver declaration | `wayland_session_descriptor` -> `DriverDescriptor` |
| Registration | `packages/d2b-provider-wayland-session/tests/registration.rs` |

## Config schema

The session's compiled spec: the Guest, Host, User, and policy references, the display identity, and the cross-zone allowance. Validate decodes it typed and refuses anything else.

## Exported resource types

`WaylandSession` is not exportable (`exportable: false`).

## Controllers / services / workers / binaries

One driver, `WaylandSessionDriver`, built by `WaylandSessionFactory`. The session owns two worker Process children and their private Endpoint children, authored by the daemon's display supervisor behind `DisplayChildSource` and materialized here into manager child rows.

## Placement and dependencies

The crate depends on the resource contracts, the runtime contracts, the declaration vocabulary, the display Provider's spec vocabulary, the core-controller child vocabulary, and the family engine crate. The display supervisor's launch material stays in the daemon behind the child-intent port.

## RBAC requirements

Reads follow the zone RBAC rows for the type; the driver performs no privileged operation, mints no ticket, and assembles no argument vector.

## Security posture

The driver never fabricates readiness: a missing child or dependency keeps the row Pending, and refusals are closed codes. Child bodies are the display Provider's signed intents; the driver adds no identity and no address.

## State and telemetry

The restart-generation annotation the display status reads travels with the child rows; the driver itself persists nothing and emits no telemetry.

## Build and test

```bash
cargo test -p d2b-provider-wayland-session
bazel test //packages/d2b-provider-wayland-session:all-tests
```

`tests/registration.rs` proves the declaration the plane registers, the four row reads, and the manager child rows the display supervisor's intents materialize into.
