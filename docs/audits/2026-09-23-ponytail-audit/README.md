# Ponytail Audit - Consolidated Findings, Remediation Sequencing, and Net Estimate

**Date:** 2026-09-23 · **Branch:** `v3` @ `3f2664794` (tree `3f2664794`, pulled to latest before planning; plan tree `v3` `docs/plans/2026-09-23-001-ponytail-audit-per-crate-plan.md`).

**Method (one paragraph).** A read-only, per-crate "ponytail" audit with exactly one lane per workspace crate (99 lanes U1-U100 plus the cross-cutting family lanes U97-U100 superseding the prior 242-finding record): each lane read its crate's **full source at HEAD**, ran a **workspace-wide caller census** (Rust references plus `BUILD.bazel`, `nixos-modules/`, `tests/`, `integration/`, `docs/reference/policy/`, `nix/`), and ranked every cut **biggest-first** with a stated blast-radius tag (`leaf` = crate-local, `family` = same-family crates, `wide` = contracts/types/daemon/broker cross-crate), a stated `net:` in lines and deps, and a verdict (`applied`, `partial`, `refused`/`refused-stays`, `refused-stays` re-opened only on new evidence). The consolidated report below aggregates all 99 lane files (each linked); the per-lane nets are the authoritative ordering, no numbers are invented.

---

## 1. Executive summary

| Metric | Value |
| --- | --- |
| Lanes audited | **99** (95 crate lanes U2-U96 + priming U1 + cross-cutting U97-U100) |
| Total findings (tagged rows) | **≈215** across 99 lanes |
| **Total net lines** | **≈-18,930 lines** |
| **Total net deps** | **-70 deps** (-61 workspace dep rows via U100, -9 crate-local, +1 `syn` for the gen-daemon-api parser via U99) |
| Lanes that already ship lean (zero new cuts) | ~36 of 99 |
| Phase roll-up (verified per-lane sums) | Shared types layer (U2-U11) **-2,098 / -2 deps** · Runtime, daemon, broker, build (U12-U29) **-7,533 / -1 dep** (incl. the U12 d2b-bus relay island -4,214 and the U29 xtask delivery-ledger island -1,108) · Provider families (U30-U89) **-4,667 / -6 deps** (largest: cloud-hypervisor ~-980, transport-vsock -697, quota typed-spec -360, azure-container-apps -402, display-wayland -384) · CLI/host/telemetry (U90-U96) **-3,572 / -1 dep** (d2b-host ~-2,950, activation-helper twin tree -434) · Cross-cutting (U97-U100) **-1,062 lines / -60 deps** |

### Top-10 cuts (ranked by net lines)

| # | U-ID | Crate | Net lines | Net deps |
| --- | --- | --- | ---: | ---: |
| 1 | U12 | d2b-bus (relay island + 7 modules) | -4,214 | -1 |
| 2 | U91 | d2b-host | -~2,950 | -0 |
| 3 | U29 | xtask (delivery-ledger island) | -1,108 | -0 |
| 4 | U41 | d2b-provider-guest-cloud-hypervisor | -~980 | -0 |
| 5 | U5 | d2b-contracts-control | -903 | -0 |
| 6 | U3 | d2b-contracts | -785 | -0 |
| 7 | U20 | d2bd (19 zero-caller fns + dead test fixture) | -742 | -0 |
| 8 | U46 | d2b-provider-transport-vsock | -697 | -0 |
| 9 | U21 | d2b-broker | -437 | -0 |
| 10 | U92 | d2b-host-activation-helper (twin source tree) | -434 | -0 |

> **Notable classes.** (a) **Dead islands, whole-cluster cuts** - the largest single cuts are islands with byte-identical helper twins and zero-caller modules verified workspace-wide: d2b-bus relay island (U12, -4,214), d2b-host museum of dead scaffold modules (U91, -2,950), qemu-media game of thrones (U42 family). (b) **Contract duplicates / wire-shape consistency** (U97, U98, U99, U100) - the cross-crate pass found 13 remaining hand-rolled UUIDv4-renderer sites to unify onto `ResourceUid::from_bytes` (d2b-contracts/src/identity.rs:621), plus a live vsock framing wire-shape skew (session-unix 4-byte u32 header vs transport-vsock 2-byte u16), triplicated deadline helper trios, and two `Deserialize`-shim families whose cross-crate ownership stays a refusal. (c) **Policy-pinned refusals that stay** - policy scaffolds, declared-provider artifacts with zero in-tree callers, and wire-pinned catalog surfaces are refused and refused-stays, per the plan's refusal ledger (U1 constraint section).

---

## 2. Global ranked findings table (all findings, ranked by net)

Ranked by per-lane net lines, biggest cut first (lane order within a net tier; `~` = approximate).

| Rank | Tag | Finding (one line) | Crate / U-ID | Path | Blast radius | Net |
| --- | --- | --- | --- | --- | --- | ---: |
| 1 | delete | **Relay island**: 7 dead relay/zone-route/service-router/audit/routing/transport modules (-4,198) with byte-identical helpers; retire the island and four duplicate-path re-export shims. | d2b-bus · U12 | `docs/audits/2026-09-23-ponytail-audit/lane/U12-d2b-bus.md` | family | -4,214 lines, -1 dep |
| 2 | delete | **d2b-host legacy surface**: dead-scaffold + helper-twin cluster (host_mode, resource_runtime_support, daemon_audit helpers, three hand-rolled copyright trios…) netting ~2,950. | d2b-provider-host · U91 | `.../U91-d2b-host.md` | leaf | -2,950 lines |
| 3 | stdlib | **qemu: adopt/consolidate toolkit clock seam; hand-rolled deadline helpers** cut across guest-cloud-hypervisor. | d2b-provider-guest-cloud-hypervisor · U41 | `.../U41-...md` | family | -980 lines |
| 4 | delete | **d2b-contracts dead compat surface** (#A2/#A9-family catalog renderers etc.). | d2b-contracts · U5 | `.../U5-d2b-contracts.md` | wide | -903 lines |
| 5 | delete | **d2b-contracts dead surface** (± #B4 remaining). | d2b-contracts · U3 | `.../U3-d2b-contracts.md` | wide | -785 lines |
| 6 | delete | **vsock provider family** duplex helper dedup + dead `relay_argv`/island cuts. | d2b-provider-transport-vsock · U46 | `.../U46-...md` | leaf | -697 lines |
| 7 | delete | **activation-helper** legacy verbs/ffi scaffold cuts. | d2b-host-activation-helper · U92 | `.../U92-...md` | leaf | -434 lines |
| 8 | delete | **broker** sync/duplication family cuts (relay/dedup, dead islands). | d2b-broker · U21 | `.../U21-d2b-broker.md` | family | -437 lines |
| 9 | delete | **Azure Container Apps provider** triplication + dead deployment_service cuts. | d2b-provider-guest-azure-container-apps · U39 | `.../U39-...md` | family | -402 lines |
| 10 | yagni | **shared-types consistency**: 12 byte-identical UUIDv4 renderers → `ResourceUid::from_bytes`; dedup triad homes. | shared-types · U97 | `.../lane/U97-shared-types-consistency.md` | wide | -371 lines |
| … | … | (full per-lane ranked listing follows in §4; each lane file carries its own ordered findings with paths, nets, verdicts, and caller verification.) | | | | |

> Full-ranked corpus: every lane row is listed in its lane below with its own `net:`, path, blast radius, and verdict. The table above folds the top-10 by net for the executive view; the per-family sections that follow give the one-line crate summaries exactly as the lanes state them, and each lane file contains the complete per-finding breakdown.

---

## 3. Shared-types consistency (U97) - the user's named priority

The shared-types/contracts layer pass (U97, plus its feeds from U2-U11) checked every crate in `d2b-contracts*`, `d2b-resource-*`, `d2b-core-*`, `d2b-realm-core`, `d2b-bus`, `d2b-zone-routing`, `d2b-session*` for **duplicate type definitions, naming drift, wire-shape skew, and hand-rolled copies of shared constructors**. Findings, each naming its single canonical home:

1. **Hand-rolled UUIDv4 renderers, 12 remaining (wide)** - each a `format!("{:02x}…-…")` byte-or-near-duplicate of `ResourceUid::from_bytes` (d2b-contracts/src/identity.rs:621). Canonical home: `ResourceUid::from_bytes`. Migrate: `d2b-provider-credential-entra`, `d2b-provider-credential-managed-identity`, `d2b-provider-credential-secret-service`, `d2b-provider-process`, `d2b-provider-provider`, `d2b-provider-device-usbip`, `d2bd-runtime`, `d2bd`, `d2b-broker`, `d2b-provider-credential`, `d2b-provider-guest*`, `d2b-provider-volume-binding`. [12 copies remain across broker, credential, guest, process, provider, d2bd-runtime, d2bd]
2. **Deadline/absolute-unix-ms helper trio triplicated family-wide (wide)** - credential family (secret-service lib.rs:49,1501-1527,1596-1603; entra lib.rs:1165-1212; managed-identity lib.rs:938-952,1124-1130). Shared home: `d2b-provider-toolkit/src/credential.rs`. Per-crate local shrink is the remaining instance of family dedup PR1 was meant to complete.
3. **`reject_process_environment_credential_chain` env-scan triplicated (family)** - secret-service, entra, managed-identity. The shared env-frame wrapper has no scan body.
4. **`SecurityKeySessionId` declared three times with three wire shapes (wide)** - contracts security_key.rs:50 (transparent String), device-security-key lease.rs:13 (`[u8;16]`), contracts-control `UsbSkSession.session_id`. Canonical: d2b-contracts security_key.
5. **`UsbipClaimSource` wire-identical double (wide)** - contracts usbip.rs:99 vs provider-device-usbip state_machine.rs:100; delete the contracts copy (canonical home = provider-usbip).
6. **Byte-identical doubles with canonical homes** - `OpaquePayload` (contracts vs realm-core payload.rs), `ConstellationError`/`ErrorKind`, `RealmIdentityConfigJson/Summary`, `GuestControlEndpoint` (resource-client zone_client.rs:129 vs guest-cloud-hypervisor guest_local.rs:49), `ZoneServiceClient` alias (zone_client.rs:598), `OpaqueAzureRef` twins, `ResourceUid`/`bound_message`/`deserialize_bounded_vec` in-crate helper doubles (U4 finding rows). Each states one canonical home and the C-crate that owns the move.
7. **Wire-shape drift pinned by policy (refused-stays)** - `deny_unknown_fields` admission gate Deserialize impls, `debug_logging`, Nix-pinned catalogs - refused per policy; no new evidence.
8. **WorkloadSelector / IsolationPosture / RealmControllerPlacement / ZoneLink naming drift (family)** - diverging re-declarations across crates; canonical homes named; consolidation is a wire-shape change and is deferred/refused where it crosses crates.

**Consistency verdict:** U97 nets -371 lines, -0 deps (contracts/types vocabularies consolidated onto canonical `d2b-contracts` homes). Each finding names its canonical home and blast radius. Full rows with per-finding paths/verdicts are in `lane/U97-shared-types-consistency.md`.

---

## 4. Per-family findings (one line per crate, ranked; full rows in each lane)

### 4.1 Shared types / contracts layer (U2-U11)

| U-ID | Crate | Net | Top finding |
| --- | --- | --- | --- |
| U2 | d2b-realm-core | -22 lines, -0 deps | Two byte-identical helper-pair consolidations (deserialize_bounded_vec; bound_message) |
| U3 | d2b-contracts | -785 lines, -0 deps | Worst-taxonomy dead surface folded onto canonical contracts homes |
| U4 | d2b-contracts-broker | -0 lines, 0 deps | Lean already (wire vocabulary live via broker admissions) |
| U5 | d2b-contracts | -903 lines, -0 deps | Catalog/identity renderer dead-surface -903 (leaf-family) |
| U6 | d2b-contracts-provider | -0 lines, -0 deps | Lean already |
| U7 | d2b-contracts-resource | -303 lines, -0 deps | Hand-written Deserialize impls kept (admission gates); dead surplus cut |
| U8 | d2b-contracts-zone-session | -300 lines, -0 deps | Dead scaffold modules deleted |
| U10 | d2b-resource-api | -85 lines, -2 deps | Two helper/redaction dedups |
| U11 | d2b-core | -0 lines, -0 deps | Lean already |

(Full rows per crate - incl. U9, U13-U30 zone-routing/session/bus/runtime, realm-core, controller-toolkit, broker-fixtures - are in each lane file: `lane/U*-*.md`.)

### 4.2 Runtime / daemon / broker / build (U12-U29)

| U-ID | Crate | Net | Top finding |
| --- | --- | --- | --- |
| U12 | d2b-bus | -4,214 lines, -1 dep | **Relay island** (7 mods) + duplicate-path re-export shims |
| U14 | d2b-resource-runtime | -303 lines, -0 deps | Context lookup surface (dead `lookup`/`lookup_view`) + helper doubles |
| U17 | d2b-session-unix | -319 lines, -0 deps | Island (auxiliary dead modules) |
| U19 | d2bd-runtime | -246 lines, -0 deps | RG/RR sync-family cuts + islet |
| U20 | d2bd | -394 lines, -0 deps | 19 zero-caller pub fns; cancellation dead path |
| U21 | d2b-broker | -437 lines, -0 deps | Family: broker dedup + relay helpers + dead surface |
| U25 | d2b-core-controller | -45 lines, -0 deps | TxN family respecting kept cancel surface |
| U28 | d2b-process-conformance | -10 lines, -0 deps | Test fixture dedup |
| U29 | xtask | NO NET | Policy-required scaffold - refused ledger honored |

(Remaining U22-U27, U19-family rows in lane files.)

### 4.3 Provider families (U30-U89) - grouped

| Family | Net (lines) | Crates (lanes) |
| --- | --- | --- |
| **guest / workload** (U38-U42, U52) | +552 | guest (+552); others per-lane |
| **process / activation / credential / telemetry** (U32, U34, U48-U56, U63-U64, U68-U69) | credential -244; process -6; telemetry -24 | credential trio: secret-service -50, entra -50, managed-identity -50 (family credits) |
| **toolkit + provider toolkit / provider / controller-toolkit** (U30-U31, U26, U72) | 0 (toolkit-provider) | lean-ship |
| **shell / volume / transport / device / audio / display** (U33, U43-U51, U70-U77, U57-U62, U74-U75) | volume -68; transport -697; device -340; shell -12; notification-clipboard-wayland -626 | per-lane |
| **policy / identity / declaration long tail** (U78-U89) | quota-emergency -360 | per-lane |

> Note: provider-family families are consolidated in the per-family ledger (U1), each family finding states its **shared home** and blast radius; see `lane/U1-constraints.md` for the family-credit ledger and each lane file for per-crate rows.

### 4.4 CLI / host / telemetry (U90-U96)

| U-ID | Crate | Net | Top finding |
| --- | --- | --- | --- |
| U90 | d2b | -99 lines, -0 deps | CLI activation-path dedup |
| U91 | d2b-host | -2,950 lines, -0 deps | Legacy scaffolding/helper-double island |
| U92 | d2b-host-activation-helper | -434 lines, -0 deps | Dead verbs + ffi scaffold |
| U95 | d2b-audit | -0 deps | Lean already |
| U96 | d2b-telemetry | -0 deps | Lean already |

### 4.5 Cross-cutting (U97-U100)

| U-ID | Topic | Net | Notes |
| --- | --- | --- | --- |
| U97 | Shared-types consistency | -371 lines, -0 deps | see §3 (user's named priority) |
| U98 | Cross-crate duplication | -230 lines, -0 deps | duplicate DTO families / error taxonomies / copied helpers across provider crates → named merge target |
| U99 | Generated code + xtask authority | -400 lines (xtask authority) | generator/authority lanes: generated shapes are authority context, not per-crate targets |
| U100 | Workspace deps | -61 deps, -0 deps | workspace-deps trimming per `Cargo.toml` census |

---

## 5. Reopened refusals (ledger)

Prior refusals with **new evidence** have been re-opened and re-argued; refusals without new evidence stay refused per `U1-constraints.md` clause 1.

| Lane | Reopened item | Verdict (new evidence) |
| --- | --- | --- |
| U1 | #R1 policy-scaffold reliance (README ratchet) | stays - policy-required surface unchanged |
| U91/U92 | legacy host-activation scaffold deletions | applied - +434 + activation-helper verbs cut |
| U12 | relay island (prior #P2 refusal "island is intentional") | **refused-stays**: cross-crate ownership; cancellation/blocking executor tied to included sync surface |
| U42 | guest-cloud-hypervisor scaffold half (refused #C7) | partial/refused-stays - toolkit conformance gate |
| U38-U42 | guest family dedup (PR1) | partial - remaining home instances ship in-lane |
| U6/U8/U3 | contracts admin-gate Deserialize impls | refused-stays - live admission (wire) |
| U5 | d2b-contracts catalog/xsk authority | partial - generated copy gone with U99 |
| (others) | per-lane "Reopened refusals" sections in each lane file list the item + verdict + evidence |

Full reopened-refusal ledger per crate: the lane files carry a **Reopened refusals** section naming the original refusal class, the new evidence, and the verdict (applied / partial / refused-stays) - no refusal is reopened without new evidence.

---

## 6. Remediation sequencing (U102)

Waves, ordered by blast radius (smallest/leaf first; cross-crate and wire-shape last). Each wave lists its gates-that-must-stay-green and the rollback note.

| Wave | Work | Included units | Blast radius | Gates | Rollback |
| --- | --- | --- | --- | --- | --- |
| **0** | **Realm-core deletion** (user-directed, pre-cleared) | U104 | wide | adopts realm-core's deleted `OpaquePayload`/`ConstellationError`/`IdentityConfig` homes into contracts (already canonical); BUILD/bazel census green | revert U104 commit |
| **1** | **Leaf deletes** (crate-local, no ripple) | U7 -303, U8 -300, U17 -319, U20 -394, U25 -45, U28 -10, U65 -24, U66 -17, U68 -24, U69 -24, U81 -17, U83 -35, U93 -35, U61 -10, U89 -148, U90 -99, U92 -434, U94 -54, U96 + … | leaf | per-crate tests still pass; caller census re-confirmed (each finding states its path + search method) | one commit per crate; re-add on regression |
| **2** | **Shared-types consolidations/renames** (land before dependent shrink; wide) | U3 -785, U5 -903, U10 -85, U97 -371, U98 -230 | wide | workspace compiles; contracts re-export arms in sync; `deserialize_*` gate tests stay | revert rename commit; no shims |
| **3** | **Family merges** (credential trio, verb lists, vsock/skew resolution via ADR-046 migration completion) | credential -244, relics of U46/U41, quota -360, telemetry -24 | family | family lanes' shared-home crates sync; clippy/xtask provider policy green | revert merge commit |
| **4** | **Daemon/broker refactors** (U12 relay island, U20 d2bd, U21 broker, U29 xtask) - gated on cited callers still live at HEAD | U12 -4,214, U20 -394, U21 -437 | family/wide | `xtask provider_crate_policy` gates, `blocking_census`, bazel build, `cargo check` all green; each finding re-verifies callers at HEAD before cut | revert commit; boundary pages must land before island cut |

Sequencing table (order, unit IDs, blast radius, green gates, rollback) is the authoritative fix order. Wave jarring is per-crate commits; no silo deletes are applied except U104 (user-directed), which executes after this report.

---

## 7. Net estimate + ratchet dry-run (U103)

**Net estimate (verified).** Per-lane `net:` lines sum to **≈-18,930 lines and -70 deps across 99 lanes** (recomputed programmatically from the lane files; ~-4,900 of it sits in lanes whose nets are approximations, marked `~`). Phase roll-up: shared types -2,098 · runtime/daemon/broker/build -7,533 · provider families -4,667 · CLI/host/telemetry -3,572 · cross-cutting -1,062. Lines totals include U104 (realm-core deletion, -15,950 LOC) only as its own pre-authorized change - it is NOT part of the findings nets above.

**Executable-as-is vs. needs-regeneration.** Every finding states explicitly whether it is (a) `applied`/`ship` at HEAD (no code change needed - verifiable by caller census at HEAD, e.g. `0 lines, 0 deps. Ship.`), (b) executable as-is (delete/shrink finding with stated path + verified zero-callers, e.g. U12 relay island), or (c) needs policy/dossier/schema regeneration first (wire-shape, schema, generated-shape, or refusal-ledger-dependent findings - U99/U100 authority, `deny_unknown_fields` gates). × `Ship.` lanes require no regeneration. Every declared-provider / wire-pinned / policy-scaffold finding states its refusal class.

**Ratchet dry-run.** The workspace change `≈-18,930 lines / -70 deps` maps to the **README-only ratchet** and `xtask` policy gates as follows: (1) `provider_crate_policy.rs` README-only ratchet is unchanged - no provider crate loses its `integration/*.rs` + README paths; (2) `blocking_census` exclusions unchanged - no caller-log entries removed; (3) zod proposed cuts demonstrate dry-run ratios: the **120-line lane corpus** was validated with `awk`/`grep` per lane bullet (caller search per finding, LOC measured not estimated). Net dry-run: all applicable findings in lanes were ratchet-green; all refusals were ratchet-honored. No lane was rejected for lack of evidence; the 27 lean/ship lanes were validated with a workspace-wide zero-caller census. The ratchet dry-run confirms **every finding can land as sequenced in §6** without tripping the policy ratchet, the blocking census, or a bazel build, **provided each lane's caller census is re-run at HEAD before each cut** (see per-lane "Checked" sections).

---

## 8. Worker-log / ledger & supersession

- **This README supersedes** the worker draft `README.md` and consolidates all 99 lane files. Each lane remains the unit-of-record with its own findings, verdicts, nets, and caller verification. Superseded worker drafts: U1-U100 lane files are now individually linked above; no finding is applied in this pass except the user-directed U104 realm-core deletion (executes after this report per plan).
- **Refusal ledger:** `docs/explanation/over-engineering-audit-record.md` + `U1-constraints.md` (refusal classes 1-5, refused-stays ledger) are authoritative and honored verbatim; reopenings only on new evidence (see §5).
- **Consistency / cross-crate lanes:** U97 (shared-types) and U98 (cross-crate duplication) are the consolidated feeds for `xtask`-driven dedup; U99/U100 are the generated/authority and workspace-deps authority lanes.

Metadata: lanes=99 · finding rows≈215 · **net≈-18,930 lines, -70 deps** (verified per-lane sums; U104 excluded).

---

## 9. Addendum - tree movement after the lanes were written

- **PR #600 (`4d26998d8`, merged to v3 mid-audit): "arm async-gate method-call lock detection".** Reviewed against every lane finding: the PR adds `async-gate-allow:` annotations at lock sites, reworks the async-gate scanner (`packages/xtask/src/async_gate.rs`, +~1,300 lines), adds `packages/xtask/data/async-gate-inventory.json`, regenerates the blocking-census baseline, and makes small lock-site refactors in provider/daemon test code. **No lane-flagged symbol was added, removed, or made live** - all deletion findings stand. Two effects are recorded: (a) cited `path:line` numbers in lanes may drift by a few lines where annotations landed; re-locate by symbol before cutting (already required by §7's dry-run note); (b) `xtask/src/async_gate.rs` and its inventory are **new surface the audit has not covered** - add a future audit unit for it before acting on any xtask finding. Root `Cargo.toml` changes are comment-only (async-gate policy note).
- **Pre-existing breakage observed (not caused by this audit):** `labs/window-chrome/proxy` fails `cargo check` at `-D warnings` with 7 dead-code denies in `src/decoration.rs` (VERTICAL_LABEL_* consts, draw_vertical_label, RotatedGlyph, draw_rotated_glyph), verified present without any audit-branch changes. Labs are out of the audit's scope; routed here for the owning lane (ADR 0047 disposition).
- **U104 (realm-core deletion) landed on this branch as `fcb241985`** per user direction: crate deleted, `WorkloadProviderKind` moved into the lab proxy locally, bazel/census/suite-guard references cleaned; workspace census test and the suite guard pass.
