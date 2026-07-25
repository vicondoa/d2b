### Added

- Two ADR 0046 envelope-structure lints now close resource-shape classes that
  manual review kept missing. The first requires every Host or Guest example
  whose `allowedDomains` admits the `user` domain to carry a non-null
  `defaultUserRef`. The second requires every complete resource envelope - one
  that declares `apiVersion`, `type`, `metadata`, `spec`, and `status` - to
  carry the universal status base, including both `status.update` and
  `status.resource`. Both lints judge only complete envelopes: a focused
  fragment, a shorthand schema table, and a status body deliberately elided
  with `...` are exempt, so the lints enforce the contract without flagging
  illustrative snippets. The universal-status lint reads only fenced YAML
  documents, so explanatory prose that references a field path such as
  `Credential.status.credential.expiresAtUnixMs` under the documented
  `status.<field>` mapping convention is never flagged. The Host/Guest lint
  additionally exempts an intentional negative example - a shape authored to be
  rejected - when it carries the greppable marker comment `d2b-lint:
  expect-d116-...`, so a teaching block that deliberately omits `defaultUserRef`
  to demonstrate the eval-time failure is not mistaken for a real declaration.

### Changed

- The ADR 0046 ResourceType and retry-scalar lints now run the repository
  scanner through the same exact validators as their unit tests instead of a
  looser set of regex substrings. The type lint extracts complete tokens from
  authoring contexts and rejects an unknown unqualified name such as
  `type: Widget`, a malformed qualified token such as `acme.d2bus.org.1Widget`
  or `acme.d2bus.org.Widget_Type`, and any token whose grammar the scanner's
  older reject set admitted but Nix would refuse. The retry-scalar lint now
  accepts only a bare decimal integer inside the frozen range, rejecting `0`,
  an out-of-range value, and non-integer values such as `true`, `null`, and
  `-1`.
- The ADR 0046 datetime lint now extracts the value from timestamp-bearing
  authoring fields and validates any value that presents as a date, catching a
  malformed instant that falls outside the lint's earlier candidate shape.
- The ADR 0046 spec-literal exemption is now bound to the exact canonical file
  and a uniquely parsed decision-register row, rather than a filename suffix
  plus a row prefix, so it can no longer be satisfied by a lookalike file or a
  non-canonical row.
- The runtime execution-budget ledger is now an honest absolute budget gate. It
  records genuine repeated execution-only samples, recomputes p95 from those
  samples, audits that every scope is measured on every repetition, requires the
  crate census to reproduce a pinned closed set exactly, and runs the
  hermetic placement lint over the census crates' integration tests. It no
  longer holds a synthetic baseline or claims historical-regression detection;
  the gate now runs as part of the local pre-merge check as well as in CI.
  Growing the census to a real multi-crate shard inventory and adding a genuine
  cross-machine reference baseline for a true historical-regression gate is
  tracked as the deferred follow-up
  `runtime-ledger-full-census-and-real-shards`.

### Fixed

- The changelog fold's committed-transaction cleanup is now restart-safe. It
  removes the committed payload in journal-last, fsynced order so the
  `COMMITTED` marker is the last thing cleared, instead of an unordered
  recursive delete that could drop the marker while a restorable backup and the
  reserved fragments survived and cause a later recovery to roll a promoted fold
  back - duplicating or losing entries. Inline recovery failures are now folded
  into the surfaced error rather than discarded, and recovery is proven
  idempotent under interruption after promotion and throughout both forward and
  rollback recovery, so repeated recovery always leaves the changelog folded
  exactly once or fully rolled back.
