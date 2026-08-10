# `d2b-provider-system-minijail` integration fixtures

## Purpose

This directory is reserved for the container, Host, Guest, cross-process, and
provider-system fixtures for `Provider/system-minijail`. They cannot run at the
hermetic layer that `tests/` occupies.

## Status

No executable fixture is wired yet. The current crate contains the required
integration README and hermetic coverage only; it does not claim a runtime
minijail or broker launch path.

## Scenarios

No scenario is declared yet. Future coverage must exercise the opaque
Provider request and broker launch boundary rather than a local spawn stub.

## Target declaration

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.

## Run

When a scenario is wired, use `make test-integration` or
`make test-host-integration` according to its declaration.

## Related guide

See [Create a Provider](../../../docs/how-to/create-provider.md).
