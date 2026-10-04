### Fixed

- The interaction family's re-arm now reads the SPEND of an internal-watch
  registration instead of the target's `(status, status.resource)` pair. A
  target pass satisfies every registration a row holds on it - the entry
  `Reconciling` publishes no projection and the exit publishes `Ready` - and
  then republishes the very pair it published before, so a row that compared
  pairs read a spent registration as a standing one and placed neither. An
  interaction row woken by such a pass was left subscribed to nothing on that
  target, was never woken by it again, and scheduled no requeue of its own
  (`mutated || !ready`), so it kept publishing `Ready` over a child or
  dependency that had moved.
- A readiness registration is now held only while its target has not reported
  `Ready`. The runtime answers a registration whose condition already holds on
  arrival, so a readiness arm against a ready target is spent before it can
  stand, and re-arming it on every spend would wake the row for that answer
  alone. The evidence arm, which no arrival can satisfy, carries every later
  change under a phase that never moves.
- A watch pair that could only be half placed is now handed back whole, and a
  registration whose release the manager refused stays recorded. Neither leaves
  a registration standing in a target actor's mailbox that no pass remembers,
  so the next pass can no longer stack a second arm on the same target. A
  target this pass stops watching has its arms released for the same reason.
- `ResourceContext` now records the internal-watch registrations this row
  still holds (`watch_is_live`) and takes a target's satisfaction as the
  record of a spend (`mark_watch_spent`, applied by the actor as it handles
  that satisfaction). The registrations travel across the context rebuild a
  spec change performs, because they live in the target actors' mailboxes and
  not in the context.