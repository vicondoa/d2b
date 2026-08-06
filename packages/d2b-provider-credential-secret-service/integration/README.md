# `d2b-provider-credential-secret-service` integration fixtures

## Purpose

This directory is reserved for the container, Host, Guest, cross-process, and
provider-system fixtures for `Provider/credential-secret-service`. They cannot
run at the hermetic layer that `tests/` occupies.

## Status

No executable fixture is wired yet. The current crate contains the required
integration README and hermetic coverage only; it does not claim runtime
Secret Service, D-Bus, systemd, or NixOS behavior.

## Scenarios

The planned scenarios are `container-service` for a fake Secret Service
process, `host-placement` and `guest-placement` for user-domain placement, and
`cleanup-rollback` for generation removal and restoration. These require real
process, D-Bus, systemd, NixOS, or generation behavior and cannot be claimed
by the hermetic fake-port tests.

## Target declaration

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.

## Run

When a scenario is wired, use `make test-integration` or
`make test-host-integration` according to its declaration.

## Related guide

See [Create a Provider](../../../docs/how-to/create-provider.md).
