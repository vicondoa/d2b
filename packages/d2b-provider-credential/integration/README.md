# `d2b-provider-credential` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`credential.rs` is declaration-only. No current Cargo target or repository
lane compiles it, so it is not test evidence. It records the intended
host-integration scenario: a managed-identity Credential reconciled through
the daemon's effect port mints its declared agent Process child under
`Provider/system-minijail` and reaches `Ready` once the child serves, and a
delete revokes the lease through the authenticated Provider session before
the owned child row is marked deleting.

Each scenario file carries exactly one `integration-target` declaration in
its first twenty lines. A real-boundary claim remains deferred until
repository orchestration compiles and invokes that scenario.
