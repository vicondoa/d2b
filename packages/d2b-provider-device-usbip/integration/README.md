# `d2b-provider-device-usbip` integration fixtures

`attach_detach_lifecycle.rs` declares `host-integration` in its first line. The
scenario belongs to `make test-host-integration`, which runs through the shared
heavy-gate semaphore. It requires a booted NixOS test Host with the USBIP host
and Guest modules, Provider process lifecycle, nftables, Network relay, and a
fake approved USB backend. KVM is preferred but the host-integration lane may
use its documented fallback.

The scenario must prove one Host backend, one relay authority per Network,
exact per-device projection apply and remove, sibling Network marker
preservation, Guest attach and detach, and wrong-Zone rejection before effect.
No ordinary integration run may use an operator's physical device. Real device
coverage is manual-only under the repository hardware gate.

New Rust scenarios must declare exactly one `//! integration-target: container`
or `//! integration-target: host-integration` line in their first 20 lines and
must communicate through the public Zone API or integration harness rather than
importing this crate's source directly.
