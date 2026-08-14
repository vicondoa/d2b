# ADR 0022: Stabilization-mode releases

- **Status:** Accepted (v1.2)
- **Date:** 2026-06-10
- **Companion ADRs:** [ADR 0015](0015-daemon-only-clean-break.md),
  [ADR 0017](0017-no-bash-fallbacks-invariant.md),
  [ADR 0018](0018-microvm-nix-removal.md),
  [ADR 0021](0021-broker-user-namespace-for-virtiofsd.md)

## Context

A stabilization release is a SemVer MINOR release that introduces no new
public surface and closes every tracked deferral from prior releases. The
deferral-zero invariant keeps correctness and completeness work from
accumulating while preserving the existing wire protocol, option schema,
bundle contract, and CLI behavior.

## Decision

A stabilization release:

1. is backward-compatible at every public surface;
2. carries no unsolicited options, wire messages, or CLI verbs;
3. closes every tracked deferral, or records an explicit scope-removal
   justification when an upstream blocker makes closure infeasible; and
4. introduces zero new deferrals.

For v1.2, the zero-deferral criterion remains a release requirement. Ordinary
static and pre-tag checks apply to the documented source, ADR, plan, and
changelog locations; this ADR does not define a separate validator.

No v1.2.x tag is cut without a passing
`tests/integration/live/live-vm-smoke.sh --full` run on the maintainer host.
The run validates VM bring-up and teardown, runner I/O, TPM behavior, HTTP
liveness, capability drops, pidfd state, zombie absence, host diagnostics,
and cleanup. It records the head revision, timestamp, result, and mode in the
operator's temporary smoke log. This is a maintainer-side gate and is not part
of the ordinary CI runner.

v1.2 tags are annotated Git tags and are not GPG-signed. Consumer flake-lock
verification uses the pinned Git tree hash; an additional tag signature does
not close a threat at the consumer integration boundary.

## Consequences

The deferral list has an enforceable zero floor before a stabilization tag.
Static checks catch new deferral markers, and the live smoke run supplies
host-only evidence that cannot be obtained from ordinary CI.

The grep patterns are intentionally narrow and do not detect arbitrary
deferral prose. Scope removal still requires a documented justification and
ordinary maintainer review.

## Future work

Before a future stabilization cycle opens, update the static patterns to the
next release identifier. If several release targets are active at once,
parameterize the pattern source rather than duplicating checks.
