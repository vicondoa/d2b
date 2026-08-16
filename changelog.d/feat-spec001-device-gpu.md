### Added

- Complete the GPU Device Provider lifecycle with Host-global claim
  admission, opaque worker identities, restart adoption, bounded status and
  telemetry, and fail-closed broker preflight.

### Changed

- Reject GPU Device resources on unsupported host platforms during Nix
  evaluation.
- Make shared Host-global leases unique, validate GPU/video identities and
  closure proofs, recover partial restarts without duplicate workers, and
  reject malformed GPU runner shapes before device opens.

### Fixed

- Keep rejected GPU worker identities owned through finalization so failed
  starts cannot respawn or release Host-global authority before closure.
- Fail closed on ambiguous or quarantined GPU restart adoption to prevent
  duplicate workers.
