# `d2b-provider-credential-secret-service` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

Required scenarios are `container-service` for a fake Secret Service process,
`host-placement` and `guest-placement` for user-domain placement, and
`cleanup-rollback` for generation removal and restoration. These require real
process, D-Bus, systemd, NixOS, or generation behavior and cannot be claimed by
the hermetic fake-port tests.

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.
