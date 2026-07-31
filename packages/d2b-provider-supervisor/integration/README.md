# `d2b-provider-supervisor` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

`broker_spawn.rs` is a declaration-only container scenario for a real broker
`SpawnRunner` round trip, `SCM_RIGHTS` pidfd handoff, process-start-time recheck,
broker wait/reap ownership, and trusted user-namespace/cgroup placement.

`systemd_transient.rs` is a declaration-only booted-host scenario for a real non-forking
transient unit, atomic identity query, descriptor reopen, unit disappearance,
service-manager wait/reap ownership, and verified user scope.

No current Cargo target or repository lane compiles these files. They are not
test evidence, and their real-boundary claims remain deferred until repository
orchestration compiles and invokes them.
