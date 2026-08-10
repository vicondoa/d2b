# Superseded Deferred Findings Register

**Feature**: `001-adr046-d2b3-completion` | **Opened**: 2026-07-29

This file is a read-only historical compatibility record for Constitution 2.1.0's retired
bounded-deferral process. Current Constitution 3.0 Discover-Fix-Verify has no round threshold
and permits no finding to become nonblocking because of elapsed rounds. Current lifecycle
findings are not added here.

## Superseded rule

The former process allowed a reviewer after eight panel rounds to refile some LOW or MEDIUM
findings here. That rule is no longer operative. It is preserved only to explain the legacy
table shape and must not be used as panel, signoff, planning, release, or successor-entry
authority.

Current handling is:

- one comprehensive discovery produces the stable shared ledger;
- fixes and verification remain ledger-scoped;
- pre-existing late MINOR and NIT observations remain nonblocking history;
- admitted late BLOCKER and MAJOR findings remain blocking; and
- no round count changes a finding's status.

The binding invariant remains `signoff = true` if and only if `recommendations` is empty.
Nothing in this historical file removes a recommendation or supplies signoff.

## Historical record shape

Legacy entries, if any had existed, contained classification metadata only and never panel
transcripts, validation output, or attestation payloads.

| ID | Wave | Round | Role | Severity | Subject area | Concern (one line) | Disposition |
| --- | --- | --- | --- | --- | --- | --- | --- |
| _(none recorded)_ | | | | | | | |

The legacy disposition vocabulary was `open`, `scheduled`, `fixed`, and `withdrawn`. Those
values are documentation of the retired format, not current transitions.

## Current ownership

This register has no entry workflow, review cadence, terminal-wave feed, release gate, or
standing obligation. Current lifecycle findings stay in the stable discovery ledger and
current process friction is recorded in `friction-log.md` without transcripts or attestation
payloads.
