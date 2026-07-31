# `d2b-provider-network-local`

The `network-local` Provider.

This crate is scaffolding. The sections below are the structure every Provider
crate README must carry; each is filled by the slice that implements
`ADR046-network-005`. Nothing recorded here is a design statement.

## Provider identity

Not yet declared. Filled by `ADR046-network-005`.

## Config schema

Not yet declared. Filled by `ADR046-network-005`.

## Exported resource types

Not yet declared. Filled by `ADR046-network-005`.

## Controllers / services / workers / binaries

Not yet declared. Filled by `ADR046-network-005`.

## Placement and dependencies

Not yet declared. Filled by `ADR046-network-005`.

## RBAC requirements

Not yet declared. Filled by `ADR046-network-005`.

## Security posture

Not yet declared. Filled by `ADR046-network-005`. Until then the standing Provider rules
apply unchanged: a Provider performs no privileged mutation, reaches host state
only through an injected typed effect port, and the broker remains the sole
privileged executor and audit owner.

## State and telemetry

Not yet declared. Filled by `ADR046-network-005`.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-network-local
cd packages && cargo clippy -p d2b-provider-network-local --all-targets
```
