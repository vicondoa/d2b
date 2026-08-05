# Friction Log

**Feature**: `001-adr046-d2b3-completion` | **Opened**: 2026-07-29

Durable record of what slowed delivery down. Required by constitution 2.1.0 Principle VI
("Delivery memory").

This is **not** a retrospective document written at the end. It is an input to planning,
maintained continuously, and it feeds the terminal wave directly: W8 has no spec members
because its contents *are* the accumulated friction, triaged at W7 close into the six
categories below.

## Categories

W8's destinations map one-to-one onto these, so categorize accordingly:

| Category | Typical destination |
| --- | --- |
| `signoff` | Panel process, review latency, roster, deferral pressure |
| `build` | Compile times, toolchain, sccache, worktree setup |
| `test` | Suite runtime, flakiness, gate placement, `tests/tools/` |
| `merge` | PR flow, stacking, rebasing, CI latency, `merge-eligibility` |
| `codegen` | `xtask gen-*`, drift gates, manifest regeneration |
| `disk` | Store growth, target dirs, GC, the 10 GiB preflight |

## What may be recorded

Classification metadata and impact. **No panel transcript, validation command output, or
attestation payload.** Describe the friction, not the review text that surfaced it.

## Log

| ID | Date | Category | Wave | Friction | Impact | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F001 | 2026-07-29 | `signoff` | W1 | A single wave required 21 follow-up rounds (W0 required 14) | Review consumed a large multiple of implementation time | Mitigated by constitution 2.1.0 bounded deferral; verify the effect at W2 close |
| F002 | 2026-07-29 | `signoff` | all | Panel review commonly runs 1-2x the coding duration; strict serialization idles implementation capacity for more than half of each cycle | Motivated the pipelined-wave amendment | Mitigated by constitution 2.0.0 pipelined dispatch; not yet executable (T585-T587) |
| F003 | 2026-07-29 | `signoff` | W0/W1 | Delivery state holds 10 competing W0 candidates, 1 panel-request, 0 receipts, 0 seals; W1 has no delivery state at all | Neither wave produced a seal; the contract's exit criteria were not met in practice | Accepted under the FR-034 waiver; sealed delivery starts at W2 |
| F004 | 2026-07-29 | `test` | W1 | The storage spike missed its whole-process RSS budget by 640 KiB (2.6%), deferring the production engine and its watch consumer from W1 to W5 | Blocks the critical path; W5 cannot start its store chain until corrections are designed | T007 prototypes the corrections early so W5 confirms rather than discovers |
| F005 | 2026-07-29 | `codegen` | W2 | `ADR-046-validation-and-delivery` §3.2 names two crate paths under W2 that no W2 work item targets; the graph assigns them to W4 | Spec prose and generated graph disagree; an implementer following prose targets the wrong wave | T575 raises it as a separate amendment; FR-046 makes the graph authoritative |
| F006 | 2026-07-29 | `test` | all | The migration map supplies explicit removal proofs for only 3 of its 16 DELETE rows | FR-023 requires one per path; 13 must be authored | T576 inventories and assigns them |

## Standing obligations

- **Log continuously.** A friction point recorded at W7 close from memory is worth much less
  than one recorded when it was felt. Add the entry in the wave where it happened.
- **Categorize on entry.** An uncategorized entry cannot be triaged into W8.
- **Record mitigation and verify it.** An entry whose Status claims mitigation should be
  re-checked at the next wave close. F001 and F002 both carry unverified mitigations right now.
- **Escalate structural friction.** If the same category recurs across three waves, it is not
  friction, it is a design problem in the delivery process, and it should become a task rather
  than another log line.

## Known unmitigated friction

Tracked here so it is not lost between waves:

- **Panel model migration was resolved.** T581-T584 landed the prior Gemini
  binding; current panel requests use `gpt-5.6-sol` at `xhigh`, while the
  Gemini/high pair remains readable for compatibility.
- **Pipelined dispatch is not executable.** Until T585-T587 land, §4 and the `wave snapshot`
  entry check still enforce the stricter serial rule.
- **FR-043 is unenforced by any gate.** It is tracked program-local by decision, so a green
  W7 seal is not evidence it shipped.
- **W6 is 257 work items across 27 crates** - nearly half the program in one wave. If wave
  size proves to be the driver of deferral pressure, W6 is where it will show.
