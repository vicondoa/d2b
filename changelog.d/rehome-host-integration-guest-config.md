### Added

- Added `nix/test-support/host-integration-node.nix`, the reusable d2b
  host-integration guest configuration, so a check's guest declaration
  outlives the `runNixOSTest` fixture that used to carry it. The module
  names both guest shapes the lane has to reproduce rather than collapsing
  them: `d2bDaemonNode` attaches a dedicated `/var/lib/d2b` state disk and
  keeps the VM module's writeback root cache, while
  `d2bCloudHypervisorNode` drops that disk, replaces the root drive's cache
  with `unsafe`, and boots through a bootloader so `/nix/store` and
  `/var/lib/d2b` share one filesystem for the hardlink farm. A per-check
  module passed as `extra` merges through the same import the fixture used,
  so its own memory, vCPU, disk, drive, and device declarations reach the
  same options list the shape contributes to. The lane reads each check's
  invocation - memory, vCPU count, disk size, `useBootLoader`,
  `virtualisation.qemu.drives`, and `virtualisation.qemu.options` - back off
  the evaluated configuration instead of booting one uniform guest.

### Changed

- Split `tests/host-integration/lib.nix`. What stays is the part bound to
  the test driver: the diagnostics prelude, the nested guest systems only
  these fixtures boot, and the provider artifacts only these fixtures
  install. The eight fixtures that boot a d2b daemon host now import the
  re-homed module directly. No fixture's assertions changed.
