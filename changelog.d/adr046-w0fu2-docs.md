### Changed

- ADR 0046 spec set: advanced the universal-status sweep so more Accepted
  resource examples serialize the universal envelope with its `status.resource`
  (D107) and `status.update` (D091) currency object, including the flat-status
  shell-pool and shell-session examples that the first pass left populated but
  un-nested. This pass did not reach every complete envelope; the residual
  complete envelopes still missing `status.update` are swept in the ADR046-W0fu3
  pass.
- ADR 0046 ZoneLink profile: aligned every normative handshake statement across
  the zone-routing, Unix transport, and Azure Relay Provider specs so a ZoneLink
  consumes the allocator-issued single-use PSK exactly once under IKpsk2, persists
  the enrolled static identity, tears down or rekeys the bootstrap session, and
  only then establishes the enrolled KK session before Ready or resource traffic.
  Enrolled steady-state and credential-acquisition KK sessions are unchanged.
- ADR 0046 Host execution policy: the mixed-domain Host example now supplies the
  `defaultUserRef` its compiled output requires, and a companion rejection example
  shows the missing-`defaultUserRef` shape that D116 fails closed at Nix eval, so
  the superset invariant is illustrated from both sides.

### Fixed

- ADR 0046 current-state prose: corrected the remaining current-source, evidence,
  and delta rows that still asserted no spec parser, generated graph, delivery
  machinery, or Make targets exist, distinguishing the tooling that has landed
  from the surfaces that remain to be hardened.

### Added

- ADR 0046 delivery docs: documented the `merge-target` capture step and the
  `MergeTarget` artifact schema, so operators can see how a sealed candidate is
  bound to its pull requests and which check conclusions permit a merge.
- Contributor docs: documented the ADR 0046 spec-literal lint allowlist. The
  sole exemption is the decision-register row that defines a rule in
  `docs/specs/ADR-046-decision-register.md`; there is no inline
  `d2b-lint-allow` marker, and the lint rejects that escape hatch by design.

### Security

- ADR 0046 ZoneLink bootstrap: closed the documentation gap that let an
  implementer read ZoneLink sessions as all-KK and skip IKpsk2 enrollment, so the
  initial cross-Zone handshake and its `bootstrap-ikpsk2` evidence can no longer be
  bypassed by following a stale spec statement.
