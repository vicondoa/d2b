### Changed

- The ADR 0046 envelope and spec-literal lints now parse fenced YAML, JSON, and
  Nix blocks into a structural document model and assert over the parsed tree,
  instead of matching line shapes. Line-oriented heuristics were the root cause
  of a family of fail-open lints that matched the examples their author happened
  to look at rather than enforcing the format they claimed to police; a block
  the parser cannot model now fails closed rather than being silently skipped.
- The universal-status lint now checks resource envelopes written as JSON as
  well as YAML, treats a document that is missing a frame key as a candidate to
  check rather than one to skip, honours a `...` elision only as a direct child
  of `status` (not anywhere in the status subtree), and rejects an inline
  `status: {}` or `status: null` on a live envelope. Compiler-emitted bundle
  envelopes, which carry `resourceType` and an explicit `status: null`, are
  recognised as a distinct contract and are not required to carry a status base.
- The D116 Host/Guest lint now reads a multiline `allowedDomains` list the same
  as an inline one, evaluates each document in a fence independently so one
  `defaultUserRef` cannot satisfy a different document, ignores a commented-out
  `defaultUserRef`, and pins the intentional-negative-example exemption to the
  exact file and the single unique marker that needs it, so the marker fails
  closed anywhere else.
- The D103 datetime lint now validates the complete alphanumeric-delimited token
  rather than a conformant prefix, so a valid instant with trailing or leading
  junk such as `2026-07-22T00:00:00.000Zjunk` is rejected. The D104 ResourceType
  lint now validates the `type` and `resourceType` fields of a quoted JSON
  envelope and an indented Nix resource declaration, not only a bare top-level
  YAML `type:`. The D108 retry-scalar lint now fails closed on any value that is
  not a bare decimal in range, rejecting tokens such as `1e3`, `banana`, and
  `nonsense` that the previous fall-through silently accepted, while still
  exempting a Rust type annotation and a `<placeholder>`.

### Fixed

- Aligned the Nix resource-shape template in `ADR-046-resources-volume.md` to the
  `type = "<ResourceType>"` placeholder convention used by every sibling spec,
  in place of a bare `type = "ResourceType"` that read as a concrete but invalid
  ResourceType name.
- Completed the universal status base on the `type: Host` envelope in
  `ADR-046-telemetry-audit-and-support.md`, which showed an isolation-posture
  status subtree without the `status.update` and `status.resource` base the
  universal-status contract requires.
