# `d2b-provider-resource-export` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`resource-export.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: a ResourceExport row is committed for an exportable subject, the driver converges it without a host effect, and a non-exportable subject is refused before any row is written.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
