# `d2b-provider-user` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`user.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: a declared `User` row resolved through the daemon's
discovery effect port against the host's own account database, with the
resulting phase and opaque identity digest published as the row's status and a
discovery that cannot complete reported as a retryable failure.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
