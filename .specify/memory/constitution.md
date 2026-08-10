<!--
Sync Impact Report
==================
Version change: 3.0.0 -> 3.1.0
Rationale: MINOR. Principle VI gains a narrow, one-time historical
disposition for the already-merged ADR-046 delivery history while preserving
every prospective panel, validation, delivery, and merge gate.

Amendment 3.1.0 (2026-08-09): ADR-046 historical gate disposition
- Principle VI expressly dispositions the missing W0/W1 panel receipts and
  seals and the unproven contemporaneous W2-W5 plan panels through the exact
  merged Wave 5 boundary at `177235ed37188b3be87525e7f016fb43401574c5`.
- The disposition also closes the retained Wave 5 candidate
  `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`,
  snapshot `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`,
  head `19b77dad63060bcadd41f1ef800978d2c53cc030`, and retained
  `panel-request.json` SHA-256
  `15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7`
  as immutable historical state with one consumed request, zero attestations,
  and no seal.
- For ADR-046 W6 predecessor checks only, merged Wave 5 boundary
  `177235ed37188b3be87525e7f016fb43401574c5` is the one-time historical
  predecessor disposition. It is not a reconstructed seal and authorizes no
  Wave 5 recovery or second binding request.
- Every post-boundary ADR-046 action requires this amendment's merge commit to
  be an ancestor of the exact execution base. W6 additionally requires the
  ordinary exact-base unanimous T221 entry plan panel and every remaining
  prospective gate.
- Rationale: missing historical evidence cannot be recreated truthfully. A
  closed, commit-bound disposition preserves that fact while restoring strict
  forward enforcement.

Amendment 3.0.0 (2026-08-08): Selected-roster Discover-Fix-Verify
- Principle VI replaces the operative fixed roster with the versioned
  candidate-bound selection artifact and current thirteen-seat role domain.
- One comprehensive discovery, a shared stable ledger, batched implementation
  responses and self-verification, and scoped verification are now binding.
- Legacy fixed-ten delivery records remain readable as compatibility data; they
  do not define current selection.
- The active bounded-deferral-after-eight-rounds rule is superseded. During
  verification, pre-existing MINOR and NIT observations are non-blocking from
  the start, while admitted BLOCKER and MAJOR findings remain blocking.
- Rationale: selection and a complete shared ledger prevent repeated
  rediscovery while preserving unanimous, request-bound sign-off.

Amendment 2.2.0 (2026-08-06): Approved contract-defect correction
- Principle IV now permits one approved correction to amend the governing plan,
  specification, or contract while moving the affected contract, schema,
  reference documentation, emitter, consumer, generated artifacts, and tests
  together.
- Approval must precede implementation and bind the defect, exact governed
  surfaces, version impact, and candidate snapshot. A vague intent to "fix
  drift" is not approval.
- The correction still requires every applicable version bump and drift gate.
  It cannot be used to land a contract-only change, defer a paired artifact, or
  treat stale prose as authority over committed passing code.
- Rationale: an accepted artifact can itself contain a defect. Requiring a
  second artificial sequencing step after the defect and coordinated repair
  are already approved adds delay without improving contract consistency.

Amendment 2.1.0 (2026-07-29): Bounded deferral and delivery memory
- Principle VI gains a "Bounded deferral after eight rounds" clause: from round
  nine onward a reviewer MAY defer a LOW or MEDIUM finding instead of blocking.
  CRITICAL and HIGH remain non-deferrable in every round.
- MINOR, not MAJOR: the sign-off invariant is untouched. `signoff` is still true
  iff `recommendations` is empty, because a deferred finding is MOVED OUT of
  `recommendations` into a register rather than left there alongside a true
  sign-off. This deliberately avoids changing the enforced consistency check in
  packages/xtask/src/delivery/panel.rs, which rejects that combination in both
  directions.
- Principle VI also gains a "Delivery memory" clause requiring two durable
  registers: a deferred-findings register and a friction log, both restricted to
  classification metadata so no panel transcript enters Git.
- Rationale: one wave in this project ran twenty-one follow-up rounds. Unbounded
  LOW/MEDIUM churn can cost more than the defects it removes.

Amendment 2.0.0 (2026-07-29): Pipelined dispatch
- Principle VI gains a "Pipelined dispatch" clause permitting the next phase's
  implementation to begin after five roster reviews return and integration tests
  pass, while requiring that the next phase issue no panel request, no seal, and
  no merge until the current phase is sealed at full unanimity and merged, and
  that it rebase onto the updated integration lineage before its own panel.
- The unanimity requirement, the roster, the seal ordering, and the merge
  ordering are unchanged. The gate moved; it did not loosen.
- Rationale: panel review commonly runs one to two times the coding duration, so
  strict serialization idles implementation capacity for more than half of each
  cycle.
- Accepted cost recorded in the principle text: rework, when a finding changes a
  contract that in-flight next-phase work already consumed.

Modified principles:
- IV. Contract-Driven Compatibility (expanded in 2.2.0)
- VI. Panel-Gated Multi-Phase Work (redefined in 3.0.0; expanded in 3.1.0)
Added sections: none
Removed sections: none

Templates and artifacts requiring follow-up:
- specs/001-adr046-d2b3-completion/ - after this amendment lands on `v3`,
  rerun the exact-base W6 entry plan panel required by T221. The feature
  artifacts remain evidence of the formerly open prerequisite and are not
  rewritten by this constitution-only amendment.
- packages/xtask/src/delivery/ and its existing unit-test surface - DISCHARGED
  in this amendment by the exact one-time Wave 5 predecessor validator and its
  positive and planted-negative coverage.
- changelog.d/ - DISCHARGED by the marker-free amendment fragment shipped with
  this change.
- specs/001-adr046-d2b3-completion/spec.md - FR-025 and FR-036 restated for the
  pipeline; new FRs added for the strict panel/seal/merge ordering. DISCHARGED:
  FR-056 through FR-059 landed, FR-025 was narrowed to exit, and FR-057 states
  the entry-versus-exit distinction that reconciles FR-025 with FR-036.
- docs/specs/ADR-046-validation-and-delivery.md - Section 4 entry criteria said
  "there is no partial-wave advance" and the tooling enforced it. DISCHARGED:
  sections 4, 12.1 and 12.4 were amended to permit a pipelined start under the
  four Principle VI conditions, and the tooling moved the prior-wave-merged
  assertion out of wave entry to the panel-request, seal and merge-eligibility
  boundary. The pipeline is executable.

Deferred / TODO items: none.
-->

<!--
Prior Sync Impact Report (1.0.0)
================================
Version change: TEMPLATE (unratified) -> 1.0.0
Rationale: Initial ratification. First concrete constitution for the project
(previous file was the unfilled placeholder template), seeded at 1.0.0 per
semantic versioning guidance for first stable adoption of a governance document.

Modified principles: n/a (initial fill; no prior named principles existed)

Added sections:
- Core Principles I-VII (Daemon-Only Control Plane, Broker-Mediated
  Privilege, Reasonable Isolation Over Convenience, Contract-Driven
  Compatibility, Test-Layer Discipline, Panel-Gated Multi-Phase Work
  [now names the selected-roster lifecycle; prior reports retain history],
  Traceable,
  Marker-Free Shipped Artifacts)
- Additional Constraints (security posture, disk hygiene, naming/versioning)
- Development Workflow & Quality Gates
- Governance

Removed sections: none (template placeholders only)

Deferred / TODO items:
- TODO(RATIFICATION_DATE): No prior ratified constitution or founding date
  exists in repo history; using the date this document was first written
  as both ratification and last-amended date. Replace if an earlier
  founding date is authoritative.
-->


# d2b Constitution

## Core Principles

### I. Daemon-Only Control Plane (NON-NEGOTIABLE)
The framework declares **exactly three** persistent root-visible surfaces:
`d2bd.service`, `d2b-priv-broker.socket`, and `d2b-priv-broker.service`.
There MUST be no framework-declared per-VM systemd unit, no host-singleton
framework service, and no legacy bash CLI fallback. Every per-VM lifecycle
step MUST run inside `d2bd`'s DAG executor; spawned runners (Cloud
Hypervisor, virtiofsd, swtpm, vhost-user-sound, USBIP attach) are launched
by the broker and handed back to `d2bd` as pidfds. A restart of `d2bd` is a
continuation event, not a teardown event: it MUST re-adopt live VMs rather
than kill them.
Rationale: a single supervised control plane is the only way the daemon-only
architecture (ADR 0015) can reason about drift, restart safety, and audit
completeness. Reintroducing per-VM units or singleton services silently
reopens the exact sprawl the clean break eliminated.

### II. Broker-Mediated, Audited Privilege
Every host mutation that requires elevated privilege (cgroup delegation,
TAP/bridge lifecycle, nftables, sysctl, `/etc/hosts`, NetworkManager
unmanaged config, modprobe, USBIP firewall rules, runner spawn, pidfd
handoff) MUST flow through a typed `d2b-priv-broker` op and be recorded as
an audited `OpAuditRecord`. `SO_PEERCRED` + membership in the `d2b` group at
`public.sock` accept time is the **only** local lifecycle authorization
surface; there is no polkit allowlist. Relay/session credentials from a
realm gateway are never mapped to local admin authority. New host-mutable
paths, locks, or ACL surfaces MUST reuse the existing storage/lock
ownership contract (opaque broker-resolved ids, OFD locks, single repair
owner) rather than inventing an ad-hoc chmod/chown/setfacl path.
Rationale: the broker is the only place host-mutating side effects are
authorized and logged; bypassing it removes both the audit trail and the
narrow, typed set of operations the threat model depends on.

### III. Reasonable Isolation Over Convenience
Per-VM boundaries (the `/nix/store` hardlink farm, TPM NVRAM persistence,
USBIP passthrough scoping, GPU/video sidecar principals, virtiofsd
zero-host-capability sandboxing) exist to keep untrusted workloads away
from the host and from each other. Changes MUST NOT weaken an isolation
boundary for convenience: no serving the full host `/nix/store` to a guest,
no casual wipe of swtpm state, no host capabilities added to a virtiofsd
profile, no cross-realm gateway or bridge sharing. Where a boundary must be
relaxed, it MUST be an explicit, reviewed, default-off opt-in with its own
principal/ACL scope, not a silent default-on convenience.
Rationale: d2b's entire value proposition is "reasonable isolation for a
single-user desktop." Every convenience shortcut here is a direct trade
against the threat model the project exists to serve.

### IV. Contract-Driven Compatibility
The manifest schema, bundle artifacts, CLI wire contract, and broker op
catalogue are versioned contracts (`manifestVersion`, `bundleVersion`,
`schemaVersion`). Adding, removing, or renaming a field or op MUST bump the
relevant version, update the paired schema/doc, and land in the same
change as the emitter/consumer edit. Drift gates (`xtask gen-*` +
`git diff --exit-code`) are the enforcement mechanism and MUST NOT be
disabled or worked around to land an out-of-sync artifact.
An approved defect in a plan, specification, or contract MAY be corrected in
the same coordinated change as the affected contract implementation. Approval
MUST be recorded before implementation and MUST identify the defect, the exact
governed surfaces, the required version impact, and the candidate snapshot.
That change MUST move every affected contract definition, schema, reference
document or governing specification, emitter, consumer, generated artifact,
and test together. It MUST run the normal drift gates and panel review. This
path does not permit a contract-only edit, a deferred paired artifact, a
missing version bump, or a success-shaped compatibility fallback.

Rationale: downstream consumers (host configs, sibling flakes, the broker,
the daemon) depend on these contracts being exact; unversioned drift breaks
them invisibly.

### V. Test-Layer Discipline
New test coverage MUST land at the lowest layer that can hermetically prove
it: nix-unit eval case, Rust unit test, Rust binary/integration test,
rendered-artifact contract test, or policy lint - in that preference order
- before reaching for a Layer-2 container/VM/live-host/hardware tier. The
Layer-1 drift and meta gates are a closed set; do not add a new top-level
`tests/*.sh` gate. Every Layer-2 and heavy command MUST run through the
`cargo xtask heavy-gate` semaphore, never as a raw script. A test's
enforcing-vs-advisory classification comes from `tests/layer1-jobs.json`,
read fresh each time, not assumed from prior knowledge.
Rationale: this ordering is what keeps the suite fast and maintainable;
routing everything through the semaphore is what keeps concurrent heavy
lanes from oversubscribing the shared Nix store, cargo target, and KVM
device.

### VI. Panel-Gated Multi-Phase Work
A multi-phase plan MUST pass a panel sign-off gate at each phase boundary:
unanimous sign-off (`recommendations: []`) from the selected lifecycle roster,
following plan review → implementation → integration → work review → advance.
The current role domain is the thirteen-seat selection table (`software`,
`test`, `product`, `docs`, `security`, `observability`, `simplicity`,
`reliability`, `agentic`, `nixos`, `networking`, `kernel`, `build`). A
versioned candidate-bound selection artifact chooses the ordered roster,
includes mandatory and triggered seats, meets the applicable floor, and may
only widen over fix deltas. Rust depth is a `software` profile; legacy
delivery artifacts retain the historical `rust` seat. Green tests alone
never waive this gate. Trivial fixes, time-critical hotfixes (with a
mandatory post-fix panel), and documentation-only changes that describe no
load-bearing behavior are the only exceptions. Where a harness stands in for
the per-round panel, it MUST preserve the same unanimity rule and no-rerun
discipline; it does not substitute for the separate, binding selected-roster
panel required once at a wave's close for wave-scale (ADR 0046-class) work.

**Discover-Fix-Verify.** A lifecycle MUST run one comprehensive discovery over
the full candidate, require an explicit complete result from every selected
seat, merge findings into one shared ledger with stable identifiers, and give
the complete ledger to implementation for batched dispositions, evidence, and
self-verification. It MUST run scoped verification against that ledger,
responses, supplied validation, the latest delta, and the full candidate.
Pre-existing late MINOR and NIT observations do not become new blockers.
Metrics are informational and never decide approval.

**Pipelined dispatch.** Implementation of the next phase MAY begin before the
current phase's panel returns unanimous sign-off, provided **all** of the
following hold:

1. At least five of the current phase's roster reviews have returned, and
2. the current phase's integration tests pass on its converged tree, and
3. the next phase issues **no panel request, no seal, and no merge** until the
   current phase is sealed at full unanimity and merged, and
4. the next phase rebases onto the updated integration lineage after that
   merge and before its own panel runs.

This permits an earlier start, never a weaker gate. Every phase still closes
only on unanimous sign-off with zero outstanding recommendations, and phases
still seal and merge in strict order. The accepted cost is **rework**: a
finding that changes a contract may invalidate in-flight next-phase work, and
absorbing that rework is the price of the pipeline. A plan using pipelined
dispatch MUST record that acceptance explicitly.

**Verification convergence.** Verification checks the shared ledger,
implementation responses, supplied validation, regressions, and admitted late
BLOCKER or MAJOR findings. A pre-existing MINOR or NIT observation discovered
after the comprehensive discovery remains non-blocking history. There is no
round-count threshold and no later transition from blocking to non-blocking.

**ADR-046 historical disposition.** The delivery history for
`specs/001-adr046-d2b3-completion` through merged `v3` commit
`177235ed37188b3be87525e7f016fb43401574c5` contains three nonconforming
evidence classes:

1. W0/W1 lack the panel receipts and seals required by this principle.
2. Contemporaneous W2-W5 plan-panel evidence is unproven.
3. Wave 5 retained candidate
   `d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`,
   snapshot `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`,
   head `19b77dad63060bcadd41f1ef800978d2c53cc030`, and retained
   `panel-request.json` SHA-256
   `15f49657490410f0fb5530513144c7c2392f567b211eb630551f3110b94633f7`
   consumed its sole binding request with zero attestations and no seal before
   the later Wave 5 tree merged at
   `177235ed37188b3be87525e7f016fb43401574c5`.

Those facts are accepted as closed historical governance deviations for those
exact bytes and identifiers only. They MUST NOT be described as gates that
passed, reconstructed through retroactive attestations, replaced with current
reviews, or used as precedent for another program or wave. The retained
candidate, request, imported evidence, and missing attestation and seal state
remain immutable. No Wave 5 recovery action, replacement candidate, second
binding request, attestation, or reconstructed seal is authorized.

For ADR-046 W6 predecessor checks only, merged Wave 5 boundary
`177235ed37188b3be87525e7f016fb43401574c5` is the one-time historical
predecessor disposition. Delivery tooling MUST match the exact retained Wave 5
candidate, snapshot, head, consumed request, and merged boundary above and
MUST reject every partial, missing, additional, or mismatched state. This
treatment records a historical exception; it does not create a Wave 5 seal.

Before any post-boundary ADR-046 implementation, resume, fix, panel, seal,
merge, or advance action, the merge commit carrying this amendment MUST be an
ancestor of the exact execution base. W6 begins only after rebasing onto that
lineage and passing the ordinary T221 unanimous selected-roster entry plan
panel against the exact base and current feature snapshot with zero
recommendations. Every later implementation, validation, Discover-Fix-Verify,
binding request, attestation, protected PR merge, seal, and merge-eligibility
requirement remains unchanged.

Rationale: convergence comes from comprehensive discovery and scoped
verification, not fatigue after an arbitrary number of open-ended rounds. The
project's own history includes a panel catching 11 HIGH findings that
automated tests caught none of, so testing and review remain complementary.

### VII. Traceable, Marker-Free Shipped Artifacts
Shipped source, docs, CLI text, and CHANGELOG entries MUST NOT carry
internal process bookkeeping (wave/phase/follow-up/finding tags). Those
belong in planning artifacts, ADRs, commit messages, and PR descriptions
only. Every in-development commit on a feature branch MUST carry the
canonical trailing wave/phase tag form; every PR must ship release notes
(a CHANGELOG entry or a `changelog.d/` fragment) and MUST NOT attribute
authorship to an AI agent, assistant, or model. Dashes MUST be spelled with
the ASCII hyphen only, everywhere.
Rationale: shipped artifacts are read by consumers with no context on the
project's internal process; mixing bookkeeping into them degrades
readability and, per the project's own tooling, is mechanically enforced
because it has recurred before.

## Additional Constraints

- **Security posture**: no new linter/formatter/pre-commit hook without
  explicit request; no new `nixpkgs.overlays` entry or `nixpkgs.url` change
  without explicit request; no secrets, real hostnames, real user
  identifiers, or real network ranges committed - use generic placeholders
  and RFC1918/RFC5737 ranges. Screenshots and visual artifacts used as
  validation evidence MUST be redacted of secrets, PII, and sensitive
  output before being committed or attached to a PR.
- **Disk hygiene**: throwaway probes and one-off experiments go under the
  gitignored `.scratch/`, never beside production code. Test eval
  expressions MUST resolve the flake via `git+file://$ROOT`, never a bare
  `path:` fetch. Rust worktrees keep independent `packages/target/`
  directories; do not share a target directory across worktrees.
- **Naming invariants**: VM/workload names MUST match `^[a-z][a-z0-9-]*$`,
  reserve the `sys-` prefix and the exact name `launcher`, and any
  relaxation of this regex or the reserved set is prohibited.
- **Versioning**: the project follows Semantic Versioning and Keep a
  Changelog; the CHANGELOG is organized by version, and internal process
  markers are stripped from a section when a version is cut.

## Development Workflow & Quality Gates

- New behavior belongs in a focused module/file under `nixos-modules/` (or
  `nixos-modules/components/` for per-VM toggles) or the relevant
  `packages/<crate>/`; do not fatten an existing unrelated file.
- Before validating, commit changes: untracked files are invisible to
  `nix flake check` and equivalent evals, so an uncommitted new module
  silently fails to apply.
- `make check` is the PR-equivalent Layer-1 gate; `make test-integration`
  and `make test-host-integration` are local host/manual pre-PR tiers that
  are not run by the PR pipeline and MUST be run locally before opening an
  agent-owned PR.
- `main` is protected: changes land via pull request, validated locally
  against the gates above, reviewed, and squash-merged. Direct pushes to
  `main` are prohibited.
- When existing code, tests, and this constitution's referenced docs
  (README, AGENTS.md, ADRs) disagree, the passing, committed code is canon;
  document the drift rather than silently re-aligning code to prose.

## Governance

This constitution supersedes any conflicting informal practice. `AGENTS.md`
and `tests/AGENTS.md` remain the detailed operating manuals for day-to-day
mechanics (exact commands, panel rosters, commit-tag grammar, test
taxonomy); where they conflict with this document on a matter of
non-negotiable principle, this constitution controls, and the conflict MUST
be resolved by amending one of the two documents in the same change that
identifies it.

**Amendment procedure**: propose the change (what principle/section, what
text, why), classify the version bump (MAJOR for a backward-incompatible
principle removal/redefinition, MINOR for a new principle or materially
expanded guidance, PATCH for clarification/wording), update the Sync Impact
Report at the top of this file, and land the amendment as its own reviewed
change with an explicit rationale in the commit/PR description.

**Compliance review**: every PR that touches a governed surface (the seven
core principles above, or a row in AGENTS.md's "Critical subsystems" table)
MUST be checked against this constitution during review. A reviewer finding
a violation treats it the same as a failing test: block the PR until
resolved or the constitution is amended first.

**Versioning policy**: this document follows semantic versioning as
described in the amendment procedure above. The version, ratification date,
and last-amended date are recorded in the footer and MUST be updated
together in any amending change.

**Version**: 3.1.0 | **Ratified**: 2026-07-29 | **Last Amended**: 2026-08-09
