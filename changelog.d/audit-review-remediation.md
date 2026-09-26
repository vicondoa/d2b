### Fixed

- A mutating request that sets neither `dryRun` nor `apply` decodes again
  instead of failing admission, so the daemon refuses it through the same
  structured `InvalidRequest` outcome and remediation string every other
  invalid mutating request gets, rather than an opaque frame error.
- A persisted Azure VM recovery record whose legacy operation pair is only
  half present now fails its decode with a typed error instead of aborting the
  thread, and the restore documentation no longer promises a predicate that
  the collapsed pair no longer needs.
- The `StatusServicesOutputV2` wire shape is pinned again, including the
  asymmetry where only `qemuMedia` is omitted when absent while the four
  sidecar fields serialize an explicit `null`.

### Changed

- Two `shared-family-knowledge` ratchet exemptions that named tokens the
  target module does not contain are removed; the list only ever shrinks.
- Seven changelog fragments no longer open with a filename title line the
  fragment parsers reject, and internal audit identifiers are out of the
  release prose.
