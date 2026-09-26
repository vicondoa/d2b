### Changed

- The daemon's interaction runtime set holds one independently lockable
  composition per Zone and hands that handle out instead of exposing the
  compositions themselves. A ComponentSession dispatch holds only its
  own Zone for the length of a request, so a second Zone's dispatch and
  the VM-start display reconcile no longer wait behind an in-flight
  request; the daemon-global slot is an install/teardown handle that is
  released before any Zone is locked.
- The synchronous ComponentSession driver lookup and the daemon-wide
  finalization take the daemon-global slot only to clone the per-Zone
  handles, then lock each Zone in turn, so a contended Zone is reported
  once it is free rather than as an absent source.
- The shared Provider effect adapter resolves its Zone resource runtime
  by awaiting the plane slot, so concurrent reconcile and attach traffic
  serializes on the slot instead of spinning an executor worker. The
  synchronous GPU authority seats keep a non-blocking seat and report
  the runtime unavailable on a collision instead of spinning.

### Fixed

- The Network runtime facet's trusted-bundle reload is awaited end to
  end: the `NetworkRuntime::bundle` seat and its callers are async, and
  the reload runs on the bundle loader worker, so serving a bundle fact
  no longer spins a worker on the bundle slot or on the synchronous
  read-and-verify of the on-disk bundle.
