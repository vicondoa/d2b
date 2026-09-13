# `d2b-provider-host` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`host.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: a committed `Host/host-system` row observed through
the daemon's Host effect port, with the probe's bounded capability,
platform, and metadata observations published as the row's status and the
degraded fallback taken when the probe cannot complete.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
