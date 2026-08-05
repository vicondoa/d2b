# volume-virtiofs integration fixtures

## Purpose

This directory is reserved for the heavier fixtures for
`Provider/volume-virtiofs`. They need a real virtiofsd binary, a listening
socket, and a booted Guest, so they cannot run at the hermetic layer that
`tests/` occupies.

## Status

No executable fixture is wired yet. The effect adapter these fixtures would
drive is owned by ProviderSupervisor and is not landed. The hermetic suite
still covers the fixed argv envelope, private socket identity, store-view
marker gating, guest-mount readiness classification, Export finalizer
ordering, and ADR 0021 template/map-write invariants.

## Scenarios

| Fixture | What only a real launch can prove |
| --- | --- |
| worker launch | the frozen argv is accepted by the shipped virtiofsd build |
| user namespace | the single-entry mapping is in place before the worker's first instruction |
| socket readiness | the private socket listens and carries the resolved group |
| guest mount | the Guest observes the share at its mount path with the expected access |
| finalizer drain | the mount is gone after the worker is deleted, across a Guest restart |

## Target declaration

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.

## Run

When a scenario is wired, use `make test-integration` or
`make test-host-integration` according to its declaration.

## Related guide

See [Create a Provider](../../../docs/how-to/create-provider.md).
