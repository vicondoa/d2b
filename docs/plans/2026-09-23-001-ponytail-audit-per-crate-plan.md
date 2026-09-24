# Exhaustive Ponytail Audit - Every Crate, Shared-Types Consistency, Remediation Sequencing

Created: 2026-09-23
Tree: branch `v3` @ `3f2664794` (pulled to latest before planning)

## Objective

Run a ponytail-style over-engineering audit across the entire Rust workspace - **one unit per crate, nothing overlooked** - plus a dedicated consistency pass over the shared types layer (`d2b-contracts*`, `d2b-resource-types`, `d2b-resource-api`, `d2b-core`, `d2b-realm-core`). Deliverable: **one consolidated findings report** plus a **remediation-sequencing (fix) plan** inside the same document. Findings are ranked biggest-cut-first; nothing is applied except the one user-directed deletion (U104).

Audit scope of judgment: over-engineering and complexity only (`delete:` / `stdlib:` / `native:` / `yagni:` / `shrink:`). Correctness bugs, security, and performance are explicitly out of scope - route to a normal review pass if spotted.

## Prior art and hard constraints (read before auditing anything)

`docs/explanation/over-engineering-audit-record.md` records a prior 242-finding audit (tree `515cbf610`, far behind current head): 139 applied, 26 partial, 54 refused, 23 not applied. Binding rules for this re-audit:

1. **Refused stays refused** unless this audit finds *new evidence* (e.g. the blocking caller was deleted since). Reopened refusals must cite what changed. Known refusal classes:
   - Policy-required scaffolds: `integration/*.rs` + README paths demanded by `packages/xtask/src/provider_crate_policy.rs` (README-only ratchet).
   - Declared-provider artifacts with zero in-tree callers (e.g. transport-unix, transport-vsock) - pinned by the policy matrix, `nixos-modules/provider-runtime-contracts.nix`, dossiers, and committed schemas.
   - Pinned wire fields / Nix-pinned catalogs (display global catalog, `debug_logging`).
   - Hand-written `Deserialize` impls that are live admission gates (qemu guest/provider spec shapes).
   - Refactors refused for cross-crate ownership (supervisor blocking executor, ZoneLink enrollment-machine merge, host/user driver merge). Re-flag only with a blast-radius statement naming the crates that must move together.
2. **Caller verification is mandatory.** Every zero-caller claim must be backed by a workspace-wide reference search (Rust references *plus* `BUILD.bazel`, `nixos-modules/`, `tests/`, `docs/reference/policy/`). The prior record's caller refutations (`d2bd/src/composition.rs:11508`, `resource_runtime.rs:3216…`) are the dominant false-positive source.
3. **`src/generated/` is out of per-crate scope** (7 crates carry it). Findings about generated shapes go to U99 (generator/authority audit) instead. A per-crate auditor may *cite* generated shapes as evidence but never flags them for deletion.
4. **Labs are out of scope** except where a lab consumes a workspace crate. `labs/window-chrome/proxy` is a standalone workspace whose disposition is ADR 0047's - it stays (user-directed); see U104 for its one intentional edit.
5. `docs/residual-review-findings/`, ADRs (`docs/adr/`), and dossiers named by xtask policy are authoritative context, not audit targets.

## Per-unit contract (applies to every crate unit U2-U96)

Each unit audits exactly one crate and produces a findings block:

- **Format:** one line per finding, ranked biggest cut first: `<tag> <what to cut>. <replacement>. [repo-relative path]`. End with `net: -<N> lines, -<M> deps`.
- **Nothing to cut:** `Lean already. Ship.` plus one sentence naming what was checked.
- **Verify every claim:** caller search per constraint 2; LOC claims measured, not estimated.
- **Exclusions:** `src/generated/`, policy-required scaffolds, refused-ledger items without new evidence.
- **Consistency feed:** contracts/types crates also note divergences (duplicate type definitions, naming drift, wire-shape skew, hand-rolled copies of shared constructors) for U97.
- **Sequencing note:** each finding gets a blast-radius tag: `leaf` (crate-local), `family` (crates within one family), `wide` (contracts/types, daemon, broker - ripples).

## Unit map

94 workspace members + 1 non-member crate (`d2b-realm-core`). LOC are approximate, for sizing only.

### Phase 0 - Priming (serial, before all others)

| Unit | Target | Notes |
| --- | --- | --- |
| U1 | Audit ledger + policy priming | Extract from `docs/explanation/over-engineering-audit-record.md` the refused/partial ledger **per crate**; inventory `src/generated/` dirs; extract xtask policy rules (`provider_crate_policy.rs`, `blocking_census.rs` exclusions, README-only ratchet list). Output: per-unit constraint packets handed to every auditor. Without this packet no crate unit starts. |

### Phase 1 - Shared types layer (audited first; findings ripple widest)

| Unit | Crate | ~LOC | Notes |
| --- | --- | --- | --- |
| U2 | `packages/d2b-realm-core` | 15,950 | Non-member ("retired owner" per `bazel/checks/BUILD.bazel:101`). Sole consumer is `labs/window-chrome/proxy` (uses only `WorkloadProviderKind`, which `d2b-contracts/src/workload.rs:13` also exports). Audit verifies no other consumer; deletion itself is U104 (user-directed). |
| U3 | `packages/d2b-contracts` | 11,509 | Core wire/identity/error vocabulary. Consistency feed primary. |
| U4 | `packages/d2b-contracts-broker` | 5,334 | Includes `src/generated/` (catalog/profiles) - generator findings to U99. |
| U5 | `packages/d2b-contracts-control` | - | |
| U6 | `packages/d2b-contracts-provider` | 14,814 | |
| U7 | `packages/d2b-contracts-resource` | 27,009 | Largest contracts crate; `src/generated/d2b_resource_v3.rs`. |
| U8 | `packages/d2b-contracts-zone-session` | 13,083 | |
| U9 | `packages/d2b-resource-types` | 1,068 | Declaration/metadata vocabulary (`metadata.rs`, `descriptor.rs` verbs). |
| U10 | `packages/d2b-resource-api` | 14,296 | Manager-backend/admission/authz surfaces. |
| U11 | `packages/d2b-core` | 15,848 | Shared app support + `src/generated/broker_operation_authz.rs`, `process_roles.rs`. |

### Phase 2 - Runtime, daemon, broker, build tooling

| Unit | Crate | ~LOC | Notes |
| --- | --- | --- | --- |
| U12 | `packages/d2b-bus` | 25,017 | |
| U13 | `packages/d2b-zone-routing` | 8,265 | |
| U14 | `packages/d2b-resource-runtime` | 15,109 | Shared metadata driver lives here (prior family 1). |
| U15 | `packages/d2b-resource-compiler` | 5,514 | |
| U16 | `packages/d2b-resource-client` | 4,842 | |
| U17 | `packages/d2b-session` | 10,980 | |
| U18 | `packages/d2b-session-unix` | 4,920 | |
| U19 | `packages/d2bd-runtime` | 43,424 | |
| U20 | `packages/d2bd` | 94,493 | Biggest crate. Prior sixth audit covered composition modules; re-audit whole crate at current head. |
| U21 | `packages/d2b-broker` | 79,925 | |
| U22 | `packages/d2b-broker-composition` | 1,642 | |
| U23 | `packages/d2b-broker-fixture-handlers` | 57 | |
| U24 | `packages/d2b-broker-fixture-syscall-surface` | 55 | |
| U25 | `packages/d2b-core-controller` | 13,884 | |
| U26 | `packages/d2b-controller-toolkit` | 191 | |
| U27 | `packages/d2b-provider-test-controller` | 301 | |
| U28 | `packages/d2b-process-conformance` | 4,210 | |
| U29 | `packages/xtask` | 40,186 | Audit the tool, not the policies it enforces; policy-required scaffolds stay. |

### Phase 3 - Provider toolkit and provider families (60 crates)

| Unit | Crate | ~LOC | Unit | Crate | ~LOC |
| --- | --- | --- | --- | --- | --- |
| U30 | `d2b-provider` | 2,585 | U60 | `d2b-provider-device-security-key` | 3,596 |
| U31 | `d2b-provider-toolkit` | 13,712 | U61 | `d2b-provider-device-tpm` | 1,716 |
| U32 | `d2b-provider-activation-nixos` | 2,744 | U62 | `d2b-provider-device-usbip` | 5,085 |
| U33 | `d2b-provider-config-nixos` | 1,530 | U63 | `d2b-provider-observability-otel` | 3,279 |
| U34 | `d2b-provider-audio-pipewire` | 1,965 | U64 | `d2b-provider-process` | 8,836 |
| U35 | `d2b-provider-clipboard-wayland` | 13,487 | U65 | `d2b-provider-host` | 979 |
| U36 | `d2b-provider-display-wayland` | 15,845 | U66 | `d2b-provider-user` | 900 |
| U37 | `d2b-provider-notification-desktop` | 5,313 | U67 | `d2b-provider-endpoint` | 1,927 |
| U38 | `d2b-provider-guest` | 3,481 | U68 | `d2b-provider-telemetry-service` | 943 |
| U39 | `d2b-provider-guest-azure-container-apps` | 2,205 | U69 | `d2b-provider-telemetry-binding` | 1,283 |
| U40 | `d2b-provider-guest-azure-virtual-machine` | 1,882 | U70 | `d2b-provider-volume` | 1,499 |
| U41 | `d2b-provider-guest-cloud-hypervisor` | 7,768 | U71 | `d2b-provider-volume-binding` | 2,388 |
| U42 | `d2b-provider-guest-qemu-media` | 2,688 | U72 | `d2b-provider-wayland-policy` | 1,171 |
| U43 | `d2b-provider-shell-terminal` | 2,764 | U73 | `d2b-provider-wayland-session` | 195 |
| U44 | `d2b-provider-transport-azure-relay` | 4,279 | U74 | `d2b-provider-audio-service` | 122 |
| U45 | `d2b-provider-transport-unix` | 714 | U75 | `d2b-provider-audio-binding` | 170 |
| U46 | `d2b-provider-transport-vsock` | 3,282 | U76 | `d2b-provider-shell-pool` | 172 |
| U47 | `d2b-provider-system-core` | 1,200 | U77 | `d2b-provider-shell-session` | 241 |
| U48 | `d2b-provider-process-systemd` | 1,183 | U78 | `d2b-provider-zone` | 223 |
| U49 | `d2b-provider-process-minijail` | 819 | U79 | `d2b-provider-zone-link` | 3,787 |
| U50 | `d2b-provider-volume-local` | 6,000 | U80 | `d2b-provider-provider` | 2,193 |
| U51 | `d2b-provider-volume-virtiofs` | 1,639 | U81 | `d2b-provider-role` | 269 |
| U52 | `d2b-provider-supervisor` | 4,971 | U82 | `d2b-provider-role-binding` | 40 |
| U53 | `d2b-provider-credential-secret-service` | 3,471 | U83 | `d2b-provider-quota` | 527 |
| U54 | `d2b-provider-credential-entra` | 2,667 | U84 | `d2b-provider-emergency-policy` | 38 |
| U55 | `d2b-provider-credential-managed-identity` | 2,694 | U85 | `d2b-provider-resource-export` | 38 |
| U56 | `d2b-provider-credential` | 2,655 | U86 | `d2b-provider-resource-import` | 38 |
| U57 | `d2b-provider-network-local` | 8,002 | U87 | `d2b-provider-command` | 442 |
| U58 | `d2b-provider-device` | 504 | U88 | `d2b-provider-operation` | 854 |
| U59 | `d2b-provider-device-gpu` | 2,434 | U89 | `d2b-provider-seccomp-profile` | 492 |

Family aggregations from the prior record for refusal-ledger lookups: guest/workload (U38-U42, U52), process/activation/credential/telemetry (U32, U34, U48-U56, U63-U64, U68-U69), shell/volume/transport/device/audio/display (U43-U51, U57-U62, U70-U77, U33), policy/identity/control-plane declaration crates (U78-U89), toolkit/shared runtime (U30-U31, U72).

### Phase 4 - CLI, host, telemetry

| Unit | Crate | ~LOC |
| --- | --- | --- |
| U90 | `packages/d2b` | 19,861 |
| U91 | `packages/d2b-host` | 14,620 |
| U92 | `packages/d2b-host-activation-helper` | 435 |
| U93 | `packages/d2b-unsafe-local-helper` | 3,275 |
| U94 | `packages/d2b-sk-frontend` | 1,123 |
| U95 | `packages/d2b-audit` | 5,299 |
| U96 | `packages/d2b-telemetry` | 1,968 |

### Phase 5 - Cross-cutting passes (after per-crate findings land)

| Unit | Target | Notes |
| --- | --- | --- |
| U97 | Shared-types consistency | Consolidate U2-U11 consistency feeds: duplicate type definitions across contracts crates, naming drift, wire-shape skew, hand-rolled copies of shared constructors (e.g. the remaining 12 `ResourceUid::from_bytes` call sites from prior finding 7), verb-list stragglers. Every finding states the one canonical home. |
| U98 | Cross-crate duplication classes | Duplication that no single-crate unit can own: identical DTO families, parallel error enums, copied helpers across provider crates, test-harness copies. Findings name the merge target crate. |
| U99 | Generated code + xtask authority alignment | Audit the *generators* (`xtask/src/gen_broker_operations.rs`, `operation_row_authority.rs`, `provider_registration_authority.rs`, `resource_type_authority.rs`, `blocking_census.rs`) and the generated-vs-hand-written boundary. Only findings on generator logic land here. |
| U100 | Workspace deps and features | `Cargo.toml` workspace deps: unused/over-broad features, unused crate deps per unit findings, `[patch]`/build-script weight (`libsqlite3-sys` patch, pinned guest lock). |

### Phase 6 - Consolidation and remediation

| Unit | Target | Notes |
| --- | --- | --- |
| U101 | Consolidated report | Single document at `docs/audits/2026-09-23-ponytail-audit.md`: executive summary + global ranked findings table, per-family sections with one subsection per crate (U-ID preserved), shared-types consistency section, generated/authority section, dedup of cross-crate overlaps (a finding surfaced by two units appears once, cross-referenced). |
| U102 | Remediation sequencing (fix plan) | Ordered waves with blast radius: (1) U104 realm-core deletion (user-directed, pre-cleared); (2) leaf `delete:` findings (crate-local, no ripple); (3) shared-types consolidations/renames (wide - land before dependent shrink work); (4) family merges; (5) daemon/broker refactors (U20/U21) last, each gated on its cited callers being still live. Sequencing table: order, unit IDs, blast radius, gates that must stay green (`xtask` policy checks, blocking census, bazel build). |
| U103 | Net estimate + ratchet dry-run | Sum all `net:` lines/deps; dry-run every finding against xtask policy gates and the refused ledger to state which findings are executable as-is vs need policy/dossier updates first. |

### User-directed deletion (executes after the report, per instruction)

| Unit | Target | Notes |
| --- | --- | --- |
| U104 | Delete `packages/d2b-realm-core` | User-directed: crate is unused by the workspace ("retired owner" per `bazel/checks/BUILD.bazel:101`; xtask `blocking_census.rs:1546` already excludes it). Precondition verified in planning: sole consumer `labs/window-chrome/proxy` uses only `WorkloadProviderKind`. **The lab stays.** Move the type definition into the lab proxy crate (it is a small enum; the proxy is a standalone workspace and must not gain a dependency on `d2b-contracts`). Steps: add the enum to `labs/window-chrome/proxy/src/`, swap its imports, drop the path dependency, delete `packages/d2b-realm-core/` + its `BUILD.bazel` entry (`BUILD.bazel:283`, `bazel/checks/BUILD.bazel:101` comment), remove the xtask non-member exclusion (`blocking_census.rs:1546`) if the census allows, update `docs/reference/realm-core.md` (retire or fold). Verify: workspace `cargo check`, lab proxy `cargo check`, xtask gates, bazel targets referenced above. |

## Execution model

- **Waves:** U1 serial → Phase 1 (U2-U11) → Phase 2 (U12-U29) ∥ Phase 3 (U30-U89) ∥ Phase 4 (U90-U96) → Phase 5 (U97-U100) → Phase 6 (U101-U103) → U104.
- Phases 2-4 are independent of each other; all depend on U1's constraint packets. Phase 5 depends on per-crate feeds. One auditor per crate; family lane leads de-duplicate within their family before handing off.
- Auditors are read-only. The only write is U104, executed after the report and fix plan are delivered.
- Scale: ~711k LOC over 95 crates. Findings expected in the hundreds; the per-unit contract keeps every block machine-comparable for consolidation.

## Verification

- U1 packet exists and every crate unit cites it (no auditor skips the ledger).
- Coverage check: the 95 crate units map 1:1 onto `Cargo.toml` members + `d2b-realm-core` (verified by comm-diff during planning); the consolidated report carries a subsection per unit - a missing U-ID fails consolidation.
- Every zero-caller finding in the report shows its reference-search method (paths searched, tool used).
- `net:` totals in U101 reconcile with U103's sum.
- U104 verification: workspace and lab-proxy builds green, xtask policy/census gates green, `grep -r "d2b-realm-core"` returns only historical docs (or none).

## Out of scope

- Applying any finding except U104.
- Correctness, security, and performance issues (route-out to review passes).
- `labs/` (beyond U104's type move), `tests/` host-integration fixtures, nixos-modules, bazel rules - audited only as evidence surfaces.
- `third_party/`, vendored patches, bazel-out symlinks.
