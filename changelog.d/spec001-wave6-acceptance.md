### Fixed

- Require the public Network reconciliation path to commit and re-read a
  typed child-readiness projection before publishing durable Ready or
  launching a dependent Guest.
- Preserve typed status resource projections while updating universal phase
  fields, and classify pre-upgrade restore backups as upgrade-required.
- Compare broker runner executables by their canonical targets so trusted
  NixOS symlink paths remain adoptable and liveness probes do not reject live
  runners.
- Preserve broker-owned runner adoption across UID transitions without
  CAP_SYS_PTRACE and resolve Cloud Hypervisor's wire role to its trusted
  bundle intent.
- Bound broker-audit evidence replay pages so restart recovery stays within
  the broker transport frame limit.
- Give broker-spawned swtpm runners separate control and Cloud Hypervisor
  server sockets so TPM-backed Guests can boot.
- Carry each VM's explicit autostart policy into the public manifest so
  d2bd does not boot workloads declared `autostart = false`.
