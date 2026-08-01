# `d2b-process` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

`pidfd_identity.rs` is declaration-only. No current Cargo target or repository
lane compiles it, so it is not test evidence. It records the intended container
scenario for a real child process, pidfd lifecycle, exit readability, and
process-start-time drift rejection without claiming broker sandbox or systemd
behavior.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
