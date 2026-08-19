### Added

- Host integration coverage for authenticated resource activation, restart
  adoption, and Provider catalog wiring.
- Cloud Hypervisor Guest coverage now reports missing nested TAP/cgroup
  posture as an explicit preflight block instead of claiming adoption.

### Fixed

- Public Network reconciliation now runs the ordered Network Provider
  controller before Guest launch, including its host-effect and child-readiness
  barriers.
- Public resource reconciliation preserves typed Get failures such as
  not-found and resource-plane-unavailable.
- Wayland version filters reject zero-valued protocol versions.
- TPM migration accepts trusted setgid-inherited file groups while rejecting
  unsafe parent directories.
