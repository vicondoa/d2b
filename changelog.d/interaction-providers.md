### Added

- Added Wayland display, desktop notification, and Wayland clipboard Providers
  with bounded lifecycle state, explicit policy and RBAC, redacted audit and
  telemetry output, authenticated stream admission, and hermetic tests.
- Carry sealed typed clipboard and notification configuration through the Zone
  resource runtime, issue supervisor-authoritative notification source and
  host-sink receipts, and keep short AF_UNIX telemetry test sockets faithful.
- Hardened nonce/idempotency cleanup, display principal lifecycle reuse, and
  clipboard rate-bucket garbage collection.

### Fixed

- Keep notification source and host-sink lifecycle tests in the Provider crate
  so supervisor packaging stays on the closed effect-port allowlist.
