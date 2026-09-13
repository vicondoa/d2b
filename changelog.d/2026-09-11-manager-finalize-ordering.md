### Changed

- Added a `finalize` step to the `ResourceDriver` contract and made the
  delete pass run it before the driver's delete: the erased actor boundary
  calls `ResourceContext::finalize_owned_resources()`, which returns
  `ChildrenDraining` while any owned child row is still live. All converted
  drivers (`activation`, `binding`, `credential`, `core`, `endpoint`,
  `guest`, `interaction`, `process`, `shared-provider`, `system-core`, plus
  the telemetry driver) now finalize their owned children before their own
  provider teardown stage, so a parent's teardown can never run ahead of a
  live child.
- The manager holds a cleanup-completed parent row (including its durable
  deleting mark) until its owned children retire, and the delete pass gates
  each level on its children draining.

### Fixed

- Restart re-links ownership before spawning actors: rows load in
  `(zone, type, name)` order, so a Volume chain's children could load before
  their owning binding and lose their owner edges; the manager now indexes
  every row and links ownership in a second pass. A crash mid-delete
  therefore resumes with the parent held until its children retire instead
  of retiring the parent while they are still tearing down.
