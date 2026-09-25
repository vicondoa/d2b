### Changed

- Guest, notification-desktop, process-systemd and volume providers no longer
  allocate a fresh string per hex byte when building operation ids, unit
  names and projection keys: each id is written into one preallocated buffer,
  and notification close paths reuse the request id they already hold instead
  of re-formatting it from the numeric id.
- The volume provider reconcile pass performs one manager child-set fetch per
  pass instead of two: the child set fetched to retire obsolete bindings is
  reused for the convergence verdict.