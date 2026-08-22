### Changed

- Moved the credential-bearing Layer-1 PR gate to a default-branch `main`
  workflow accepted for protected `main` and `v3` targets. Remote-eligible
  Rust and policy actions use BuildBuddy, while Nix, fixture, and other
  local-only actions remain local under the existing fixed target sets and
  stable `check` result. Protected `main` and `v3` pushes retain trusted cache
  seeding.

### Security

- Added trusted/bootstrap checkouts, immutable PR and run metadata validation,
  PR/head cache namespaces, and an anonymous-memfd BuildBuddy credential
  boundary. Missing or failed remote authentication now fails closed in the
  credential-bearing CI path. Pull-request target runs source fixed suite
  definitions and workflow/Make control files from immutable default-branch
  `main`, while protected pushes use their immutable event commit. The tier-0
  preflight remains local and credential-free. Credential-bearing jobs reject
  local action execution and run local policy checks on a fresh
  credential-free runner.
