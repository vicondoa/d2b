### Fixed

- Controller resource snapshots now carry the singular owner as one `OwnerIdentity` (uid + generation) instead of two independently settable fields, so a snapshot can no longer hold an owner uid without its generation.