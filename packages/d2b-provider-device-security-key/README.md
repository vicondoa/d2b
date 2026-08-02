# `d2b-provider-device-security-key`

This crate implements the unprivileged contracts for
`Provider/device-security-key`. It owns the Device lease, bounded session
observations, CID translation, and relay/frontend Process declarations. Core
resolves physical authority and supplies opaque effect tickets.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `device-security-key` |
| Provider reference | `Provider/device-security-key` |
| ResourceType | `Device` |
| Package | `packages/d2b-provider-device-security-key/` |

The Provider is bound to the Device's Zone through Core's same-Zone resource
references. Semantic Service/Binding projection and backing-set selection are
not implemented by this crate.

## Config schema

The Device extension is
`device-security-key.d2bus.org/Device/spec`, version `1.0`. The bounded
settings used by this crate are:

| Setting | Bounds or rule |
| --- | --- |
| `vsockPort` | `1-65535`, default `14320` |
| `sessionRingSize` | `8-256`, default `32` |
| `leaseTimeoutSecs` | `30-3600`, default `300` |

Nix and Core author the physical Device selector. The Provider accepts no
hidraw path, sysfs path, bus identifier, credential, or host selector string.
Core supplies the opaque physical USB backing claim after Zone and authority
checks.

## Exported resource types

The crate exports the standard `Device` implementation contracts and the
Provider-owned `HostRelay` and `GuestFrontend` Process declaration helpers.
`FrontendProcessDeclaration` accepts only a same-Zone `Guest` ResourceRef and
derives the deterministic `device-<uid-short>-sk-frontend` name. The relay
uses `device-<uid-short>-sk-relay`.

Lease IDs, session records, CIDs, relay tickets, and frontend attachment
handles remain opaque bounded protocol values. They are not ResourceTypes,
status history, or semantic projection objects.

## Controllers / services / workers / binaries

`SecurityKeyController` sequences physical authority claim, hidraw open,
single-session lease state, terminal recording, and authority release.
`SecurityKeyEffectPort` is the only effect boundary. `SecurityKeyLease` keeps
the Core-issued authority lease and relay LaunchTicket private.

The Host relay is unprivileged and receives only the opaque LaunchTicket. The
Guest frontend is user-domain and receives only its Core-issued attachment.
This crate declares those workers; Core and the existing process system
provide their actual placement and supervision.

## Placement and dependencies

The relay is Host placed. The frontend is Guest placed in the `user` domain
and is constructed only from a `Guest` execution reference. ResourceRefs are
same-Zone by contract, so the Provider never resolves a cross-Zone Guest or
accepts a caller-supplied transport address.

The crate depends on `d2b-contracts` for neutral resource identity and on
`serde` for test-side bounded settings. It has no daemon, broker, host
lifecycle, semantic projection factory, or sibling Provider dependency.

## RBAC requirements

The controller needs bounded read/watch access to its Device, Guest, and
owned Process dependencies, plus status and finalizer authority for its own
Device. Core admits each physical backing claim and launch ticket. The
Provider has no direct broker, filesystem, hidraw, UHID, credential, or host
mutation permission.

## Security posture

Core must admit the Host physical USB backing tuple before the Provider opens
hidraw. A conflict returns `physical-usb-backing-conflict` before any open
effect. The broker-returned fd is carried only inside the opaque relay
LaunchTicket. No path, bus ID, selector string, session bytes, raw fd, or
host credential enters this crate.

Only one session can be active for a lease. Session IDs and effect tickets
redact their values in `Debug`; CTAP payloads and credential material are not
stored or logged. Relay and frontend boundaries remain separate and
unprivileged.

## State and telemetry

The Provider retains only the bounded recent-session ring and the current
lease phase. The ring evicts its oldest record at capacity and stores an
opaque session ID plus a closed result, never CTAP bytes or credential data.
Physical identity and host effect audit records remain with Core.

Metrics and diagnostics use fixed Provider, component, operation, outcome, and
error values. Zone names, resource names, device identity, paths, PIDs, CIDs,
and session material are excluded from labels and messages.

## Build and test

```bash
cd packages
cargo check -p d2b-provider-device-security-key
cargo test -p d2b-provider-device-security-key
cargo nextest run -p d2b-provider-device-security-key
cargo clippy -p d2b-provider-device-security-key --all-targets -- -D warnings
cargo fmt --package d2b-provider-device-security-key -- --check
cargo run -p xtask -- check-provider-layout
```

The container-targeted integration scenario exercises the public Provider
boundary with a fake Core effect port. It never opens a host device or owns a
credential. Host/Guest and hardware coverage belong to their repository lanes.

## Future standalone use

An extracted Provider repository would retain the neutral contracts, signed
component descriptor, opaque effect boundary, lease/session policy, and
Process declarations while replacing workspace packaging and release
metadata.
