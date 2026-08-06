# `d2b-provider-system-core` integration fixtures

## Purpose

This directory is reserved for the container, Host, Guest, cross-process, and
provider-system fixtures for `Provider/system-core`. They cannot run at the
hermetic layer that `tests/` occupies.

## Status

No executable fixture is wired yet. The current crate contains the required
integration README and hermetic coverage only; it does not claim bootstrap
runtime behavior.

## Scenarios

No scenario is declared yet. Future coverage must exercise the fixed
core-controller placement without replacing it with a fake that claims to be
the bootstrap path.

## Target declaration

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.

## Run

When a scenario is wired, use `make test-integration` or
`make test-host-integration` according to its declaration.

## Related guide

See [Create a Provider](../../../docs/how-to/create-provider.md).
