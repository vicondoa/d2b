# `d2b-provider-device-security-key`

This crate implements `Provider/device-security-key`, the physical HID
security-key Device Provider.

## Config and Nix authoring

The Device extension is
`device-security-key.d2bus.org/Device/spec` version `1.0`. Its bounded
settings are `vsockPort` (default `14320`), `sessionRingSize` (8-256, default
32), and `leaseTimeoutSecs` (30-3600, default `300`). Nix authors declare a
physical `Device` with a `hidraw` inventory selector under
`d2b.zones.<zone>.resources.<name>`.

## Controllers, Processes, and placement

The controller owns one Host relay Process named
`device-<uid-short>-sk-relay` and one Guest frontend Process named
`device-<uid-short>-sk-frontend`. The frontend is Guest/user placed; the
relay is Host placed and unprivileged. The controller owns one lease and a
bounded recent-session ring.

## Dependencies and RBAC

The Provider reads its Device, Guest, and Process dependencies, writes status
and finalizers for its Device, and creates only its owned relay/frontend
children. Core supplies the `SecurityKeyEffectPort`; no Provider-level broker
permission is granted.

## Security and state ownership

Core derives the Host physical USB backing tuple and must admit it before the
Provider asks to open hidraw. A conflict returns
`physical-usb-backing-conflict` before any open effect. The broker-returned fd
is placed only in the relay LaunchTicket. No path, bus ID, selector string,
session bytes, or raw fd appears in this crate. The lease permits one session
per Device.

## Telemetry and audit

Metrics use fixed operation/outcome/error labels only. Zone/resource names,
device identity, CTAP bytes, paths, and PIDs are not metric labels or
diagnostics. Core owns path-free broker audit records.

## Build and test

```bash
cargo test -p d2b-provider-device-security-key
cargo xtask check-provider-layout
```

The `integration/` fixtures run through the existing container or Host/Guest
lane, not the hermetic Cargo test suite.

## Future standalone use

The Provider boundary remains portable: retain `d2b-contracts`, the signed
component descriptor, opaque effect port, and Process declarations when
packaging this crate in a standalone Provider repository.
