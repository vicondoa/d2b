### Added

- A Guest realized through the manager now establishes its own authenticated
  ComponentSession, and the acceptance lane can see it. The converted Guest
  effect path resolves the committed Guest and its guest-control Endpoint,
  connects (reusing the live session cache), registers the live generation on
  the Zone target directory, and re-adopts every assignment the directory
  holds for that Guest - so a target-local realization present inside the Guest
  is re-bound to the new generation and a missing one stays for its owning
  actor to realize again, never inherited across a reconnect.
- The target-control session generation is published on every acceptance, not
  only on a replacement: `Guest ComponentSession Resource API server starting`
  is emitted at `info` with the accepted route's own `generation` field, which
  is the same value the Guest binds as its live target-control generation and
  the parent records in the session descriptor. Previously the first
  acceptance after boot published nothing, and the only existing event was at
  `debug` - invisible under the daemon's default `info` filter - so the host
  journal showed a bound listener with no session behind it.

### Fixed

- A legitimately reconnected Guest reports `Ready` again. The Guest
  incarnation fence pinned the session's *live accepted* generation, which
  advances on every reconnect by design (the Guest admits only a strictly
  newer generation - `session_generation_is_fresh`,
  `next_session_generation`), against the enrollment-scoped generations, so
  after a daemon restart the row served `phase: Pending` with
  `runtimeReady: false` / `bootstrapReady: false` forever. The fence now
  carries the *enrolled* identity generation the live session was admitted
  under (the generation the Guest re-verifies as the floor of every
  acceptance); the live accepted generation stays the binding's
  `session_generation`, the freshness marker the lifecycle plans consume.
- A VMM Process whose identity has not actually changed survives a daemon
  restart. The retire-before-launch pre-flight treated a per-pass identity
  input the current pass could not resolve (the owning Guest's uid, resolved
  after the adopting pass seeded the entry) as an identity *change* and
  destroyed the live `cloud-hypervisor-runner`, so every restart relaunched
  the VMM and invalidated the Guest's enrolled boot identity. Retirement now
  requires a *proven* change - both sides known and different - while the
  destructive gates (`finalize`, `stop`, `has_active`) keep the strict
  comparison, so an unresolved request still refuses rather than proceeds.
- A reconcile pass that computed a `status.resource` projection publishes it
  whatever its outcome: `InProgress` (a long effect in flight) used to drop
  the projection it had just computed, leaving the row on the pre-pass layer
  while the driver already knew better. Invalidation - a spec change or
  deletion - is the only drop case.
- The VolumeBinding driver's guest-mount gate reads real evidence again. The
  KTD6 drain gate had no observation surface after the volume-leg conversion
  and answered the documented `Ok(false)` default; it now asks the Zone target
  directory for the row's own assignment and only a target-local realization
  the Guest reports `ready` answers "mounted". No assignment, no live session,
  no recorded realization, and a stale-generation handle all answer "not
  mounted", so a drain never force-clears a serve the target cannot confirm.
