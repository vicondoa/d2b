### Changed

- A resource read now returns one classified result (`Present`, `Absent`,
  `Unavailable`, `Error`) that also records the plane it was answered from,
  instead of each call site collapsing everything that is not a row into
  "not found". The classification carries its own default: absence and
  unavailability defer and requeue, and only a site that can name its
  terminal evidence reports a terminal failure. A provider or manager that
  cannot answer is therefore no longer indistinguishable from a resource
  that is genuinely gone.
- The guest, volume, binding and endpoint controllers read through the
  classified surface, and the guest effect surface folds an unreadable row
  into one named terminal error that records the plane and the cause.

### Fixed

- A volume binding whose parent volume has not been committed yet no longer
  fails terminally. The absent parent row now requeues, and the owner
  mismatch failure is reserved for a parent row that is present with a
  different owner - so a binding that is simply early no longer reports a
  permanent failure that only a later reconcile could clear.
