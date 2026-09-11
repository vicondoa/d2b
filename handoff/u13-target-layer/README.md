# U13 target-layer handoff (delete after applying)

Two hunks that wire the U13 target layer into the composition. They are **not**
applied anywhere: they are parked here so the merge owner can apply them in the
fixed merge order.

| File | Base it applies to | Verified how |
| --- | --- | --- |
| `u13-wiring-resource-runtime.patch` | primary-tree bytes of `manager.rs` / `resource.rs` / `context.rs` (commit `562d9b58b`) | `git apply --check` against `/home/paydro/projects/d2b` clean; applied in the U13 worktree over those bytes: `cargo test -p d2b-resource-runtime` 77 passed / 0 failed (includes the new manager-level target test), then reverted clean |
| `u13-wiring-plane.patch` | `packages/d2bd/src/resource_plane_v3.rs` at HEAD `63d4fe5aa` | `git apply --check` against `/home/paydro/projects/d2b` clean; with both hunks applied, `cargo check -p d2bd --features test-support --tests` finishes clean |

`u13-wiring-resource-runtime.patch` touches `context.rs` and `manager.rs`, which
another lane owns; it is a patch rather than a commit for that reason (see the
U13 report). `u13-wiring-plane.patch` is the composition-side wiring for the
resource plane and is gated on the U9 conversion: its call sites must be rebased
if U9 moves the plane bring-up.

Still to be written by the U9-integrated plane, and deliberately not faked here:
the `GuestTargetControl` implementation over the guest's target-control path
(`GuestComponentSessionClient` -> guest target runtime). `ResourcePlaneV3::
bind_guest_target` takes exactly that port.
