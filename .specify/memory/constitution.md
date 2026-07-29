<!--
Sync Impact Report
==================
Version change: TEMPLATE (unratified) → 1.0.0
Rationale: Initial ratification. This is the first concrete constitution for
the project (previous file was the unfilled placeholder template), so it is
treated as a MINOR-vs-MAJOR non-event and seeded at 1.0.0 per semantic
versioning guidance for first stable adoption of a governance document.

Modified principles: n/a (initial fill; no prior named principles existed)

Added sections:
- Core Principles I-VII (Daemon-Only Control Plane, Broker-Mediated
  Privilege, Reasonable Isolation Over Convenience, Contract-Driven
  Compatibility, Test-Layer Discipline, Panel-Gated Multi-Phase Work
  [explicitly names the default ten-role panel roster], Traceable,
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

Templates requiring follow-up (not modified by this command; flagged for
maintainers of dependent Spec Kit templates/commands):
- .specify/templates/plan-template.md - verify "Constitution Check" gates
  reference these seven principles once that template exists/is edited.
- .specify/templates/spec-template.md - none required (no conflicting
  language found; N/A at this time since template not present in repo).
- .specify/templates/tasks-template.md - none required (N/A, not present).
- README.md / AGENTS.md - no edits made; this command's scope is
  constitution-only per the Scope Guard.
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
A multi-phase plan MUST pass a panel sign-off gate at each phase boundary
before the next phase's implementation is dispatched: unanimous sign-off
(`recommendations: []`) from the applicable roster, following plan review →
implementation → integration → work review → advance. The default
per-round roster is the **ten-role panel** (`software`, `test`, `nixos`,
`networking`, `security`, `rust`, `product`, `docs`, `observability`,
`kernel`); a plan may name a different roster explicitly, but the gate
requires unanimous sign-off from whichever roster is selected. Green tests
alone never waive this gate. Trivial fixes, time-critical hotfixes (with a
mandatory post-fix panel), and documentation-only changes that describe no
load-bearing behavior are the only exceptions. Where a harness (e.g. a
five-seat phase council) stands in for the per-round ten-role panel, it MUST
preserve the same unanimity rule and no-rerun discipline; it does not
substitute for the separate, binding ten-role panel required once at a
wave's close for wave-scale (ADR 0046-class) work.
Rationale: the project's own history includes a panel catching 11 HIGH
findings that automated tests caught none of - testing and review are
complementary, not substitutable.

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

**Version**: 1.0.0 | **Ratified**: 2026-07-29 | **Last Amended**: 2026-07-29
