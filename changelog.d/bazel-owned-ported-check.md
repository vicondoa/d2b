### Added

- The host-integration lane can now run a check whose assertions are the lane's own Rust. A ported check is a function over the lane's guest-control surface (`packages/d2b-test-vm-harness/src/checks/`), declared in that crate's table of ported checks, and its guest image carries no `check.py`: the image's manifest says which side of the lane carries the check's assertions, so a check that has been ported and a check that has not are both ordinary checks to the pool, the inventory and the report. A ported check reports under its own result entry with its own diagnostics, and a check whose image says it asserts in Rust and which has no module in the lane is a lane failure rather than a check that quietly does not run.
- The guest-control surface gained the diagnostics primitives a ported check asserts through, in the same one place the fixtures' Python prelude provides them: `stage` (the phase a failure names), `diag` (a labelled diagnostic command that is never fatal), `diag_unit` (a unit wait that prints the unit's status, its journal and the zone's debug dump when it does not settle) and `diag_wait` (the same for a command wait, with the wait and the row set named). A ported check's failure therefore prints the stage it was in, the rows it was asserting on, and the lines that explain them, in the prelude's own order and wording.
- The lane tolerates a check with no fixture end to end. The guest-image action accepts a check declared by name (`ported_check`) as well as one declared by its fixture, and reads the guest such a check boots out of the reusable node module's table of ported checks (`nix/test-support/host-integration-node.nix`); the image is built from that node exactly as a fixture's guest was, and keeps the check's driver name as a filter alias.

### Changed

- Ported `daemon-smoke` to Rust: its assertions moved unchanged into `packages/d2b-test-vm-harness/src/checks/daemon_smoke.rs` - the same stages, the same commands, the same 30s and 180s bounds, in the same order - and the guest it boots (the reusable daemon node plus `jq`) is now declared in `nix/test-support/host-integration-node.nix` rather than in the fixture.

### Removed

- Removed `tests/host-integration/daemon-smoke.nix`. The check it declared is the same check, with the same guest and the same result in the lane; what is gone is the fixture the assertions used to be written in.
