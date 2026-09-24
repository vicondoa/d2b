# U18 d2b-session-unix
net: -6 lines, -0 deps

- delete `is_guest_control_transport` (vsock.rs:23) - the only workspace refs are the definition and the `pub use` name at lib.rs:235; zero callers anywhere (no tests, benches, d2bd, d2bd-runtime, d2b-provider-*, docs, or nix modules - verified workspace-wide with `grep -rn "is_guest_control_transport\b" packages --include=*.rs`). Keep the live sibling `is_guest_control_transport`? No - the live admission gate is `is_guest_control_transport`'s counterpart `FramedVsockTransport` whose `with_descriptor` route uses `controller_bootstrap_attachment_policy()`; the guest-control mouth is reached through `guest_control_transport_descriptor()` + `FramedVsockTransport`, never through this predicate. [packages/d2b-session-unix/src/vsock.rs:23] (leaf)

## Checked
Read all of `src/` (adapter.rs 1237, descriptor.rs 689, vsock.rs 668, socket.rs 131-818, pidfd.rs 173, systemd.rs 437, subject.rs 137, credit.rs 292, zone_admission.rs 129, error.rs 101, vsock.rs vsock.rs) and `lib.rs` full re-export surface; then workspace-wide caller verification for the whole public surface (`controller_*_policy` / `is_guest_control_transport` / `FramedVsockTransport` / `NativeVsockListener` / pidfd verifyrs). Policy factories and credits are live (d2bd, d2bd-runtime, provider-toolkit, zone-routing, providers, tests). `is_guest_control_transport` is the sole zero-caller item; everything else on the surface has at least one reachable consumer. No open prior findings for this crate (U18 ledger: none); no generated/ or integration-scaffold material implicated.

## U3 outcome (2026-09-24)
- applied: is_guest_control_transport (vsock.rs) + its `pub use` arm (lib.rs). R4 at HEAD: definition + export arm only.
