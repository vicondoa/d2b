# Contract: Operator CLI

**Owning spec**: `ADR-046-cli-and-operations` | **Wave**: W5 (surface), W7 (cutover verbs)

## What this surface is

The `d2b` binary is the only operator surface. There is no bash fallback and no env-knob
escape hatch. Companion tools consume it, so its shape is a public contract, not an
implementation detail.

## The clean break

`ADR-046-cli-and-operations` defines a "v2 command surface removed at 3.0 clean break". This
is the single change with the widest blast radius outside the host itself: every desktop
companion reads this surface or the socket beside it.

## Obligations

| # | Obligation | Requirement | Wave |
| --- | --- | --- | --- |
| CLI-1 | Resource inspection: list and inspect resources, owning Provider, status, and the reason for any degraded or failed condition | FR-016, SC-005 | W5 |
| CLI-2 | Every failure names a specific cause and an actionable next step | FR-017, SC-004 | W5 |
| CLI-3 | Cutover verbs: a non-mutating preview, and an apply gated on explicit intent plus exact content-bound consent | FR-020, FR-021 | W7 |
| CLI-4 | The apply path refuses to pass the rollback boundary without a recorded recovery-point attestation | FR-043, SC-025 | W7 |
| CLI-5 | Retired verbs are removed with a removal proof, in their own commit, after the successor is integrated | FR-023 | W5-W7 |
| CLI-6 | `d2b userd` is removed only after parity with the fixed user supervisor Process | FR-041 | W5 |

## Retirement register

Populate as each is retired. `d2b host migrate-storage` is already classified: it served the
one-time v1-to-v2 storage layout cutover and has **no v3 successor**, so it belongs on the
FR-042 explicit retirement list rather than the parity list.

| Verb | Successor | Parity or retirement |
| --- | --- | --- |
| `d2b host migrate-storage` | none | Retirement - justified, must appear in release notes |
| `d2b userd *` | fixed user supervisor Process under `Provider/system-systemd` | Parity - gated |

## Acceptance

- Both machine-readable and human output modes behave per the CLI contract, with exit codes
  matching the documented table.
- The cutover preview modifies nothing, and the apply path is unreachable without both consent
  and attestation.
- No retired verb remains, verified by its removal proof.
