### Added

- The daemon now reads, verifies, and publishes the deployment's verified new
  deployment graph before the generation publication, before any Zone plane
  opens its store, and before any provider controller activates. The graph is
  self-hashed with the framed canonical digest the Nix bundle compiler and the
  artifact catalog already share, gated on the release's schema tag, and bound
  against the compiled provider registration table, so there is no separate
  configurable implementation allowlist on either side of the boundary.
- A tampered graph, a document of another contract version, an implementation
  this build does not compile, an unorderable publication plan, and a verified
  graph missing a required foundation `RoleBinding` each refuse startup. The
  refusal is terminal for the resource plane rather than falling back to an
  admission that allows every mutation.
- The publication is ordered: the fixed foundations and the deployment's own
  state `Volume` publish first, and every declared provider follows. The
  state's `Volume` is a foundation row precisely so a provider that keeps
  component state in it never waits on a row only that provider could create.
- A Guest reads and verifies the same document at its own deployment root and
  publishes only target-local authority: its own Zone, its own bindings, no
  host surface, and no credential custody. A publication that asks for any of
  those is refused rather than narrowed.
- The Activation family verifies the same bytes and refuses to plan a runner
  or dispatch its handoff effect for a deployment that did not publish its own
  compiled implementation identity.
- `zone-resources.nix` derives the deployment graph from the generated
  provider projections, and `host-daemon.nix` installs the rendered document
  into the deployment root before `d2bd` starts. The daemon and the broker
  share one deployment root, so both halves of the trust root read one
  document. This adds no root-visible unit: the daemon/broker root-unit
  boundary is unchanged.
