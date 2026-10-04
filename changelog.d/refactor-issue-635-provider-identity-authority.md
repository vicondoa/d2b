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
- Watch conditions are now split into LEVEL and EDGE. A readiness subscription
  is an EDGE - `ReadyChanged`, satisfied when the target ENTERS or LEAVES
  `Ready` - and an EDGE is never answered on arrival, so a row can hold one
  unconditionally against a target that already reports `Ready`. That is what
  the previous conditional (hold the readiness arm only while the target has
  not reported `Ready`) could not do: it left a row unsubscribed to a ready
  target, so a degraded row that publishes no new layer kept reporting `Ready`
  and stayed silent. The evidence arm, `ProjectionChanged`, is an EDGE too.
- The in-flight marker a pass publishes on its way IN is no longer evaluated
  against the registered watches, and is no longer the `before` an EDGE
  condition is compared against. Every pass published `Reconciling` between
  two `Ready`s, which an EDGE condition reads as leaving `Ready` and entering
  it again: so every subscriber of an already-converged row was woken by every
  pass of it, re-armed, and ran a pass whose own publication woke whatever it
  watched. The cost was a cascade that scales with the graph rather than with
  the work - the display acceptance graph took 2598 reconcile passes under the
  full parallel suite where it now takes 425, and the whole suite runs in
  under a third of the wall clock. The same marker no longer resets the
  published projection, so a pass that republishes the layer it published
  before no longer reads as a change. What a pass CONCLUDED with is what an
  EDGE condition is answered against, which is what its name claims.
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
- The blocking-API census is back under its committed baseline. This branch
  added blocking calls and hid them behind new
  `#[allow(clippy::disallowed_methods)]` attributes instead of removing them;
  those calls are gone. The Endpoint and guest-target test doubles take
  `tokio::sync::Mutex` rather than a `std::sync` guard, the Provider process
  doubles take `tokio::sync::Mutex` rather than a `parking_lot` one, the
  broker's dispatch audit test drives the daemon call directly instead of
  parking the executor on an explicit `block_on` plus three `std::fs` reads,
  the plane registry publishes its committed Provider identities once instead
  of guarding them with a read-write lock, and the dead `#[allow]` attributes
  and `// async-gate-allow:` markers those calls needed are deleted with them.
