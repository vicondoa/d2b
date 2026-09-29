# `d2b-provider-device-usbip` integration fixtures

`attach_detach_lifecycle.rs` declares `host-integration` in its first line. The
scenario belongs to `make test-host-integration`, the Bazel lane
(`//bazel/checks/vm:host_integration_lane_run`). It requires a booted NixOS
test Host with the USBIP host and Guest modules, Provider process lifecycle,
nftables, Network relay, and a fake approved USB backend. The lane declares
`/dev/kvm` as a precondition with no emulation fallback and is x86_64-linux
only; it is a local pre-PR surface, not a CI gate.

The heavy-gate semaphore this scenario used to run behind was deleted with the
rest of the repository's heavy-gate orchestration (`CHANGELOG.md`: "Remove the
heavy-gate semaphore, self-reexec guards, and host provisioning"). Nothing
replaces it: the lanes run their own work directly, and nothing serializes
concurrent heavy lanes on one host.

The scenario must prove one Host backend, one relay authority per Network,
exact per-device projection apply and remove, sibling Network marker
preservation, Guest attach and detach, and wrong-Zone rejection before effect.
No ordinary integration run may use an operator's physical device. Real device
coverage is manual-only under the repository hardware gate.

New Rust scenarios must declare exactly one `//! integration-target: container`
or `//! integration-target: host-integration` line in their first 20 lines and
must communicate through the public Zone API or integration harness rather than
importing this crate's source directly.
