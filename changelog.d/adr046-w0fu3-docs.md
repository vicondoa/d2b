### Security

- ADR 0046 ZoneLink: specified the crash-safe
  `Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready` session state
  machine on the owning ZoneLink-handler work item and every mirror, replacing
  the prior generic `Pending/Established/Disconnected/Reconnecting/Revoked`
  model that had no enrollment or PSK-consumption semantics. Resource traffic is
  now prohibited before the enrolled KK session reaches Ready; each PSK consume,
  enrollment persist, and bootstrap teardown crash window has a defined recovery;
  and revocation invalidates both the sealed enrollment record and the active
  session, requiring a fresh single-use PSK and a new IKpsk2 enrollment before
  reconnect. An implementer can no longer retain a revoked enrollment or bypass
  bootstrap evidence by following the stale state machine.
- ADR 0046 threat model: replaced the `spec.childStaticKeyFingerprint`
  reconnect trust anchor, which contradicted the six-field ZoneLink schema, with
  the private sealed enrollment-record and child key-pin authority bound to the
  child Zone uid and the allocator enrollment, and updated the detection and
  reconnect-validation text to match. The schema no longer implies an illegal
  seventh field or an omitted key-pin check.

### Fixed

- ADR 0046 spec set: completed the universal-status sweep across every complete
  resource envelope, adding the mandatory `status.update` (D091) currency object
  and nesting type-specific fields under `status.resource` (D107) wherever a
  complete envelope still omitted them, including the Credential example that had
  used `status.credential` instead of `status.resource`. The corresponding
  claim in the earlier ADR046-W0fu2 changelog fragment, which asserted the sweep
  was already complete, has been corrected.
- ADR 0046 Host/Guest execution policy: added a valid `defaultUserRef` to every
  mixed-domain example that still omitted it or set it null (D116), retaining the
  omission only in explicitly labelled rejection fixtures, so the superset
  invariant holds across the primitive-resource-composition, system-core, and
  credential-entra specs and the remaining mixed-domain examples.
- ADR 0046 current-state prose: corrected the remaining current-source rows that
  still asserted the spec registry, implementation graph, and execution-time
  ledger do not exist, distinguishing the tooling that has landed and is wired
  through `xtask`, the drift gate, and Layer 1 from the binding, wiring, and
  hardening work that remains.
- ADR 0046 delivery docs: corrected the `MergeTarget` artifact example, which
  used pull-request number `0` (rejected by the implementation), to a positive
  number and stated the nonzero constraint explicitly.
- Contributor docs: corrected the spec-literal lint allowlist guidance in
  `AGENTS.md`. There is no inline `d2b-lint-allow` marker; the lint rejects that
  escape hatch, and the sole exemption is the decision-register row that defines
  a rule in `docs/specs/ADR-046-decision-register.md`.

### Changed

- Contributor docs: aligned the test quick-start with shipped behaviour.
  `tests/README.md` no longer claims that invoking a live script directly
  bypasses the heavy-gate semaphore, matching `tests/AGENTS.md` and the scripts'
  self-re-exec self-guard. `AGENTS.md` and `tests/README.md` now document the
  `test-runtime-ledger` Layer-1 job as an absolute per-test and per-crate
  execution-budget gate with no baseline and no regeneration workflow, noting
  that the `Makefile` and `tests/layer1-jobs.json` are authoritative if the
  prose diverges.
