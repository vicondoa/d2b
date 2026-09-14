# `d2b-provider-command` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`command.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: the seed commits Command rows, the process controller materializes one spawn operation per command, and a command whose argv placeholder names no declared parameter is refused at seed.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
