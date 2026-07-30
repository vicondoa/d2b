# `d2b-provider-system-systemd`

The `system-systemd` Process Provider controller.

The sections below are the structure every Provider crate README must carry.
They are placeholders here: each is filled by the slice that implements
`ADR046-provider-003`. Nothing recorded here is a design statement.

## Provider identity

Not yet declared. Filled by `ADR046-provider-003`.

## Config schema

Not yet declared. Filled by `ADR046-provider-003`.

## Exported resource types

Not yet declared. Filled by `ADR046-provider-003`.

## Controllers / services / workers / binaries

Not yet declared. Filled by `ADR046-provider-003`.

## Placement and dependencies

Not yet declared. Filled by `ADR046-provider-003`.

## RBAC requirements

Not yet declared. Filled by `ADR046-provider-003`.

## Security posture

Not yet declared. Filled by `ADR046-provider-003`. Until then the standing Provider rules
apply unchanged: a Provider performs no privileged mutation, reaches host state
only through an injected typed effect port, and the broker remains the sole
privileged executor and audit owner.

## State and telemetry

Not yet declared. Filled by `ADR046-provider-003`.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-system-systemd
cd packages && cargo clippy -p d2b-provider-system-systemd --all-targets
```
