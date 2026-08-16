### Changed

- The ADR 0046 spec-literal drift lints now exempt a rejected literal only on
  the decision-register row that defines the rule it violates. The previous
  generic inline allow marker, which could suppress a real violation from any
  line in any spec file, is gone, so a lint can no longer be silently defeated
  where it matters most.
- The changelog gate now classifies patch and protocol-definition files
  (`.patch`, `.proto`) and every other unrecognized extension as code by
  default, exempting only an explicit prose and data allowlist. A patch-only or
  protocol-only change, including a deletion, can no longer ship without a
  release note.

### Fixed

- The ADR 0046 datetime lint now validates that a millisecond-precision
  timestamp names a real instant, rejecting impossible calendar dates (for
  example month 13, day 31 of a 30-day month, or February 29 of a non-leap
  year) and leap seconds (`:60`), not merely the `YYYY-MM-DDTHH:MM:SS.sssZ`
  shape.
- The ADR 0046 ResourceType lint now enforces the frozen qualified grammar in
  full: a qualified token must be `<provider>.d2bus.org.<Type>` where the
  provider matches `^[a-z][a-z0-9-]*$` within 63 bytes and the type matches
  `^[A-Z][A-Za-z0-9]{0,62}$`, and an unqualified token must be one of the
  standard catalog names. A token missing the provider segment, carrying an
  extra segment, using a lowercase type, or exceeding the byte bounds is now
  rejected instead of accepted.
- The ADR 0046 retry-scalar lint now verifies that the frozen millisecond value
  is an integer, rejecting a quoted-string or floating-point value that the
  earlier key-and-shape check accepted.
- The changelog fold is now recoverable across an abrupt interruption. It keeps
  a durable transaction journal and a byte backup of the previous changelog,
  fsyncs each state transition, and preserves the original changelog until the
  promotion rename succeeds. A later run detects an interrupted fold and either
  finishes a committed one or rolls an uncommitted one all the way back, so a
  crash can never leave a half-consumed fragment set or a changelog missing the
  entries whose fragments were already removed.
