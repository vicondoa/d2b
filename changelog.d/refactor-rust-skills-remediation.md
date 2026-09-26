### Fixed

- Repaired three tests that could not fail - the daemon readiness pair, the bus telemetry closed-label check, and the Wayland display registry handlers - so each one now fails when the behaviour it names breaks, and dropped a redundant revision-display assertion that could never fail either way.
- Malformed wire input no longer panics the broker, the daemon runtime, or the Wayland policy engine: a malformed authoritative audit join, a malformed broker zone digest, and a malformed driver zone token now return typed refusals at those boundaries.
- Moved blocking reaping, file locking, ACL application, and NSS group lookup off the broker's async executor workers, so a busy executor no longer stalls on host syscalls.
- The published v2 storage lifecycle report schema now names each issue field the way the daemon writes it, so a consumer validating a real report no longer rejects the five issue variants that carry a renamed field.
- The daemon API reference now documents an audit response page as the wire carries it - entries, the continuing cursor, and completion - rather than as the daemon holds it in memory.
- The blocking-API baseline row for the device Provider's `block_on` count is real and stays; its recorded call site named a line inside the test, and the census counts the async test attribute that expands to the call.

### Changed

- Documented the workspace's Rust library surface: crate and module docs, one-line summaries, `# Errors`/`# Panics`/`# Safety` sections, and runnable examples on the items that stated no contract, across the CLI, the bus, the daemon runtime, the broker, the resource runtime, and `xtask`.
- Simplified expression-level code: iterator pipelines over index loops, derived `Default` and `Debug` implementations over hand-written ones, `From`/`FromStr` conversions over ad-hoc parsers, and shared helpers where the same scaffold was copied.
- Removed clones, copies, and dead rebindings that ownership did not require, keeping the same sharing, locking, and borrow boundaries.
