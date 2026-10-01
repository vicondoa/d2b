### Added

- The deployment document a Guest image carries is now a rendered fixture
  rather than a stand-in: the fixture aggregate materializes
  `deployment-bootstrap-work.json`, which is
  `nixos-modules/deployment-bootstrap.nix`'s own `documentFor "work"` output,
  and Rust contract tests read those bytes. `DeploymentBootstrap::decode`
  verifies the rendered publication under the deployment-root name the
  daemon and broker read, the daemon's guest boot path publishes it, and the
  implementation identities it declares are checked against the provider
  registrations this build compiles, so the producer and the verifier cannot
  drift apart unnoticed.
- The broker's installed admitted-effect table now has an owner: a case in
  the daemon runtime module drives the real admission over the wiring the
  serve path installs and pins that the empty live table refuses a
  well-formed carrier with `unknown-implementation`.

### Changed

- The Guest ComponentSession eval case that delivered a two-field stand-in
  graph now derives the delivery path from the deployment constructor's own
  file name, and is named for the delivery it proves rather than for a
  producer/verifier agreement it does not check.
- The admitted-effect integration test states in its own header that its
  table and private execution values are test-owned fixtures, and that
  production installs the empty table.