# Phase 0 Research: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

This document resolves the unknowns in the plan's Technical Context. Every decision below is
grounded in the Accepted ADR-046 specification set, the committed implementation graph, or
the working tree. Where the normative specs already decide something, this document records
the decision and its source rather than re-deciding it.

---

## R1: What exactly does the next wave (W2) contain?

**Decision**: W2 delivers 19 work items across two specs, in two independent parallel groups
with zero file-overlap edges between them.

| Group | Items | Scope |
| --- | --- | --- |
| `wi:ADR-046-primitive-resource-composition` | `ADR046-primitives-001..003` (3) | The nine v3 primitive resource contract modules; systemd and minijail system Providers; volume Provider schema half |
| `wi:ADR-046-zone-routing` | `ADR046-routing-001..016` (16) | Zone routing engine, resolver, service; bus session/transport/zone-route/relay; Zone Nix option generation; resource client |

**Rationale**: W2 entry criteria are already satisfied. All 14 prior work items are `Merged`,
which is the condition wave entry actually tests. The graph records **zero
`file-overlap-order` edges and zero `shared-contract` edges touching W2**, and the generated
index states plainly that "W2 has one writer" for the single contended file
(`nixos-modules/assertions.nix`, written only by `ADR046-routing-011`). The two groups are
therefore genuinely file-disjoint and MUST be launched in the same coordination cycle per
FR-028.

Within `zone-routing` there are 22 intra-wave dependency edges forming a chain rooted at
`ADR046-routing-001` (the `zone_routing.rs` contract) and `ADR046-routing-007` (the bus
session module). `ADR046-routing-016` is the deepest node at rank 11. The three
`primitives-*` items have no intra-wave dependencies at all and can start immediately.

**Alternatives considered**: Serializing the two groups. Rejected: it contradicts FR-028 and
the delivery contract's positive anti-serialization obligation, and there is no file overlap
to justify it.

### Spec drift found (record, do not silently fix)

`ADR-046-validation-and-delivery.md` §3.2's W2 row names `packages/d2b-process/` and
`packages/d2b-provider-supervisor/` as W2 destinations. **No W2 work item has either path.**
The only owner is `ADR046-process-001`, which the graph assigns to **W4**. The machine-readable
`ADR-046-work-items.json` and `ADR-046-implementation-graph.json` are authoritative per the
repository's "existing code is canon" rule.

**Decision**: Treat the graph as authoritative. W2 creates the two *system Provider* crates
plus a shared process-conformance library; `d2b-process` and `d2b-provider-supervisor` are W4.
Record this in the plan's drift log and raise it to the integrator; correcting the §3.2 prose
is a specification amendment that would re-open that spec's validation evidence, so it is not
done inside a wave.

---

## R2: How is the failed storage footprint target resolved?

**Decision**: By the four named design corrections already assigned in decision D128, then a
rerun of the **unchanged** whole-process gate with **no baseline subtraction**.

The spike measured 25,216 KiB against a 24,576 KiB gate: 640 KiB, about 2.6 percent over. Six
of seven thresholds passed. The corrections are split across two work items and are not
negotiable:

| Owner | Corrections |
| --- | --- |
| `ADR046-store-004` (backend) | Revision-key range-seek replay; streaming decode that never decodes older complete envelopes; one shared immutable `ChangeBatch` fanned out to matching watchers instead of a clone per watcher; bounded backend signals for range seeks, replay rows scanned/decoded, shared batches and fan-out references, and writer queue depth/capacity |
| `ADR046-store-002` (watch consumer) | One global bounded watch-admission budget; small per-watch cursor/filter state only; typed admission backpressure before registration; deterministic slow-watcher eviction with cursor-based resume; watch-budget signals |

**Rationale**: Every one of these is a memory-amplification fix. Clone-per-watcher fan-out and
decode-everything replay are the two structural reasons a 100-watch fixture inflates resident
memory, so the corrections attack the cause rather than the threshold. The delivery contract
states that a failed hard target "changes the Proposed design; it is never resolved by
weakening durability, authorization, or audit", which FR-030 restates.

**Consequence for sequencing**: `ADR046-store-004` and `ADR046-store-002` move **atomically**
and both sit in W5 with `ADR046-store-003`, `-005`, and `ADR046-reconcile-003`. The backend
cannot be accepted on watch-layer evidence and the watch layer cannot be accepted without the
corrected backend. This is the single highest-risk item in the program: if the corrections do
not recover 640 KiB, the design changes again rather than the gate moving.

**Alternatives considered**:
- Subtract an empty-process baseline. **Rejected by the spec explicitly** - the gate is
  whole-process and "no process-baseline subtraction is allowed".
- Re-baseline the threshold through a decision-register amendment. Rejected: it would not
  weaken durability/authz/audit literally, but it defeats the stated intent and would need to
  re-open D128 and every spec that cites the budget.
- Reduce durability (fewer fsyncs, group-commit relaxation). Rejected: forbidden outright;
  the spec pins full crash-safe durability with one fsync per write transaction and no
  reduced-durability mode.

---

## R3: Which desktop companions block the release, and what does each consume?

**Decision**: Five companions form the release-blocking set required by FR-039 and FR-040.
There is **no canonical inventory in the repository today**; the set below was reconstructed
from README, AGENTS.md, reference docs, ADRs 0035/0040/0041/0042/0043, and `nixos-modules/`.
Publishing that inventory is itself a deliverable.

| Companion | d2b surface consumed | Blocking risk |
| --- | --- | --- |
| `d2b-toolkit` | Shared client DTOs, public-socket framing, Wayland color parsing, Waybar helpers. Other companions build on it. | **Highest** - it is the shared substrate; it must adapt first or the rest cannot |
| `d2b-wlterm` | Public socket `ShellOp` family (`List`/`Attach`/`Detach`/`Kill`), capability discovery via `runtime.operationCapabilities.guest.shell`, launcher metadata, `d2b-wayland-proxy` package | High - `d2b shell` and launcher metadata both change shape at 3.0 |
| `d2b-wlcontrol` | Public socket only; `/etc/d2b/ui-colors.{json,css}`; `d2b audio status --json`; security-key state via `WlcontrolSkStatus`/`WlcontrolAction`; graceful-stop semantics | High - deepest contract surface of any companion |
| `d2b-clip-picker` | Versioned newline-delimited JSON picker protocol over an inherited `socketpair()` fd; realm target names and accent colors | Medium - protocol is versioned and d2b-clipd-owned, but target naming changes |
| `weezterm` | **None.** Supplies the terminal binary the wlterm launcher invokes; follows only nixpkgs | Low - no d2b runtime contract |

**Rationale for treating this as a deliverable, not a lookup**: two of the five most
contract-coupled companions (`d2b-wlcontrol`, `d2b-clip-picker`) are absent from the AGENTS.md
sibling-flake section entirely, and README names them only in one prose line. A release gate
that depends on "every companion" needs the set written down and versioned, or the gate is
unenforceable.

**Decision detail**: The companion inventory MUST be published as a reference document naming
each companion, the exact surface it consumes, and its verification status. The natural home is
`docs/reference/`, and per FR-019 it lands in the same wave as the contract changes it tracks.

**Alternatives considered**: Deriving the set at release time from flake inputs. Rejected -
these are consumer-composed siblings that d2b core deliberately does not import, so they do
not appear in `flake.lock`; the one-way composition rule makes automatic discovery impossible.

---

## R4: Which capabilities may be retired, and which must reach parity?

**Decision**: The migration map's DELETE rows split cleanly. 12 name a successor and are
parity-enforced under FR-041; 4 have no successor and belong on the FR-042 explicit
retirement list.

**Parity-enforced (successor named)**: `WorkloadPlacement` into `Guest.spec`/`ZoneLink`;
`DurableExecutionProvider` into `EphemeralProcess`; `CredentialProvider` into the three frozen
credential Provider families; `ObservabilitySinkProvider` into `Provider/observability-otel`;
`InfrastructureProvider` into Guest runtime Providers; `NodeProvider` into
`Host`/`Guest`/`ZoneLink` status; `WorkloadOp` wire family into `ResourceOp` for `Guest`;
the three per-realm PID1 units into parent-spawned processes; `d2b userd` into a fixed user
supervisor Process (explicitly "after parity"); and the `vmsRef` bridge into Zone Guest
resources.

**Explicit retirement (no successor)**:

| Artifact | Justification recorded in the map |
| --- | --- |
| `RelayProvider` trait | Dead trait, no implementation found; relay behavior covered by `TransportProvider`. Sole DELETE row with no owning work item |
| `allocator-json.nix` | No separate allocator service; provisioning folds into fixed core controllers |
| `/etc/d2b/allocator.json` | Same - artifact of the deleted emitter |
| `/run/d2b/allocator.sock` | Same - no allocator socket |

The last three are one coherent cluster: the allocator service is deleted outright.

**Gap found**: the map supplies explicit removal proofs for only **3** of the 16 DELETE rows
(the per-realm unit family, the `WorkloadOp` wire types, and `RelayProvider`). FR-023 requires
a removal proof for *each* superseded path. **Decision**: authoring the missing removal proofs
is in scope and belongs with the wave that removes each path, not deferred to W7 bulk deletion.

**Also noted**: "parity" appears in only two rows of the entire map, both for `d2b userd`. It
is currently a one-off phrase rather than an enforced convention, which is precisely why
FR-041 has to state it as a requirement.

---

## R5: Do the ADR-046 Zone runtime processes violate the daemon-only constitution principle?

**Decision**: No. The design is consistent with Principle I and actively reduces unit count.

**Rationale**: The migration map assigns DELETE to `d2b-r-<realm>-broker.socket`,
`d2b-r-<realm>-broker.service`, and `d2b-r-<realm>-controller.service`, replacing each with a
**parent-spawned process** (`d2bbr-r-<zone-id>`, `d2bd-r-<zone-id>`) rather than a PID1 unit.
Per-Zone runtimes are supervised children, exactly as the current per-VM runners are. No new
`systemd.services.*` declaration enters `nixos-modules/`, and the existing policy test that
denies retired unit names continues to apply.

**Verification**: the host exit criterion in AGENTS.md - `systemctl list-units` matching
`^(d2b|microvm)` returns 3 - remains the check, and the map's own removal proof for the
per-realm family is `systemctl list-units 'd2b-r-*'` returning empty.

**Alternatives considered**: Declaring per-Zone systemd units for supervision. Rejected -
directly prohibited by Principle I and ADR 0015, and the specs already chose parent-spawn.

---

## R6: What is the delivery tooling's actual readiness?

**Decision**: The tooling exists and is production-reachable; the remaining work is hardening
and process, not creation. Wave delivery can start immediately.

**Findings**:
- `packages/xtask/src/delivery/` implements `snapshot`, `validate-import`, `panel-request`,
  `panel-attest`, `seal`, `merge-target`, `merge-eligibility`. `DELIVERY_SCHEMA_VERSION = 2`.
- Panel policy constants are pinned in code: provider `github-copilot`, model
  `gemini-3.1-pro-preview`, reasoning effort `high`, and a closed 10-role roster asserted at
  length 10. The coding model is `gpt-5.6-luna`; the two are deliberately distinct so a
  lane cannot both author a change and attest to it.
- The heavy-gate semaphore, runtime ledger, spec-set generator, and implementation-graph
  generator are all landed and under `make test-drift`.
- Delivery state resolves to `$XDG_STATE_HOME/d2b/delivery` (here:
  `~/.local/state/d2b/delivery`) and **refuses any root inside a git working tree**, which is
  what keeps FR-027's evidence out of the repository automatically.

**Caveat carried into the plan**: `history-proof` is *not* a standalone subcommand despite
being described as a "canonical proof tool"; it runs inside `merge-eligibility`. Any plan step
that assumes a separate invocation is wrong.

**Also**: all nine `ADR046-delivery-*` work items are still `Planned` even though the tooling
is landed and reachable. Their state tracks full Destination-plus-Validation verification, not
existence. They belong to W0 and its dependents, so they are inside the W0/W1 waiver's blast
radius; FR-035 requires them to reach `Merged` before any wave that owns them can seal.

---

## R7: Toolchain, platform, and scale parameters

**Decision**: Inherit the existing pinned environment unchanged. No new toolchain, linter,
formatter, or overlay is introduced (Principle: constitution "Additional Constraints").

| Parameter | Value | Source |
| --- | --- | --- |
| Rust toolchain | `1.94.1`, components `rustfmt`, `clippy` | `packages/rust-toolchain.toml` |
| Workspace | 35 members; `d2b-priv-broker` and `d2b-guest-shell-runner` excluded by design | `packages/Cargo.toml` |
| Storage engine | redb `=4.1.0`, **provisional pin**, isolated to the proof workspace until the corrected backend lands | D128, spike `Cargo.toml` |
| Target platform | `x86_64-linux` NixOS with KVM; graphics gated to x86_64 | `checkVmPlatform`, flake checks |
| Test layers | nix-unit eval cases, cargo unit/integration, rendered-artifact contract tests, policy lints, flake checks; podman containers and `runNixOSTest` for Layer 2 | `tests/AGENTS.md` |
| Hard scale fixtures | 10,000 resources; 100 live watches | §10.4 |
| Program size | 531 remaining work items, 53 remaining specs, 27 Provider crates, 7 waves | implementation graph |

---

## Open risks carried into the plan

| # | Risk | Why it matters | Mitigation |
| --- | --- | --- | --- |
| RK-1 | The RSS corrections may not recover 640 KiB | Blocks W5 and every wave after it; the gate cannot be moved | Prototype the fan-out and range-seek corrections in the existing proof workspace *before* W5 opens, so the rerun is a confirmation rather than a discovery |
| RK-2 | Companion adaptation stalls the release | FR-039 makes external repositories a release blocker while FR-045 forbids publishing a preview they could build against | Publish the companion inventory and replacement contracts early (W5, with the CLI spec); track each companion's status as a release checklist item |
| RK-3 | W6 is 257 items across 27 crates | Nearly half the program in one wave | The specs guarantee each Provider's hermetic suite compiles without any other Provider; exploit the five file-disjoint families |
| RK-4 | Destructive validation on the daily driver | A bad cutover costs the working environment | FR-043 recovery-point attestation is the primary control; rehearse cutover phases on disposable Zone state first |
| RK-5 | 13 of 16 DELETE rows lack removal proofs | FR-023 requires one per path | Author proofs with the removing wave, not at W7 |
| RK-6 | The W0/W1 waiver hides unverified foundation | Waived waves carry the contracts every later wave builds on | SC-021 forces the unwired surfaces to become reachable, which re-tests them in anger |
