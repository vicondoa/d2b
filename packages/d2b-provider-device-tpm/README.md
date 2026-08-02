# `d2b-provider-device-tpm`

This crate implements `Provider/device-tpm`, which manages the emulated TPM
`Device` ResourceType and its state-preserving controller.

## Config and Nix authoring

The Device extension is
`device-tpm.d2bus.org/Device/spec` version `1.0`. Its bounded settings are
`logLevel` (1-20, default 20) and `startupClear` (default true). Nix authors
declare a `Device` under `d2b.zones.<zone>.resources.<name>` with
`providerRef = "Provider/device-tpm"`, `deviceClass = "emulated"`, and an
exclusive arbitration. State directories and Provider artifacts are never
Device-spec fields.

## Managed resources and placement

The controller manages one emulated Device and creates one pre-start
EphemeralProcess followed by one long-lived swtpm Process. Both run on the
Host under Core's Process controller. The persistent TPM Volume is independent
and is not owned by the Device finalizer.

## Dependencies, RBAC, and security

The controller reads its Zone Device, Volume, and Process dependencies, writes
only its own status/finalizer, and relies on Core's injected
`TpmEffectPort`. Core maps opaque state intents to audited state-directory
hardening and runner launch. No path, fd, broker socket, executable path, or
ambient capability enters this crate. A missing or mismatched tamper marker
fails closed.

## Telemetry and audit

Provider metrics use fixed semantic labels only. Zone/resource names, state
paths, TPM contents, and identity material never become labels or diagnostics.
Core owns the path-free effect audit record.

## Build and test

```bash
cargo test -p d2b-provider-device-tpm
cargo xtask check-provider-layout
```

Hermetic tests use fake effect ports. The `integration/` scenarios require the
existing container or Host integration lane.

## Future standalone use

The crate boundary intentionally depends on `d2b-contracts` plus serde only.
An extracted repository would retain the Provider identity, signed component
descriptor, and Core effect-port adapter while replacing workspace packaging.

