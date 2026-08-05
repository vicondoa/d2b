### Added

- Proposed ADR 0054, selecting one Cargo workspace and dependency hub for d2b
  product packages while retaining the no-bash walker as a separate tooling
  workspace. Targeted broker and guest builds remain separate, and generated
  package-scoped closure inventories preserve privileged and static dependency
  minimality under the shared product lock.
