# ADR 0022: Stabilization-mode releases

- Status: Accepted (v1.2)
- Date: 2026-06-10
- Plan slice: v1.2 §P1.1 "Pre-tag gate infrastructure"
- Companion ADRs: [ADR 0015](0015-daemon-only-clean-break.md),
  [ADR 0017](0017-no-bash-fallbacks-invariant.md),
  [ADR 0018](0018-microvm-nix-removal.md),
  [ADR 0021](0021-broker-user-namespace-for-virtiofsd.md)

## Context

D2b follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).
Under that spec, a MINOR version increment signals backward-compatible
changes. A **stabilization release** is a specific flavour of MINOR
bump: it introduces no new public surface, carries no new feature work,
and closes every tracked deferral from prior releases. Its defining
criterion is the deferral-zero invariant (I3, see below).

v1.2 is the first stabilization release. The impetus: v1.0 and v1.1
each deferred a small set of correctness and completeness items
(seccomp BPF compilation, broker-pre-NS extension to remaining roles,
bridge IPv6 sysctl boot-time application, runner-shape test coverage,
etc.) that are individually low-risk but collectively represent
technical debt whose accumulation would compound over future feature
releases. A dedicated stabilization cycle brings the deferral list to
zero without introducing new scope, making v1.3 a clean feature-work
baseline.

Without a documented policy, "stabilization" is an informal intent
that can drift as the cycle progresses. Panel reviewers and sub-agents
need a crisp, enforceable definition so they can evaluate whether a
proposed commit is in scope and whether the tag preconditions are met.

## Decision

### Stabilization release = SemVer MINOR, deferral-zero flavour

A stabilization release:

1. **Is a SemVer MINOR increment** — backward-compatible at every
   public surface (wire protocol, option schema, bundle contract, CLI
   exit codes). No public API is removed or changed in a breaking way.
2. **Carries no new public surface** — no new options, no new wire
   messages, no new CLI verbs. Additive-only changes that are required
   to close a tracked deferral are allowed; unsolicited additions are
   not.
3. **Closes every tracked deferral** — every item in the deferred list
   (v1.2: D1..D18) either ships or receives an explicit scope-removal
   commit with documented justification. A deferral is removed from
   scope only when upstream blockers make it technically infeasible
   within the release cycle and the blocker is documented as a release
   caveat; removal is not used to lower the bar.
4. **Introduces zero new deferrals** — no work is moved to a future
   release to make room for new features. The no-new-deferral invariant
   (I3) is the defining criterion: a stabilization release passes I3
   or it does not ship.

### No-new-deferral invariant (I3)

> **I3**: Zero new v1.3 deferrals are authored during the v1.2
> development cycle.

Enforcement mechanism: a grep-based check is wired into
`tests/static.sh` and invoked by `make pre-tag`. The check fails
(exit non-zero) if any of the following files contains a string
matching one of these patterns:

| File scope | Patterns checked |
| --- | --- |
| `plan.md` | `v1\.3 deferral`, `Tracked for v1\.3`, `TODO\(v1\.3\)` |
| `CHANGELOG.md` | same |
| `docs/adr/*.md` | same |
| `nixos-modules/**.nix` | same |

These patterns are chosen to catch the three ways a deferral is
typically authored in this repository:

- Inline `v1.3 deferral:` comment in prose or Nix source
- `Tracked for v1.3` in a CHANGELOG or ADR section heading
- `TODO(v1.3)` Rust/Nix code annotation

Committing a new deferral-shaped string causes the static gate and
the pre-tag gate to fail closed. If a future release genuinely needs
to defer something, the author must update the pattern list in this
ADR (plus the grep invocation in `tests/static.sh`) via an ADR
amendment, explicitly surfacing the policy relaxation for panel review.

### Required pre-tag live-smoke gate (D1)

No v1.2.x tag is cut without a passing run of
`tests/integration/live/live-vm-smoke.sh --full`. This gate:

- Exercises end-to-end VM bring-up for both `personal-dev` and
  `work-aad` on the maintainer's desktop (requires KVM + systemd +
  privileged broker; skipped in CI).
- Validates the functional sidecar probes listed in ADR 0023's
  lifecycle matrix: virtiofsd file-IO, TPM `tpm2_getrandom`, CH HTTP
  API liveness, CAP_NET_ADMIN bit-clear, pidfd-table snapshot, zero
  zombies, `d2b host doctor --read-only` exit 0, and teardown
  cleanliness.
- Records the result in `${TMPDIR:-/tmp}/d2b-smoke-run-log.txt` as a single line:
  `<HEAD-SHA> <ISO-timestamp> {PASS|FAIL} {lite|full}`.

Panel sub-agents verify the smoke gate at each panel round (I5) by
reading the operator-supplied smoke log and asserting the most-recent
line matches the HEAD SHA under review. R1 and R3 require `--full`
mode; R2 allows `--lite`.

The gate is **maintainer-side only** and never runs in CI (no KVM
access in the standard GitHub Actions runner pool). CI covers the
Layer-1 static gate (`tests/static.sh`) and the subset of tests safe
to run without a privileged host environment.

### Panel-signoff cadence

A stabilization release requires three panel rounds before tagging:

- **R1**: Full 10-discipline panel dispatched at the feature-complete
  HEAD. Each panelist verifies I5 (smoke log matches HEAD, `--full`
  mode). R1 must-fix items are drained as `v1.2fuN` commits; each
  commit cites its panel finding reference (I7).
- **R2**: Panels that returned CONDITIONAL / FAIL / CRITICAL in R1
  are re-dispatched. PASS panels are spot-checked on changed surfaces.
  Smoke log may be `--lite` for R2.
- **R3**: Full 10-discipline panel at the R2-drained HEAD. Smoke log
  must be `--full`. Requires 10/10 PASS with no unresolved must-fix
  items before the tag is cut.

If R3 surfaces new must-fix items, R2 and R3 are repeated. A fourth
round is an escalation signal (scope creep); the integrator surfaces
it to the maintainer rather than silently repeating the cycle.

Each panel round requires the maintainer to preserve the updated smoke
log before dispatching panel sub-agents, so the I5 check is
machine-verifiable without adding repo-local test artifacts.

### Tag-signing policy

v1.2 tags are **annotated-only** (`git tag -a`), not GPG-signed.

Rationale: consumer flake-lock verification uses Git tree hashes,
which are cryptographically committed and verified by Git itself (SHA-1
with collision-resistant content addressing, moving toward SHA-256).
A GPG-signed tag adds an out-of-band key management burden (key
distribution, key rotation, consumer toolchain setup) without closing
a real attack vector at the consumer integration layer: the attacker
who can substitute a tag object can equally substitute the commit it
points to, and the consumer's `flake.lock` pins the tree hash, not the
tag object. GPG tag signing addresses a threat model where tag objects
are mutable and consumers verify them out-of-band; neither premise
holds here.

The tag message for each v1.x tag cites every deliverable closed
(D1..D18 for v1.2) and the panel-round reference confirming 10/10
PASS, so the release audit trail lives in the annotated tag object
itself rather than in a separate signed artefact.

## Consequences

Positive:

- The deferral list has an enforceable zero floor at every v1.2 tag.
  `tests/static.sh` and `make pre-tag` fail closed on any new
  deferral string before it can reach a tag candidate.
- The stabilization-mode definition is precise enough for panel
  reviewers to evaluate scope-creep proposals against a concrete
  criterion.
- The three-round panel cadence with a machine-verifiable smoke log
  ensures no tag ships without a recent live-deploy validation.
- The tag-signing decision is explicit and revisable: if future
  consumer tooling requires GPG-signed tags, an ADR amendment
  documents the change.

Negative:

- The no-new-deferral grep patterns are fragile: a developer who
  writes `v1.3 deferral` in a commit message body (not checked) but
  not in the listed files would bypass the gate. The check covers
  the canonical locations where deferrals are authored in this repo;
  it is not a general-purpose deferral detector.
- Scope-removal of a deferral (removing D-N from the list because it
  is upstream-blocked) requires a documented justification commit, not
  just a plan-file deletion. The gate does not enforce this; it relies
  on panel review to catch undocumented scope removals.

## Future work

- v1.3 (feature release): the I3 grep patterns in `tests/static.sh`
  are updated to target `v1\.4 deferral` etc. before the v1.3
  development cycle opens.
- If a future stabilization release needs multiple deferral-target
  versions under development simultaneously, the grep pattern list
  will need to be parameterised (e.g., driven by a
  `NEXT_RELEASE_TAG` variable in the Makefile).

## References

- [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html)
- v1.2 plan §P1.1, §6 (invariants I1–I7)
- [ADR 0015](0015-daemon-only-clean-break.md) — daemon-only clean break (v1.0)
- [ADR 0017](0017-no-bash-fallbacks-invariant.md) — no bash fallbacks (v1.1)
- [ADR 0018](0018-microvm-nix-removal.md) — microvm.nix removal (v1.1)
- [ADR 0021](0021-broker-user-namespace-for-virtiofsd.md) — broker user-NS for virtiofsd (v1.1.1)
