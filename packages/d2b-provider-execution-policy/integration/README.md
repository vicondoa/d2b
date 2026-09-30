# `d2b-provider-execution-policy` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`execution-policy.rs` is declaration-only. No current Cargo target or
repository lane compiles it, so it is not test evidence. It records the
intended host-integration scenario: an ExecutionPolicy row is committed, an
execution instance selects it under granted authorization, a backend missing
one of the policy's mandatory confinement facets is refused instead of
launched without it, and the compiled seccomp filter the broker loads matches
the selected SeccompProfile row.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
