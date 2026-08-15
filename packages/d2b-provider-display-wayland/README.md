# `d2b-provider-display-wayland`

This crate owns the authenticated Wayland projection foundation consumed by
clipboard-wayland. It keeps compositor and GPU attachment grants opaque,
supervises no host-singleton service, and publishes only bounded status,
audit, and telemetry observations.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[display-wayland dossier](../../docs/specs/providers/ADR-046-provider-display-wayland.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `display-wayland` |
| Provider reference | `Provider/display-wayland` |
| Package | `packages/d2b-provider-display-wayland/` |

## Config schema

The Provider config is the signed `display-wayland` artifact with a bounded
principal pool (`1..=32`). `WaylandSession` requires Guest, Host, User, and
qualified WaylandPolicy references plus `crossDomainTrusted = true`.

## Exported resource types

The Provider projects `WaylandSession` and `WaylandPolicy`. Policy compilation
rejects unknown interfaces and virtualizes clipboard-manager globals.

## Controllers / services / workers / binaries

The crate exposes the Zone display controller, same-user user portal, opaque
LaunchTicket, path-free readiness event, and Host proxy / Guest frontend
templates. The proxy and frontend have no d2b-bus authority after launch.

Runtime admission and process supervision are daemon-owned. This crate does
not install standalone Provider binaries; `d2bd` launches signed workers
through authenticated ComponentSession and ProviderSupervisor effect ports.

## Placement and dependencies

The controller is a Zone system component. The user portal is one per active
same-UID compositor session. GPU and compositor connections arrive as
ProviderSupervisor attachment grants; no socket path or `WAYLAND_DISPLAY` is
accepted.

## RBAC requirements

Resource and effect admission is delegated to Core, ComponentSession, and the
typed ProviderSupervisor launch boundary. The controller never receives an FD.

## Security posture

Proxy principals are hash-derived or drawn from a pre-provisioned bounded pool.
Finalization retains its finalizer when Process termination is ambiguous.

## State and telemetry

No Provider state Volume is declared. Audit records hash identity fields and
telemetry labels are closed; display content, paths, titles, and app IDs are
not observable.

## Build and test

```bash
cargo check -p d2b-provider-display-wayland
cargo test -p d2b-provider-display-wayland
```

The tests are hermetic and cover policy layering, readiness, principal-pool
exhaustion, redacted status, and lifecycle transitions. Cross-process
integration fixtures use fake display and GPU services; they do not require a
live compositor.
