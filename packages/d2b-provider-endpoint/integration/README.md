# `d2b-provider-endpoint` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`endpoint.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: the binding-owned virtiofsd socket realized and
removed through the daemon's endpoint effect port while the producer worker
Process row is live, and the guest-runtime control endpoints observed through
the guest's committed VMM Process row.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
