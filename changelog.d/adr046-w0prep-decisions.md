### Added

- Recorded twenty ADR 0046 foundation decisions (D099 to D118) in the decision
  register, closing the implementation-level contracts the first implementation
  stage needs before any of its four serialized slices can open.
- Froze the resource-plane byte formats that become permanent the moment any
  data exists: the `d2bkey/v1` store key codec, the `d2bval/v1` value frame, the
  `d2b-cjson/v1` canonical JSON profile with its domain-separated SHA-256
  digests, the resource protobuf representation and field-number policy, the
  UUIDv4 resource UID spelling, the fixed-precision UTC timestamp spelling, and
  the ResourceType segment grammars and byte bounds.
- Froze the security-critical resource-plane contracts: the literal
  bootstrap-authorization allow table, its two-phase derivation from durable
  store state, and its one-way end condition; and the store commit boundary,
  which admits only a pre-authorized mutation carrying a policy snapshot the
  write transaction rechecks without duplicating any authorization logic.
- Froze the resource API surface: the always-present common status layer, the
  outcome scalar encodings, the authenticated subject context component types,
  the service name and code-generation ownership, the v3 error model, the
  request/list/watch/batch admission bounds, the revision-log compaction
  defaults, and the owner, finalizer, label, annotation, and reference bounds.
### Changed

- Replaced the resource API spec's `## Limits` section, which asserted that
  bounds were frozen but listed only the axes with no values, with the frozen
  numeric tables and the derivation anchor for each value. Over-limit input now
  has a defined rejection class, so admission control is implementable and the
  section no longer claims a property it did not have.
- Closed the resource API spec's `## Errors` class set at exactly 31 classes.
  It previously read as an open list, which left the wire enum unbounded, and
  gave the bounded error `reason` an explicit 512-byte ceiling and redaction
  rule.
