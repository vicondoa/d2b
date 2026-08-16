### Added

- Added the authenticated transport-vsock Provider with replay-safe Guest and
  Zone session admission, bounded framing, named-stream bridging, and native
  guest relay lifecycle.
- Added allocator-only transport settings and integration coverage for CID
  authority, restart matching, close ordering, redaction, and attachment
  refusal.

### Fixed

- Fixed relay finalization, transport close budgeting, degraded observation
  retention, and end-to-end open deadlines.
