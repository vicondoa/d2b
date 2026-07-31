# `d2b-provider-credential-entra` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

Required scenarios are `container-service`, `guest-placement`, and
`cleanup-rollback`, using a fake Entrablau login/token Endpoint. CI must not
contact live Entra. A real Entrablau identity-Guest login, TPM state, and cloud
token flow remains a separate manual external integration obligation.

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.
