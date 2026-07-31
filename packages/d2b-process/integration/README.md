# `d2b-process` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

`pidfd_identity.rs` declares the container scenario for a real child process,
pidfd lifecycle, exit readability, and process-start-time drift rejection. It
does not claim broker sandbox or systemd behavior.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines and must be invoked by the repository orchestration for that
target before its real-boundary claim is made.
