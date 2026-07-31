# `d2b-provider-supervisor` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

`broker_spawn.rs` declares the container scenario for a real broker
`SpawnRunner` round trip, `SCM_RIGHTS` pidfd handoff, process-start-time recheck,
broker wait/reap ownership, and trusted user-namespace/cgroup placement.

`systemd_transient.rs` declares the booted-host scenario for a real non-forking
transient unit, atomic identity query, descriptor reopen, unit disappearance,
service-manager wait/reap ownership, and verified user scope.

Each file is a scenario contract, not evidence by itself. The matching existing
repository integration lane must invoke it before those real-boundary claims are
made.
