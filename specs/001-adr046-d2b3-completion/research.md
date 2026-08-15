# Phase 0 Research: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

This document resolves the unknowns in the plan's Technical Context. Every decision below is
grounded in the Accepted ADR-046 specification set, the committed implementation, or
the working tree. Where the normative specs already decide something, this document records
the decision and its source rather than re-deciding it.

---

## R1: What exactly does the next wave (W2) contain?

**Historical record**: W2 was described as 19 implementation items across two specs, in two
independent groups with zero recorded file-overlap edges. This retained sequencing note
documents the dependency history.

| Group | Items | Scope |
| --- | --- | --- |
| `wi:ADR-046-primitive-resource-composition` | `ADR046-primitives-001..003` (3) | The nine v3 primitive resource contract modules; systemd and minijail system Providers; volume Provider schema half |
| `wi:ADR-046-zone-routing` | `ADR046-routing-001..016` (16) | Zone routing engine, resolver, service; bus session/transport/zone-route/relay; Zone Nix option generation; resource client |

**Rationale correction (2026-08-06)**: the original research declared W2 entry satisfied
because all 14 prior work items were `Merged`. That proves the predecessor-state exit
condition only; it does not prove T008's candidate-recovery prerequisite or the other
pre-dispatch facts. Downstream W2 implementation now exists while T008 remains unchecked.
T008 is therefore a historical entry record, not a current closure condition. The retained
task records show zero `file-overlap-order` edges and zero `shared-contract` edges touching
W2. The implementation record states that `nixos-modules/assertions.nix` has one
writer, `ADR046-routing-011`. The affected groups were file-disjoint; that fact
does not retroactively establish product behavior.

Within `zone-routing` there are 22 dependency edges forming a chain rooted at
`ADR046-routing-001` (the `zone_routing.rs` contract) and `ADR046-routing-007` (the bus
session module). `ADR046-routing-016` is the deepest node at rank 11. The three
`primitives-*` items have no intra-wave dependencies at all and can start immediately.

**Alternatives considered**: Serializing the two groups. Rejected because it
contradicts FR-028 and there is no file overlap to justify it.

### Spec drift found (record, do not silently fix)

Historical task prose names `retired process package` and
`packages/d2b-provider-supervisor/` as destinations, but no current task owns
either path. The implementation boundary is `ADR046-process-001` and the
committed Provider supervisor code; current work follows that boundary.

**Historical disposition**: The two *system Provider* crates and the shared
process-conformance library are recorded as W2 work, while `d2b-process` and
`d2b-provider-supervisor` are recorded as W4. Current implementation follows
committed code, the owning product contracts, and focused validation.

**Historical status note (2026-08-07)**: This research record is historical.
The §3.2 destination drift remains unresolved; resolve it against committed code,
the owning product contract, and focused evidence.

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

**Historical implementation consequence**: `ADR046-store-004` and `ADR046-store-002` were
recorded as an atomic backend/watch change alongside the related store and reconciliation
items. The backend cannot be established by watch-layer evidence alone, and the watch layer
cannot be established without the corrected backend. This remains the highest-risk technical
item in the retained record: if the corrections do not recover 640 KiB, the design changes
rather than the threshold.

**Alternatives considered**:
- Subtract an empty-process baseline. **Rejected by the spec explicitly** - the gate is
  whole-process and "no process-baseline subtraction is allowed".
- Re-baseline the threshold through a budget-contract change. Rejected: it would not weaken
  durability/authz/audit literally, but it defeats the stated intent and would invalidate D128
  and every contract that cites the budget.
- Reduce durability (fewer fsyncs, group-commit relaxation). Rejected: forbidden outright;
  the spec pins full crash-safe durability with one fsync per write transaction and no
  reduced-durability mode.

---

## R3: Which desktop companions block the release, and what does each consume?

**Decision**: Four companions form the current release-blocking set required by FR-039 and
FR-040. `docs/reference/companion-contracts.md` revision 2 is the canonical inventory. The
original reconstruction from README, AGENTS.md, reference docs, ADRs
0035/0040/0041/0042/0043, and `nixos-modules/` established publication as a deliverable;
revision 2 records the negative determination that excludes `weezterm`.

| Companion | d2b surface consumed | Blocking risk |
| --- | --- | --- |
| `d2b-toolkit` | Shared client DTOs, public-socket framing, Wayland color parsing, Waybar helpers. Other companions build on it. | **Highest** - it is the shared substrate; it must adapt first or the rest cannot |
| `d2b-wlterm` | Qualified ShellSession Resource lifecycle, ProcessAttachClient named streams, provider-neutral launcher metadata, canonical Host/Guest execution references, and the `d2b-wayland-proxy` package. The retired public-socket `ShellOp` family is unsupported. | High - the shell and launcher integration moves to the v3 replacement surfaces |
| `d2b-wlcontrol` | Public socket only; `/etc/d2b/ui-colors.{json,css}`; `d2b audio status --json`; security-key state via `WlcontrolSkStatus`/`WlcontrolAction`; graceful-stop semantics | High - deepest contract surface of any companion |
| `d2b-clip-picker` | Versioned newline-delimited JSON picker protocol over an inherited `socketpair()` fd; realm target names and accent colors | Medium - protocol is versioned and d2b-clipd-owned, but target naming changes |

**Rationale for treating this as a deliverable, not a lookup**: `AGENTS.md` names no companion,
and README names only three of the four current members in one prose line under non-canonical
short names. A release gate that depends on "every companion" needs the set written down and
versioned, or the gate is unenforceable.

**Decision detail**: The companion inventory MUST be published as a reference document naming
each companion, the exact surface it consumes, and its verification status. The natural home is
`docs/reference/`, and per FR-019 it lands in the same implementation change as the contract
changes it tracks.

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
is in scope and belongs with the implementation that removes each path, not deferred to a
later bulk cleanup.

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

**Verification and code-canon correction**: committed code exposes canonical `d2b.slice` plus
the three persistent service/socket units `d2bd.service`, `d2b-priv-broker.socket`, and
`d2b-priv-broker.service`. The AGENTS.md exit-criterion prose that counts three raw
`d2b*`/`microvm*` matches is therefore stale. FR-075 is the exact canonical comparison:
enumerate the complete loaded namespace, fail if enumeration fails, exclude only
`d2b.slice`, sort, and require exactly those three remaining names. Injected unexpected
slice and service names remain after the sole exclusion and fail equality. This feature batch
records the drift and keeps committed code; it does not edit AGENTS.md. The map's own removal
proof for the per-realm family remains `systemctl list-units 'd2b-r-*'` returning empty.

**Alternatives considered**: Declaring per-Zone systemd units for supervision. Rejected -
directly prohibited by Principle I and ADR 0015, and the specs already chose parent-spawn.

---

## R6: Focused validation readiness

Current readiness is established by focused tests for the changed component, fixture-backed
contract checks where applicable, and conditional container, host, live, hardware, or
performance lanes. An advisory result is not enforcing evidence. Generated product schemas
still use their owning ecosystem generators and move with source contracts. Rust capability
boundaries use retained defining-crate compiler assertions and focused contract tests.

## R7: Toolchain, platform, and scale parameters

**Historical environment snapshot (superseded where current plan or committed code differs):**
The research inherited the then-pinned environment unchanged. No new
toolchain, linter, formatter, or overlay was introduced (Principle: constitution "Additional
Constraints").

| Parameter | Value | Source |
| --- | --- | --- |
| Rust toolchain | `1.94.1`, components `rustfmt`, `clippy` | `packages/rust-toolchain.toml` |
| Workspace | 35 members; `d2b-priv-broker` and `d2b-guest-shell-runner` excluded by design | `packages/Cargo.toml` |
| Storage engine | redb `=4.1.0`, **provisional pin**, isolated to the proof workspace until the corrected backend lands | D128, spike `Cargo.toml` |
| Target platform | `x86_64-linux` NixOS with KVM; graphics gated to x86_64 | `checkVmPlatform`, flake checks |
| Test layers | nix-unit eval cases, cargo unit/integration, rendered-artifact contract tests, policy lints, flake checks; podman containers and `runNixOSTest` for Layer 2 | `tests/AGENTS.md` |
| Hard scale fixtures | 10,000 resources; 100 live watches | §10.4 |
| Program size | Historical scope of 531 implementation items across 53 specs and 7 sequencing increments; 27 Provider crates | retained implementation task records; historical and non-authoritative |

---

## Open risks carried into the plan

| # | Risk | Why it matters | Mitigation |
| --- | --- | --- | --- |
| RK-1 | The RSS corrections may not recover 640 KiB | The resource-store footprint remains unverified until the rerun completes | Prototype the fan-out and range-seek corrections in the existing proof workspace, so the rerun is a confirmation rather than a discovery |
| RK-2 | Companion adaptation stalls the release | FR-039 makes external repositories a release blocker while FR-045 forbids publishing a preview they could build against | Publish the companion inventory and replacement contracts early; track each companion's status as a release checklist item |
| RK-3 | Provider work spans many crates | Cross-provider changes can hide contract drift | Keep each Provider's hermetic suite independently testable and use focused checks for each changed family |
| RK-4 | Destructive validation on the daily driver | A bad cutover costs the working environment | FR-043 recovery-point attestation is the primary control; rehearse cutover phases on disposable Zone state first |
| RK-5 | 33 of the current 48 DELETE/REPLACE census rows lack removal proofs | FR-023 requires one per removed path | Add a path-specific proof with the relevant source, packaging, fixture, and policy checks before removal |
| RK-6 | Historical foundation work has uneven evidence | Later components depend on those contracts | Run focused contract and live-reachability checks for the affected foundation surface; use current evidence for current decisions |

## NIX-8 and NIX-9 code-canon adjudication

Read-only searches against the current merged code found:

```text
hostGenerationRebuildRef|host-generation-rebuild-ref
  -> zero matches in nixos-modules/, packages/, tests/, examples/, templates/

ApplyHostGenerationHandoff|SourceGenerationCompatibilityFloorV1|
apply-authorized-handoff|source-handoff-v1
  -> zero matches in packages/, nixos-modules/, tests/

host generation|host-generation
  -> zero matches in the d2b, d2bd, broker, and contracts source trees

ZoneHandlerName::SystemCoreHost|ZoneHandlerName::SystemCoreUser
  -> zero matches in the contracts, daemon, and system-core source trees

OpenPeerPidfdFromAcceptedSocket
  -> zero matches in packages/ and tests/
```

Therefore NIX-8, NIX-9, the handler contract, and the peer-pidfd operation are not landed.
Their prospective ownership, ordering, and file maps remain described by the owning member
specifications and implementation contracts.

---

## R8: Can privilege-dropped publication link an exact unnamed inode?

**Prior observation**: the 2026-08-07 feature-filesystem probe reported
success for `linkat(AT_EMPTY_PATH)`, but it did not bind the process effective capability
set, user namespace, procfs mount, target mount, kernel, or production filesystem. Linux
restricts `AT_EMPTY_PATH` linking to a caller with `CAP_DAC_READ_SEARCH`; that observation
therefore cannot authorize the required zero-effective-capability target.

**Historical Wave 5 decision**: the retired plan forbade `AT_EMPTY_PATH` and
create-and-unlink link probing. Its preimage, candidate-sidecar, and request-output design
wrote, file-synced, and revalidated an
`O_TMPFILE` inode, retain a validated procfs `/proc/self/fd` directory fd, and use
`linkat(proc_self_fd_dirfd, decimal_fd, target_parent_fd, final_name,
AT_SYMLINK_FOLLOW)` to capability-free link the exact opened inode directly to its final
no-replace name. No linked temporary or name-consuming publication rename exists.

The former matrix and its failure injections are read-only historical design evidence.
They authorize no current run or publication path.
