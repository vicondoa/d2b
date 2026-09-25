### Fixed

- Repaired three tests that could not fail - the daemon readiness pair, the bus telemetry closed-label check, and the Wayland display registry handlers - so each one now fails when the behaviour it names breaks, and dropped a redundant revision-display assertion that could never fail either way.
- Malformed wire input no longer panics the broker, the daemon runtime, or the Wayland policy engine: a malformed authoritative audit join, a malformed broker zone digest, and a malformed driver zone token now return typed refusals at those boundaries.
- Moved blocking reaping, file locking, ACL application, and NSS group lookup off the broker's async executor workers, so a busy executor no longer stalls on host syscalls.
