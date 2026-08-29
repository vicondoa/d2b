### Removed

- Removed the in-repository Gas City packages, modules, fixtures, host
  integration, smoke check, flake outputs, Bazel registrations, retired ADRs,
  and contributor environment plan.
- Contributor orchestration now lives in the standalone
  [d2b-gascity](https://github.com/vicondoa/d2b-gascity) repository, while
  NixOS host distribution and installation live in
  [gascity.nix](https://github.com/vicondoa/gascity.nix). Consumers must first
  cut over to those projects, then run the standalone smoke checks and capture
  rollback evidence, before adopting the d2b revision that removes the old
  exports. Rollback means pinning the prior d2b revision; d2b does not claim
  that external proof was run locally.
- d2b does not migrate, delete, chmod, chown, or sweep existing
  `/var/lib/gascity*` or `/run/gascity*` state.
