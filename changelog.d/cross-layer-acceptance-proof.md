### Added

- Added the cross-layer acceptance proof: one Provider declaration carried from
  the declaration compiler through the per-Zone manager and the store's
  authority publication to a handler invocation. A signed in-memory Provider
  artifact naming a launchable controller is compiled and projected by the real
  compiler, the `Provider` row and the controller `Process` row it generated are
  committed by the real manager over the real store, both are published to the
  broker's real admitted authority projection through the production
  publication coordinator, and the production `Provider` driver then reconciles
  the row and publishes what it observed. The assertion is the far end's: the
  handler's own published observation has to show that it read the row the
  compiler generated, as that Provider's owned child with the compiler's own
  `processClass`, `providerRef`, and `executionRef`, owned by the uid the store
  committed, and that it resolved that child's controller session against the
  Provider's own committed identity and generation. A companion case drives the
  same chain with a publication identity the Zone's accepted graph holds no
  grant for: the broker refuses the candidate with its own closed refusal code,
  authorizing stage, and identity-not-authorized reason, the Zone is left fenced
  with its accepted cursor unmoved, the store commits no row, and the handler
  never runs.
  Each case is proven discriminating by breaking the chain at one
  seam at a time and watching the matching case fail: rewriting the controller
  row the compiler projects, making the Provider handler stop reading its
  owned children, and making the store commit without the broker's fence each
  break the admitted case; making the broker's own authorization admit a
  candidate whose Zone holds no grant for it breaks the refused case.