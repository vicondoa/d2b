### Fixed

- The directly invokable heavy-gate self-guard now removes inherited Cargo and
  Rust compiler shell functions before building its verifier, and explicitly
  bypasses function lookup for the Cargo command.
- Host inspection and CLI subprocesses now use a fixed root-owned executable
  search path, while the privileged broker invokes udevadm through an absolute
  NixOS system path.
