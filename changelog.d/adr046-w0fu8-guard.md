### Fixed

- The directly invokable heavy-gate self-guard now removes inherited Cargo and
  Rust compiler shell functions before building its verifier, and explicitly
  bypasses function lookup for the Cargo command.
