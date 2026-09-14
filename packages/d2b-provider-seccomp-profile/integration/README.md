# `d2b-provider-seccomp-profile` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`seccomp-profile.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: the seed commits SeccompProfile rows, a role referencing an uncommitted profile is refused, and the compiled posture the broker applies matches the committed row.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
