### Changed

- Move Cloud Hypervisor Guest lifecycle ownership into the authenticated
  controller path, including restart adoption, session recovery, finalizer-safe
  deletion, and Bazel-injected host integration.
- Treat generated Guest store-view Volumes as configuration-owned watched
  dependencies instead of Cloud Hypervisor owned children.

### Fixed

- Prevent controller session and Process watch work from blocking daemon
  startup or Resource commits, and preserve exact running Process identity
  across status-only revision changes.
- Make eligible Resource deletion and final-finalizer cleanup complete through
  foreground store garbage collection.
- Keep inherited controller-session descriptors and bootstrap readiness probes
  compatible with the static Guest build's safe-Rust policy.
- Bound Cloud Hypervisor runner ACL grants to the broker-owned state root and
  avoid mutating world-traversable ancestors.
- Qualify Guest store-view intents, runner identities, and cgroup leaves by
  Zone so same-named Guests remain isolated.
