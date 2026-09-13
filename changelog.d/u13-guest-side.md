### Added

- Guest-side target-control service (U13 guest half): the
  `d2b.target-control.v1.TargetControl` ttrpc method is registered on the same
  authenticated ComponentSession that carries the Guest-local Resource API, so
  a Host-zone resource can realize target-locally inside a Guest through the
  published `GuestTargetRuntime`. Frames are decoded with the U13 codec; a
  frame that is unreadable or carries a foreign protocol token is refused
  before any dispatch.
- The live session generation is bound from every accepted session's
  authenticated route (`GuestTargetService::bind_session`), so the fence is
  the daemon's actual live generation: a request naming a non-live generation
  answers `SessionUnavailable` and performs no effect.
- The consumption path for converted types: each admitted realize is applied
  through the target-local effect code registered for its resource type, with
  `specDigest` recomputed from the exact carried bytes before the effect runs.
  A realization is reported `ready` only after its effect is serving; a failed
  effect leaves it `realizing` for the owning Host driver to retry. Adoption
  after a reconnect re-discovers the effect and reports `missing` when it is
  gone.

### Notes

- No converted type registers Guest-side target-local effect code yet, so the
  service admits no type in production today: realize frames for types without
  effect code are refused rather than recorded. Process and EphemeralProcess
  effects keep running through the preserved `run_guest_process_reconciliation`
  path, which this service does not replace. Registering a type's effect code
  is what admits it.
