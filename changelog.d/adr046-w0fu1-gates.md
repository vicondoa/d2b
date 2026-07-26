### Added

- Three ADR 0046 spec-literal drift lints run as fixture-independent
  contract-test policy binaries in the mandatory policy gate: datetimes under
  `docs/specs/**` must be exactly millisecond-precision RFC 3339
  (`YYYY-MM-DDTHH:MM:SS.sssZ`), qualified ResourceType tokens must use the
  `.d2bus.org.` infix, and the retry delay must be the integer `retryAfterMs`
  scalar rather than a superseded duration-string form. Each rule enforces a
  frozen decision that a hand enumeration had previously miscounted, and the
  only exemption is the exact decision-register row that defines the rule.

### Changed

- The policy meta-gate now executes the fixture-independent contract-test
  policy binaries (the dash ban, the ADR 0046 manifest bijection, and the
  changelog gate) directly and fails closed if any of them is skipped, filtered
  to nothing, or reports zero tests. These binaries are excluded from the
  workspace test run, so the policy gate is now their guaranteed execution
  point in CI.
- The changelog gate now classifies deletions and every executable and
  configuration surface (shell, Makefile, workflow, and data manifests) as a
  code change, so a removed module or a shell-only behaviour change can no
  longer ship without a release note.

### Fixed

- The tier-0 dash scan no longer reports success when the scan itself errors
  (unreadable file, vanished file, bad pattern) or when file enumeration fails;
  it now distinguishes "no matches" from a scan error and fails closed on the
  latter.
- The ADR 0046 manifest policy gate now compares every serialized work-item
  field, including `dependencyOwner`, `destination`, `detailedDesign`,
  `validation`, `reuseAction`, and `reuseSource`, against its Markdown
  declaration, so a generator regression that alters a field value can no
  longer regenerate cleanly and pass.
- The changelog fold now stages the rewritten changelog on the same filesystem,
  promotes it atomically, and reserves fragments before promotion so a failed
  write or deletion leaves `CHANGELOG.md` byte-unchanged with every fragment
  intact instead of a corrupted or partially consumed changelog.
