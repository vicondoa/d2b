# `d2b-provider-credential-managed-identity` integration fixtures

## Purpose

This directory is reserved for the container, Host, Guest, cross-process, and
provider-system fixtures for `Provider/credential-managed-identity`. They
cannot run at the hermetic layer that `tests/` occupies.

## Status

No executable fixture is wired yet. The current crate contains the required
integration README and hermetic coverage only; it does not claim live IMDS or
Azure runtime behavior.

## Scenarios

The planned scenarios are `container-service`, `host-guest-placement`,
`aca-credential-ref`, and `cleanup-rollback`. They use fake IMDS and must prove
the ACA bundle carries `credentialRef` without a raw managed-identity client-ID
field. No test contacts live IMDS or Azure.

## Target declaration

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.

## Run

When a scenario is wired, use `make test-integration` or
`make test-host-integration` according to its declaration.

## Related guide

See [Create a Provider](../../../docs/how-to/create-provider.md).
