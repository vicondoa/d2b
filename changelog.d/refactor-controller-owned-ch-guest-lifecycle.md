### Added

- Added exact peer-bound controller ResourceV3 sessions with bounded bootstrap descriptor handoff and scoped Cloud Hypervisor assignments.
- Added a strict private Cloud Hypervisor Guest setup descriptor, deterministic child ResourceRefs, UID-free create batches, and private runtime identity fencing.

### Fixed

- Revoked stale controller assignments on disconnect or replacement and retried stale lease revocations during refresh.
- Reused Bazel's pinned nixpkgs input across isolated Nix unit surfaces instead of resolving the Git input during every test.
