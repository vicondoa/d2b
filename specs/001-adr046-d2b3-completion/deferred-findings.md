# Deferred Findings Register

**Feature**: `001-adr046-d2b3-completion` | **Opened**: 2026-07-29

Durable record of every LOW and MEDIUM panel finding deferred under constitution 2.1.0's
bounded-deferral rule. Required by Principle VI ("Delivery memory").

## The rule

Once a wave has completed **eight** panel rounds, a reviewer in round nine or later MAY
classify a **LOW or MEDIUM** finding as deferred rather than blocking. **CRITICAL and HIGH are
never deferrable** and continue to block sign-off in every round.

Deferral is a re-filing, not a dismissal. The finding moves **out of `recommendations`** and
into this register, which is what preserves the sign-off invariant: `signoff` stays `true` if
and only if `recommendations` is empty. That invariant is enforced in code at
`packages/xtask/src/delivery/panel.rs:277` in both directions, so a deferred finding left in
`recommendations` alongside `signoff: true` is rejected by `panel-attest`.

Two things are process violations, not shortcuts:

- deferring without recording the entry here; and
- re-ranking a finding the reviewer believes is CRITICAL or HIGH downward in order to defer it.

## What may be recorded

Classification metadata only. **No panel transcript, validation command output, or attestation
payload may appear in this file** - those never enter Git. Record severity, subject area,
wave, round, reviewer role, and a one-line neutral description of the concern.

## Register

| ID | Wave | Round | Role | Severity | Subject area | Concern (one line) | Disposition |
| --- | --- | --- | --- | --- | --- | --- | --- |
| _(none yet - no wave has run a panel under this plan)_ | | | | | | | |

**Disposition** is one of:

- `open` - deferred, not yet addressed
- `scheduled: W<n>` - assigned to a later wave
- `fixed: <wave>` - addressed, with the wave that closed it
- `withdrawn` - the reviewer or a later round retracted it

## Standing obligations

- **Review at every wave close.** Deferred findings are an input to the next wave's planning,
  not an archive. A finding deferred in W4 that is still `open` at W7 should be either
  scheduled or explicitly withdrawn, not carried silently to the release.
- **Feed the terminal wave.** Anything still `open` when W7 closes is triage input for W8,
  alongside the friction log.
- **Release gate.** Before tagging 3.0, confirm no `open` entry describes something that
  should have blocked. A deferral that turns out to have been a mis-ranked HIGH is a finding
  about the review process itself and belongs in the friction log too.

## Deferral pressure indicator

If a wave defers more than a handful of findings, the signal is usually not that the findings
were unimportant - it is that the wave was too large, or that a contract was unsettled when
implementation started. Record that observation in the friction log rather than only here.
