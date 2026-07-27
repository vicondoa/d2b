### Fixed

- The directly invokable heavy-gate self-guard now removes inherited Cargo and
  Rust compiler shell functions before building its verifier, and explicitly
  bypasses function lookup for the Cargo command.
- Host inspection and CLI subprocesses now use a fixed root-owned executable
  search path, while the privileged broker invokes udevadm through an absolute
  NixOS system path.
- Process-marker ratchet failures now identify the gate and require stale
  exemptions to move from `activePaths` to `retiredPaths`, while explicitly
  rejecting any change to the frozen path universe.
- Rust tests that launch Bash or POSIX shell fixtures now route through the
  inherited-function scrubber, including direct Cargo test invocation.
- The manifest-driven Layer-1 local and CI graphs now run the dedicated
  nix-unit corpus target and require it in the CI rollup.
