# `d2b-provider-volume` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`volume.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: the volume-local layout effect realized through the
daemon's volume effect port while the derived `VolumeBinding` children are
committed, adopted across a restart, and torn down.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
