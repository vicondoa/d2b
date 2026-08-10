# `d2b-provider-credential-entra` integration fixtures

## Purpose

This directory is reserved for the container, Guest, cross-process, and
provider-system fixtures for `Provider/credential-entra`. They cannot run at
the hermetic layer that `tests/` occupies.

## Status

No executable fixture is wired yet. The current crate contains the required
integration README and hermetic coverage only; it does not claim live
Entrablau, TPM, or cloud-token runtime behavior.

## Scenarios

The planned scenarios are `container-service`, `guest-placement`, and
`cleanup-rollback`, using a fake Entrablau login/token Endpoint. CI must not
contact live Entra.

## Target declaration

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.

## Run

When a scenario is wired, use `make test-integration` or
`make test-host-integration` according to its declaration.

## Related guide

See [Create a Provider](../../../docs/how-to/create-provider.md).
