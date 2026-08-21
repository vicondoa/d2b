# `d2b-provider-notification-desktop`

This crate owns authenticated, transient desktop notification streams. It
keeps delivery state and action capabilities in host-sink process memory only.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[notification-desktop dossier](../../docs/specs/providers/ADR-046-provider-notification-desktop.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `notification-desktop` |
| Provider reference | `Provider/notification-desktop` |
| Package | `packages/d2b-provider-notification-desktop/` |

## Config schema

Configuration is the signed `notification-desktop` artifact: bounded pending
notifications, action nonce TTL/capacity, display-wayland dependency, and a
closed `guestSources` category allowlist.

## Exported resource types

Notification delivery is stream-only and exports no semantic ResourceType.

## Controllers / services / workers / binaries

The crate exposes a placement controller, source/sink stream DTOs, admission
checks, a host-sink effect port, an observer projection, and single-use action
nonces. D-Bus integration is supplied through the pre-opened effect port.

Runtime process admission and supervision are daemon-owned. The package also
owns the `d2b-sk-waybar-helper` compatibility binary; `d2bd` owns the
authenticated ComponentSession service loops and launches signed workers
through its ProviderSupervisor effect ports.

## Placement and dependencies

Guest sources use enrolled Noise KK; local desktop observers use authenticated
Unix seqpacket sessions. The host sink is user-domain and depends on the
display-wayland Provider.

## RBAC requirements

Only exact stream purposes are admitted. Notification content never selects a
resource operation, and action invocation requires the same authenticated
observer session plus a live nonce.

## Security posture

The sink accepts sanitized DTOs through a typed effect port. No D-Bus address,
socket path, credential, or host singleton is created by this crate.

## State and telemetry

No Provider state Volume is declared. Audit and telemetry retain only digests
and closed semantic labels; summary, body, icon, and action text are excluded.

## Build and test

```bash
bazel test //packages/d2b-provider-notification-desktop:d2b_provider_notification_desktop_doc_test
```

The tests cover bounded DTOs, sanitization, session admission, action replay,
dependency placement, and in-memory lifecycle. Integration fixtures use fake
ComponentSession and D-Bus effect ports.
