# `d2b-provider-credential-entra`

The `credential-entra` Provider.

This crate is scaffolding. The sections below are the structure every Provider
crate README must carry; each is filled by the slice that implements
`ADR046-cred-entra-001`. Nothing recorded here is a design statement.

## Provider identity

Not yet declared. Filled by `ADR046-cred-entra-001`.

## Config schema

Not yet declared. Filled by `ADR046-cred-entra-001`.

## Exported resource types

Not yet declared. Filled by `ADR046-cred-entra-001`.

## Controllers / services / workers / binaries

Not yet declared. Filled by `ADR046-cred-entra-001`.

## Placement and dependencies

Not yet declared. Filled by `ADR046-cred-entra-001`.

## RBAC requirements

Not yet declared. Filled by `ADR046-cred-entra-001`.

## Security posture

Not yet declared. Filled by `ADR046-cred-entra-001`. Until then the standing Provider rules
apply unchanged: a Provider performs no privileged mutation, reaches host state
only through an injected typed effect port, and the broker remains the sole
privileged executor and audit owner.

## State and telemetry

Not yet declared. Filled by `ADR046-cred-entra-001`.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-credential-entra
cd packages && cargo clippy -p d2b-provider-credential-entra --all-targets
```
