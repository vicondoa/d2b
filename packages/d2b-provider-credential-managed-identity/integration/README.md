# `d2b-provider-credential-managed-identity` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

Required scenarios are `container-service`, `host-guest-placement`,
`aca-credential-ref`, and `cleanup-rollback`. They use fake IMDS and must prove
the ACA bundle carries `credentialRef` without a raw managed-identity client-ID
field. No test contacts live IMDS or Azure.

Each future `integration/*.rs` file must carry exactly one
`//! integration-target: container` or `//! integration-target: host-integration`
declaration in its first twenty lines.
