# `d2b-provider-clipboard-wayland`

This crate owns Zone-scoped clipboard mediation behind
`Provider/display-wayland`. Clipboard content remains in bounded clipd-host
memory or one validated attachment FD and never enters status, audit, or
telemetry.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[clipboard-wayland dossier](../../docs/specs/providers/ADR-046-provider-clipboard-wayland.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `clipboard-wayland` |
| Provider reference | `Provider/clipboard-wayland` |
| Package | `packages/d2b-provider-clipboard-wayland/` |

## Config schema

The signed Provider config contains Host/User placement, an optional typed
display dependency, closed MIME policy, bounded history/FD/rate caps, and
picker timing. Cross-Zone transfer is disabled by default.

## Exported resource types

The Provider is service-only and exports no semantic ResourceType. Core owns
the two component Process resources; the controller creates only operation
scoped picker EphemeralProcesses.

## Controllers / services / workers

The crate exposes clipd-host policy/history/FD safety, clipboard-controller
placement, metadata-only picker records, and fail-closed audit queue types.
The display bridge is a typed effect port, not a filesystem or compositor
socket.

Runtime process admission and supervision are daemon-owned. This crate does
not install standalone Provider binaries; `d2bd` launches the signed
controller, host service, and picker worker through authenticated
ComponentSession and ProviderSupervisor effect ports.

## Placement and dependencies

clipd-host is a user-domain service on the configured Host/User. The
clipboard-controller is a system-domain controller. The display dependency is
optional for host-only mode and required for Guest bridge operations.

## RBAC requirements

ComponentSession service names and attachment classes are closed. FDs are
accepted only after object/filesystem classification and `MSG_CTRUNC`
fail-closed checks.

## Security posture

There is no shared filesystem bridge, per-Guest socket group, direct
`WAYLAND_DISPLAY`/`NIRI_SOCKET` access, DND path, or primary-selection path.

## State and telemetry

Clipboard history is bounded process memory. Guest stop/lock/destroy operations
suspend or purge it. Audit size is bucketed and telemetry labels are closed.

## Build and test

```bash
cargo check -p d2b-provider-clipboard-wayland
cargo test -p d2b-provider-clipboard-wayland
```

The tests cover MIME and secret-hint policy, FD safety, bounded history,
guest lifecycle, picker metadata, dependency placement, and fail-closed audit.
Integration fixtures use fake display and bridge services without a live
compositor.
