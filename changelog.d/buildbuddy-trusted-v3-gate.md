### Changed

- Moved the credential-bearing Layer-1 PR and push gate to protected `v3`.
  Remote-eligible Rust and policy actions use BuildBuddy, while Nix, fixture,
  and other local-only actions remain local under the existing fixed target
  sets and stable `check` result.

### Security

- Added trusted/bootstrap checkouts, immutable PR and run metadata validation,
  PR/head cache namespaces, and an anonymous-memfd BuildBuddy credential
  boundary. Missing or failed remote authentication now fails closed in the
  credential-bearing CI path. Fixed suite definitions and the workflow/Make
  control files are now overlaid from trusted `v3`, and the tier-0 preflight
  remains local and credential-free. Credential-bearing jobs reject local
  action execution and run local policy checks on a fresh credential-free
  runner.
