# `d2b-provider-provider` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

`provider.rs` is declaration-only. No current Cargo target or repository lane
compiles it, so it is not test evidence. It records the intended
host-integration scenario: the daemon plane commits a Provider row beside its controller Process and state Volume children, the driver publishes Pending while the controller session evidence is absent, publishes Ready once the evidence is live, and gates the row's retirement on the controller Process child.

Each scenario file carries exactly one `integration-target` declaration in its
first twenty lines. A real-boundary claim remains deferred until repository
orchestration compiles and invokes that scenario.
