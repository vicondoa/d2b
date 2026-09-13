# `d2b-provider-volume-binding` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`binding.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: the binding-owned worker Process and its Endpoint
realized through the daemon's serving effect port, the derived worker plan
re-derived across a restart, and the drain gate holding while the target Guest
observes the mount.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
