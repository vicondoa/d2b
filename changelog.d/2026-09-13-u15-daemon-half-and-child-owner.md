### Added

- The daemon/fixture half of the spec's §36 invariant matrix is discharged:
  volume recovery over an existing host layout (adopts, never recreates, one
  layout effect across both driver lifetimes), the ZoneLink product rows
  (ordinary guest realization never creates or moves a link; link topology and
  guest availability move independently), and the shared-provider restart rows
  (the declared child set is adopted after a restart and re-committed when a
  child is missing), with the process/VM rows and the shared-backend limits
  cited to the tests and host-integration stages that already prove them.

### Fixed

- A derived owned-child ensure could silently re-parent an existing child row
  to a different owner. The manager now refuses a child ensure (single or
  declarative diff) whose parent is not the row's committed owner, so the
  durable owner, generation and spec survive and only the committed owner can
  update the child in place. Binding a not-yet-owned row stays the authored
  `Ensure { owner }` decision.
