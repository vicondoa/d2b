# `d2b-provider-network-local` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

No fixture is declared yet. Each scenario file added here must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines, and is invoked by the existing repository
test orchestration for that target.
