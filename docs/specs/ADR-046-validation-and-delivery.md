# ADR 0046 validation and delivery contract

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-validation-and-delivery` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | ADR 0046 integrator, `xtask` delivery tooling, panel/validator process owners |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-store-redb`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-componentsession-and-bus`, `ADR-046-primitive-resource-composition`, `ADR-046-zone-routing`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-core-controllers`, `ADR-046-resources-network`, `ADR-046-resources-credential`, `ADR-046-provider-state`, `ADR-046-resources-zone-control`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-volume`, `ADR-046-resources-device`, `ADR-046-telemetry-audit-and-support`, `ADR-046-cli-and-operations`, `ADR-046-nix-configuration`, `ADR-046-current-code-migration-map`, every `ADR-046-provider-*` dossier, and the forthcoming `ADR-046-security-hardening`, `ADR-046-streamline`, `ADR-046-reset-and-cutover`, `ADR-046-feasibility-proofs` closing specs |
| Supersedes | This repository's current `AGENTS.md` "Panel review" phase-gate as the *sole* review mechanism for ADR 0046 work (extended, not replaced, per §12); ad hoc per-agent validation ordering for ADR 0046 implementation |

## 1. Purpose and scope

This spec is the normative implementation delivery contract for ADR 0046 /
d2b 3.0. It defines: the exact wave/dependency graph derived from every
`ADR-046-*` spec's declared `Depends on` edges; per-wave entry/exit criteria;
Git Town stack shape and worktree/branch ownership; speculative readiness and
the anti-serialization file-overlap graph; the shared-prep pattern for
contended files; codegen/schema pin management; current-code deletion gates;
the full validation matrix from Tier0 through manual hardware/live/cloud
tiers; the sole heavy-gate mechanism; the immutable candidate snapshot,
validator-evidence, and ten-role panel/attest/seal/eligibility process; PR
opening versus final-lane semantics; merge order; post-wave
branch/worktree/GC cleanup policy; and the release/cutover gate.

This spec is documentation only. It creates no crates, dependencies, Nix
modules, services, controllers, Providers, state stores, CI workflows, or
`xtask` subcommands. Per ADR 0046 decision D024, future W0–W7 implementation
(§3) requires a separate request. This spec is the binding contract that
request must follow; it does not itself begin that work, and no cleanup,
branch deletion, or worktree removal described in §14 is performed by this
change.

## 2. Manifest closure gate (Gate 0 — precondition for any implementation wave)

No `ADR046-W*` implementation wave in §3 may open until **all** of the
following are true, per the parent ADR's "Review and acceptance" contract and
`docs/specs/README.md`:

1. Every file in the `docs/specs/ADR-046-*` manifest — the 24 top-level specs,
   the 27 `docs/specs/providers/ADR-046-provider-*` dossiers, this spec, and
   the four forthcoming closing specs (`ADR-046-security-hardening`,
   `ADR-046-streamline`, `ADR-046-reset-and-cutover`,
   `ADR-046-feasibility-proofs`) — is `Status: Accepted`.
2. `ADR-046-decision-register` has zero rows under "Open decisions."
3. `docs/specs/ADR-046-spec-set.json` and `docs/specs/ADR-046-work-items.json`
   exist, validate against their generator, and enumerate every spec above
   with matching content digests (§8).
4. The ADR/spec PR has both required human review gates from the parent ADR:
   approval before the immutable final panel snapshot, and approval after
   unanimous panel signoff (this is the spec-authoring panel, distinct from
   the per-implementation-wave panel defined in §12).
5. No spec contains an unresolved decision, a missing ResourceType/
   core-controller/Provider dossier, an undefined ref/owner/controller/
   process/state/limit/error/test, a work item without exact source and
   destination paths, or a claim that proposed v3 implementation is already
   live.

Gate 0 is re-evaluated, not waived, if any manifest member changes content
after being marked Accepted (parent ADR: "Any content change invalidates
validation and panel evidence").

## 3. Delivery wave topology

### 3.1 Derivation rule

Each spec's wave is `1 + max(wave(dep) for dep in "Depends on" if dep is an
ADR-046-* spec)`, with `ADR-046-decision-register` at wave floor. This is a
plain topological layering over the exact `Depends on` edges recorded in
every spec's metadata table (verified directly against each file at baseline
`b5ddbed6`, not inferred). Two specs in the same wave are parallel-safe by
*dependency* satisfaction only; §6 additionally requires file-disjointness
before two specs may run in literally concurrent worktrees. Where a spec's
own `Depends on` list is a strict subset of another same-wave spec's
prerequisites, its implementation branch MAY open earlier than its assigned
wave boundary under the speculative-readiness rule in §6 — the wave number
below is the latest-safe placement, not the earliest-possible one.

### 3.2 Wave assignment table

| Wave | Specs (all must be `Accepted`; Gate 0 already covers this) | New/changed crates and modules (destination roots) |
| --- | --- | --- |
| `ADR046-W0` | `ADR-046-terminology-and-identities` → `ADR-046-resource-object-model` → `ADR-046-resource-store-redb` → `ADR-046-resource-api-and-authorization` (serial sub-steps, one integrator branch) | `packages/d2b-contracts/src/v3/{identity,resource_ref,resource,resource_status,resource_schema}.rs`; `packages/d2b-resource-store/`, `packages/d2b-resource-store-redb/`; `packages/d2b-contracts/proto/d2b-resource-v3.proto`; `packages/d2b-resource-api/`; `nixos-modules/{options-zones,resources,index}.nix` |
| `ADR046-W1` | `ADR-046-resource-reconciliation` ‖ `ADR-046-componentsession-and-bus` | `packages/d2b-controller-toolkit/`; `packages/d2b-core-controller/src/{hints,dependencies,owner_reconcile}.rs`; `packages/d2b-contracts/src/v3_component_session.rs`; `packages/d2b-session/`; `packages/d2b-session-unix/`; `packages/d2b-bus/` |
| `ADR046-W2` | `ADR-046-primitive-resource-composition` ‖ `ADR-046-zone-routing` | `packages/d2b-contracts/src/v3/{host,guest,execution_policy,process,volume,user,network,device,credential}.rs`; `packages/d2b-process/`; `packages/d2b-provider-supervisor/`; `packages/d2b-zone-routing/` |
| `ADR046-W3` | `ADR-046-provider-model-and-packaging` (single spec; strictly serial — every downstream Provider dossier depends on it) | `packages/d2b-provider/`; `packages/d2b-provider-toolkit/`; one `packages/d2b-provider-<base>-<implementation>/` skeleton generator |
| `ADR046-W4` | `ADR-046-components-processes-and-sandbox` ‖ `ADR-046-core-controllers` ‖ `ADR-046-resources-network` ‖ `ADR-046-resources-credential` ‖ `ADR-046-provider-state` (five parallel specs) | `packages/d2b-process/`, `d2b-provider-supervisor/` (process effect ports); `packages/d2b-core-controller/`; `packages/d2b-provider-network-local/` schema half; `packages/d2b-provider-credential-*/` schema half; Volume `stateSchema`/`persistenceClass`/`sensitivityClass` extension |
| `ADR046-W5` | `ADR-046-resources-zone-control` ‖ `ADR-046-resources-host-guest-process-user` ‖ `ADR-046-resources-volume` ‖ `ADR-046-resources-device` ‖ `ADR-046-telemetry-audit-and-support` ‖ `ADR-046-cli-and-operations` ‖ `ADR-046-nix-configuration` (seven parallel specs) | `packages/d2b-provider-system-{core,systemd,minijail}/`; `packages/d2b-provider-volume-{local,virtiofs}/` schema half; `packages/d2b-provider-device-*/` schema half; `packages/d2b-telemetry/`, `d2b-audit/`; `packages/d2b/` CLI; `nixos-modules/resources-*.nix` |
| `ADR046-W6` | All 27 `ADR-046-provider-*` dossiers, grouped into five file-disjoint provider families (§3.3) | One `packages/d2b-provider-<base>-<implementation>/` per Provider (27 crates) |
| `ADR046-W7` | `ADR-046-streamline` (final; also requires `ADR-046-security-hardening`, `ADR-046-reset-and-cutover`, `ADR-046-feasibility-proofs` closed) | Cross-cutting friction fixes, reset/cutover mechanics, and the release gate (§15) |

Waves are numbered `ADR046-W0`…`ADR046-W7` — an ADR-046-scoped namespace,
distinct from this repository's general per-plan `Wn` commit-tag convention
in `AGENTS.md`. Commit subjects for ADR 0046 implementation work use
`( ADR046-W<n> )`, `( ADR046-W<n>fu<m> )`, and
`( ADR046-W<n>fu<m> <S><n> )` following the same severity/ordinal grammar
`AGENTS.md` already defines, so existing tooling and human reviewers read one
consistent tag shape.

### 3.3 Wave 6 provider families (file-disjoint parallel tracks)

`ADR046-W6` is the largest wave (27 crates). Every dossier's `Depends on` list
resolves to wave ≤5 prerequisites only (verified per-dossier against the
metadata tables read directly from `docs/specs/providers/*.md`), so all 27
are dependency-parallel. File-disjointness (one crate directory per Provider,
per D012/`ADR-046-provider-model-and-packaging` crate boundary) makes them
worktree-parallel too, grouped into five independently staffed tracks so no
single agent/reviewer owns all 27 at once:

| Track | Providers (crate `packages/d2b-provider-<base>-<implementation>/`) |
| --- | --- |
| System/Host/Guest (7) | `system-core`, `system-systemd`, `system-minijail`, `runtime-cloud-hypervisor`, `runtime-qemu-media`, `runtime-azure-container-apps`, `runtime-azure-virtual-machine` |
| Storage/network/device (7) | `volume-local`, `volume-virtiofs`, `network-local`, `device-tpm`, `device-usbip`, `device-security-key`, `device-gpu` |
| Interaction (5) | `display-wayland`, `audio-pipewire`, `clipboard-wayland`, `notification-desktop`, `shell-terminal` |
| Credentials (3) | `credential-secret-service`, `credential-entra`, `credential-managed-identity` |
| Transport/observability/activation (5) | `transport-unix`, `transport-vsock`, `transport-azure-relay`, `observability-otel`, `activation-nixos` |

Within a track, the 3–7 Providers are further parallel (each is its own
crate, its own `ADR-046-provider-<name>.md` dossier, and its own
`tests/`/`integration/` tree per D059). The only intra-track ordering
constraint is `volume-local` before `volume-virtiofs` (D083: volume-virtiofs
never writes Volume layout/spec/ownership fields, but its controller
`Depends on` `ADR-046-resources-volume` and reads Ready Volume rows created
by volume-local in integration tests) and `network-local` before
`device-usbip` (device-usbip's dossier lists `ADR-046-resources-network` as a
dependency for its firewall/export attachment). Both are soft integration-test
orderings, not authoring blockers — the crates themselves may be authored
concurrently; only their `integration/` scenario tests need the peer Provider
present.

### 3.4 Full dependency edge table

The edges below were read directly from every spec's `Depends on` metadata
row at baseline `b5ddbed6`; they are the source of truth for §3.1's wave
placement and for the speculative-readiness check in §6.

| Spec | Depends on (ADR-046-* only) | Computed wave |
| --- | --- | --- |
| `decision-register` | none | floor |
| `terminology-and-identities` | `decision-register` | W0 (step 1) |
| `resource-object-model` | `decision-register`, `terminology-and-identities` | W0 (step 2) |
| `resource-store-redb` | `terminology-and-identities`, `resource-object-model` | W0 (step 3) |
| `resource-api-and-authorization` | `terminology-and-identities`, `resource-object-model`, `resource-store-redb` | W0 (step 4) |
| `resource-reconciliation` | `resource-object-model`, `resource-store-redb`, `resource-api-and-authorization` | W1 |
| `componentsession-and-bus` | `terminology-and-identities`, `resource-api-and-authorization` | W1 |
| `primitive-resource-composition` | `resource-object-model`, `resource-reconciliation` | W2 |
| `zone-routing` | `terminology-and-identities`, `componentsession-and-bus`, `resource-api-and-authorization`, `resource-object-model`, `resource-reconciliation` | W2 |
| `provider-model-and-packaging` | `resource-object-model`, `resource-api-and-authorization`, `resource-reconciliation`, `primitive-resource-composition` | W3 |
| `components-processes-and-sandbox` | `provider-model-and-packaging`, `primitive-resource-composition`, `componentsession-and-bus` | W4 |
| `core-controllers` | `resource-store-redb`, `resource-api-and-authorization`, `resource-reconciliation`, `provider-model-and-packaging` | W4 |
| `resources-network` | `resource-object-model`, `primitive-resource-composition`, `resource-reconciliation`, `provider-model-and-packaging`, `terminology-and-identities` | W4 |
| `resources-credential` | `terminology-and-identities`, `resource-object-model`, `resource-api-and-authorization`, `resource-reconciliation`, `provider-model-and-packaging`, `primitive-resource-composition`, `componentsession-and-bus` | W4 |
| `provider-state` | `terminology-and-identities`, `resource-object-model`, `primitive-resource-composition`, `provider-model-and-packaging`, `resource-store-redb`, `componentsession-and-bus` | W4 |
| `resources-zone-control` | `decision-register`, `terminology-and-identities`, `resource-object-model`, `resource-api-and-authorization`, `resource-store-redb`, `core-controllers`, `provider-model-and-packaging`, `resource-reconciliation` | W5 |
| `resources-host-guest-process-user` | `decision-register`, `terminology-and-identities`, `resource-object-model`, `resource-api-and-authorization`, `resource-reconciliation`, `components-processes-and-sandbox`, `provider-model-and-packaging`, `primitive-resource-composition`, `core-controllers` | W5 |
| `resources-volume` | `resource-object-model`, `primitive-resource-composition`, `resource-reconciliation`, `provider-model-and-packaging`, `components-processes-and-sandbox` | W5 |
| `resources-device` | `resource-object-model`, `primitive-resource-composition`, `resource-api-and-authorization`, `resource-reconciliation`, `provider-model-and-packaging`, `components-processes-and-sandbox` | W5 |
| `telemetry-audit-and-support` | `terminology-and-identities`, `resource-object-model`, `resource-store-redb`, `componentsession-and-bus`, `core-controllers`, `components-processes-and-sandbox`, `provider-model-and-packaging` | W5 |
| `cli-and-operations` | `terminology-and-identities`, `resource-object-model`, `resource-api-and-authorization`, `provider-model-and-packaging`, `components-processes-and-sandbox`, `componentsession-and-bus` | W5 |
| `nix-configuration` | `terminology-and-identities`, `resource-object-model`, `primitive-resource-composition`, `provider-model-and-packaging`, `core-controllers` | W5 |
| all 27 `provider-*` dossiers | see §3.3; every dossier's deepest edge resolves to a W5 spec (`resources-host-guest-process-user`, `resources-volume`, `resources-device`, `resources-zone-control`, `resources-credential`, `telemetry-audit-and-support`, `cli-and-operations`, or `nix-configuration`) | W6 |
| `security-hardening`, `streamline`, `reset-and-cutover`, `feasibility-proofs` (forthcoming) | the entire manifest (cross-cutting closing review) | W7 |

## 4. Per-wave entry/exit criteria

Every wave (`ADR046-W0`…`ADR046-W7`) uses this template. A wave's exit
criteria are its successor's entry criteria; there is no partial-wave
advance.

**Entry criteria (all required):**

1. Gate 0 (§2) has passed, or — for `ADR046-W1` onward — every spec assigned
   to this wave and every wave before it has a `Merged` implementation state
   recorded in `docs/specs/ADR-046-work-items.json` (§8).
2. Every destination path this wave's work items name (§3.2, §7) is free of
   an open, unresolved contention flag from an earlier wave.
3. The wave's Git Town stack (§5) has been proposed against the exact parent
   commit named in its dependency edges (§3.4), not against a stale `main`.
4. The `cargo xtask heavy-gate` semaphore (§11) is available (not held past
   its 30-minute timeout by a stale prior-wave validation run).

**Exit criteria (all required):**

1. Every spec's work items assigned to this wave show `Validation` evidence
   satisfying §10's applicable matrix rows, imported per §12.2.
2. The immutable candidate snapshot (§12.1) for this wave's integrated tree
   has all required CI, local, and host validator lanes reporting (pending is
   acceptable only while the PR is open, per §13; not at wave close).
3. The ten-role panel (§12.3) has returned unanimous `signoff: true` against
   that exact snapshot, with zero outstanding `recommendations`.
4. `cargo xtask delivery wave seal` (§12.4) has produced a sealed record
   binding this wave's `candidate_id`/`content_id`/`snapshot_sha256`.
5. `cargo xtask delivery wave merge-eligibility` reports eligible for every
   PR in the wave's stack, and each has merged root-to-leaf through GitHub
   (§13).
6. Post-wave cleanup (§14) is recorded as pending for the integrator (not
   necessarily executed before advancing — advancing needs the merge, not the
   worktree teardown).

No wave may begin implementation subagent dispatch before its entry criteria
hold; no wave may be marked delivered before its exit criteria hold. This
mirrors and tightens this repository's existing `AGENTS.md` "Phase gate"
rule (`## Panel review` → `### Phase gate`): where that rule allows a panel
per implementation round, ADR 0046 restricts the **binding** panel to exactly
one occurrence per wave, run only against the wave's single immutable final
snapshot (§12), never against interim implementation rounds within the wave.

## 5. Git Town stack shape and worktree/branch ownership

ADR 0046 implementation is large, cross-cutting, and only partially
file-disjoint by default (`ADR046-W0` in particular is one shared contract
surface). It therefore follows this repository's existing
`AGENTS.md` §"Stacked PR workflow for large waves" as its baseline, tightened
to the exact contract below (adapted, per D001/D041, from the more granular
stacked-wave/anti-serialization workflow already proven on this
codebase's sibling ADR-0045 lineage):

1. **One branch/worktree per file-disjoint slice**, never per person. A
   slice is one row of §3.2/§3.3 that shares no destination path with any
   other concurrently open slice (checked against §7's contention list).
   Branch names are `adr046-w<n>-<slice>`, for example `adr046-w4-network`,
   `adr046-w6-device-tpm`, `adr046-w6-credential-entra`.
2. **Stack only real dependencies.** A slice branch targets `main` if every
   one of its `Depends on` specs already merged to `main`; it targets the
   exact prerequisite PR branch if that prerequisite is still open but
   dependency-complete-enough per §6's speculative rule. `ADR046-W0`'s four
   serial steps are one branch each, stacked linearly
   (`adr046-w0-identities` → `adr046-w0-object-model` → `adr046-w0-store` →
   `adr046-w0-api`), proposed with
   `git town propose --stack --non-interactive --no-browser`.
3. **`ADR046-W1`/`ADR046-W2`/`ADR046-W4`/`ADR046-W5`/`ADR046-W6` parallel
   slices** each branch from the exact merged (or, speculatively, exact
   open) tip of their prerequisite branch and target `main` once that
   prerequisite merges — never targeting an unrelated parallel sibling slice.
4. **The integrator owns**: shared-prep commits (§7), Cargo.toml workspace
   member list and `flake.nix` output additions, `docs/specs/ADR-046-spec-set.json`
   / `ADR-046-work-items.json` regeneration, cross-slice conflict resolution,
   root-to-leaf merge order (§13.3), branch retargeting after a lower PR
   merges, and post-wave cleanup (§14). The integrator is not the default
   implementation sink for any slice that can be assigned independently
   (mirrors the sibling repository's anti-serialization invariant, item 4).
5. **PR bodies** contain only dependency, base/head/tree, `candidate_id`/
   `content_id`, and check-status summaries, per §12.5 — never raw
   validation output, panel transcripts, or AI/tool/model attribution.
6. **Reviewers and panel roles** inspect the plan/diff and supplied evidence;
   they do not re-run tests/builds/evals unless the integrator explicitly
   asks, per this repository's existing `AGENTS.md` panel-prompt rule.
7. Slice worktrees are removed only after their commits are integrated and
   their real `packages/target/` (if any, distinct from the shared-cache
   symlink) is cleaned, per §14.

## 6. Speculative readiness and the anti-serialization file-overlap graph

### 6.1 Speculative-start rule

A slice's implementation branch **may** open before its assigned wave number
in §3.2 closes, provided:

1. every spec it `Depends on` (§3.4) already has `Merged` work-item state
   (not merely "wave complete" — the precise edge, not the coarse wave), and
2. no destination path it will write (per its work items' `Destination`
   field) is currently claimed by another **still-open** branch, per the
   contention index in §6.2/§7.

For example, `resources-network` (computed wave W4) and `resources-credential`
(also W4) may each open as soon as `provider-model-and-packaging` (W3) merges
— they need nothing from `components-processes-and-sandbox` or
`core-controllers`, which are their W4 siblings, not their prerequisites. This
is a positive launch requirement, not merely permission: when a slice's exact
dependency edges are satisfied and its destination paths are uncontended, the
integrator MUST open it in the same coordination cycle as any other
newly-ready slice, mirroring the sibling repository's "Anti-serialization
invariant" item 2 (a ready slice sitting idle without a recorded file-
ownership/tooling blocker is a process failure to correct, not a preference).

### 6.2 File-overlap graph (build procedure)

Before opening any wave's slices, the integrator builds a file-overlap graph:

1. List every work item's `Destination` field for every spec entering that
   wave (§3.2's crate/module column, expanded to the exact paths recorded in
   each spec's "Implementation work items" section).
2. Two slices sharing a destination path are one connected component and
   MUST be either (a) internally ordered within one branch/worktree, or (b)
   split by a shared-prep commit (§7) that lands the contended symbol once,
   after which both slices branch from that prep commit and touch disjoint
   regions of the (now-existing) file.
3. Distinct connected components run concurrently in separate worktrees.
   Partition by actual file paths, never by a desire to avoid all future
   conflict — avoiding possible conflicts is not, by itself, grounds to
   serialize two components that do not share a file.
4. Record the connected-component count, the launched-slice count, and any
   blocked slice with its exact blocker (contended path, missing shared-prep
   commit, or genuine cross-cutting security invariant per §6.3) at wave
   entry and after every panel round. A launch count below the ready count
   without a recorded blocker fails wave entry criteria (§4).

### 6.3 Narrow serial exception

A security-sensitive cross-cutting invariant (for example: the
`AuthenticatedSubjectContext` mapping in `componentsession-and-bus`, or the
bootstrap-authorization narrowing in `resource-api-and-authorization` §"Native
RBAC resources") may stay serial only when the wave plan names the exact
files, the invariant, and the unblock commit. Every downstream slice is
dispatched the moment that unblock commit lands; the exception never
silently expands to cover the rest of the wave.

## 7. Shared-prep pattern and known contention files

The following destination paths are written by more than one spec/work item
and require a shared-prep commit landed on the wave's root branch (by the
integrator, or by whichever slice's work item is listed first below) before
the other claimant's worktree opens:

| Contended path | Claimed by | Resolution |
| --- | --- | --- |
| `packages/d2b-contracts/src/v3/volume.rs` | `ADR046-primitives-001` (base struct), `ADR-046-resources-volume` (full schema), `ADR-046-provider-state` (`stateSchema`/`persistenceClass`/`sensitivityClass`/quota/sealing fields) | `resources-volume`'s base Volume struct lands first (small commit) in `ADR046-W5`; `provider-state`'s extension fields land as an immediate fast-follow commit on the same branch before both fan out to `volume-local`/`volume-virtiofs`/interaction Provider slices in `ADR046-W6` |
| `packages/Cargo.toml` workspace member list | every new crate across `ADR046-W0`–`ADR046-W6` | integrator-only trailing commit per merged slice; never edited inside a slice's own PR diff except to add that slice's own single new member line, which the integrator rebases to the current tail before merge |
| `flake.nix` package/output list | every new Provider crate (`ADR046-W6`) | integrator-only trailing commit, batched per track (§3.3), same rule as Cargo.toml |
| `nixos-modules/index.nix`, `nixos-modules/default.nix` | `ADR046-identities-002` (zones/resources), every `ADR-046-provider-*` Nix authoring section (`ADR046-W5`/`ADR046-W6`) | `ADR046-W0` lands the base zones/resources wiring; each `ADR046-W6` Provider slice appends its own resource-type Nix module import as a single line, rebased by the integrator at merge time, never touching another Provider's import line |
| `packages/d2b-contract-tests/tests/workspace_policy.rs` | every Provider crate-layout assertion (D059/`ADR046-pstate-011`-equivalent gates), one row per Provider | integrator batches one appended assertion per merged `ADR046-W6` slice; a slice's own PR adds only its own assertion function, appended after the current last function, never reordering existing ones |
| `docs/specs/ADR-046-spec-set.json`, `docs/specs/ADR-046-work-items.json` | regenerated after every spec status/work-item-state change (§8) | integrator-only; regenerated and committed as the last commit of each wave, never inside a slice's own PR |
| `packages/d2b-core-controller/src/rbac.rs`, `authz_audit.rs` | `resource-api-and-authorization` (W0-adjacent api-002 work item), `resources-zone-control` (Role/RoleBinding schema), `telemetry-audit-and-support` (audit hooks) | `resources-zone-control` (W5) lands the concrete Role/RoleBinding schema atop the W0 `authz.rs` skeleton; `telemetry-audit-and-support` (W5, parallel) adds only its own `authz_audit.rs` audit-emission hooks, a distinct file, so this is a false-positive overlap once split at the file (not module) level — recorded here so the integrator does not accidentally serialize two already-disjoint files under one shared symbol name |

Any newly discovered contention during wave execution is added to this table
in the same PR that discovers it; the parent decision register process
(`decision-required` protocol in `docs/specs/README.md`) governs disputes
about *how* to split a contended file, not *whether* contention blocks
parallelism (it does, until resolved).

## 8. Codegen/schema pin management

1. `docs/specs/ADR-046-spec-set.json` and `docs/specs/ADR-046-work-items.json`
   are generated indexes (per `docs/specs/README.md`) binding exact member
   files, versions, statuses, dependency edges, content digests, and
   implementation work items. They are generated once the initial member set
   exists (already true — all 24 top-level specs and 27 dossiers exist at
   this baseline) and regenerated as the last commit of every wave in §3.2.
   A wave's exit criteria (§4) include this regeneration; `make test-drift`
   (extended per work item `ADR046-delivery-004`, §17) fails if the committed
   index is stale relative to the specs it indexes.
2. Every ResourceType/Provider spec's committed
   `docs/reference/schemas/v3/<kind>.json` (per `ADR-046-nix-configuration`
   §"Build-time JSON validation") is generated by `xtask gen-schemas` and
   drift-checked exactly as this repository's existing v2 schemas are today
   (`tests/unit/gates/drift-check.sh`, extended to the `v3/` tree). A schema
   change without a matching `apiVersion`/`schemaVersion` bump fails this
   gate, per the existing manifest-contract convention in `AGENTS.md`
   ("Critical subsystems" → "Manifest contract").
3. Generated Nix option types/docs for every `d2b.zones.<zone>.resources.<name>`
   surface are derived from the same `ResourceTypeSchema` and signed Provider
   schema used for build-time validation (per `ADR-046-nix-configuration`);
   `make test-drift` gains one row per `ADR046-W5`/`ADR046-W6` spec asserting
   the two sources never diverge (`xtask gen-nix-options` + `git diff
   --exit-code`).
4. The artifact catalog digest format (`ADR-046-current-code-migration-map`
   §0.2 "Artifact catalog") is pinned once in `ADR046-W0` and never
   re-derived per Provider; every later spec's `d2b.artifacts.<id>` usage
   validates against that one pinned encoding.
5. `packages/d2b-contract-tests/tests/policy_*.rs` gains one policy lint per
   wave for that wave's frozen bounds (Provider/Role/RoleBinding bounds from
   D073; Volume bounds from D062; Device arbitration bounds from D063), so a
   later spec cannot silently loosen an earlier wave's frozen limit without
   a visible policy-test diff.

## 9. Current-code deletion gates

No `RETAIN`/`ADAPT` current-v3 path is deleted merely because a wave's specs
are Accepted; per the parent ADR's "Current-code fit" table and
`docs/specs/README.md`'s work-item field "Removal proof", deletion requires:

1. The exact successor ResourceType/Provider/controller is integrated
   (its own wave's exit criteria, §4, are met) **and** covered by the tests
   named in that work item's `Validation` field.
2. The specific removal-proof test named in the migration map
   (`ADR-046-current-code-migration-map` §8.2, e.g. `systemctl list-units
   'd2b-r-*'` returns empty; `grep -r WorkloadOp packages/ --include='*.rs'`
   returns zero results; `grep -r RelayProvider packages/ --include='*.rs'`
   returns zero results) passes on the candidate snapshot.
3. The deletion is its own commit, tagged with the wave/finding that proved
   the successor, and is never bundled into the same commit that lands the
   successor (so a revert of one does not silently also revert the other).
4. `DELETE`-disposition rows (per the migration map's disposition-code
   table, §0/§1–§9) are deleted only in the wave whose successor spec closes
   them — for example, the per-realm PID1 broker/controller systemd units
   (§5.2 of the migration map) are deleted only after `ADR046-W5`'s
   `resources-zone-control`/`core-controllers` successors are integrated and
   the removal-proof test passes; `d2b-realm-router` session types are
   deleted only after `ADR046-W1`'s `componentsession-and-bus` successor
   routes every v3 peer path.
5. A `REPLACE`-disposition row (e.g. `d2b-realm-router/src/router.rs` →
   Zone-local resource routing) follows the same rule but may retain its old
   file as a dead, test-gated stub for one wave beyond its successor's
   landing if — and only if — a still-open sibling slice's integration test
   fixture references it; the stub's removal is then a follow-up commit in
   the same wave, not deferred indefinitely.
6. This spec's own `ADR046-W7` ("streamline & cutover") is the wave that
   performs bulk final deletion of every remaining `RETAIN`-until-parity row,
   gated by `ADR-046-reset-and-cutover`'s destructive-cutover mechanics and
   by the release/cutover gate in §15. No deletion happens in this
   documentation-only change.

## 10. Validation matrix

Every row below maps to the Layer taxonomy in `tests/AGENTS.md` and
`tests/README.md`. New ADR 0046 test surfaces are added to the **existing**
closed Layer-1 set (nix-unit cases, Rust unit/integration/contract/policy-lint,
flake checks) wherever hermetically possible; Layer-2 tiers (container,
runNixOSTest, live-host, hardware) are used only where Layer 1 provably
cannot cover the behavior, exactly as `tests/AGENTS.md`'s "one rule" requires.
No new top-level `tests/*.sh` gate is added; new coverage extends the
existing orchestrators (`tests/static.sh`, `make test-*` targets) by
manifest/fixture, per that file's closed-set rule.

### 10.1 Tier0 and Layer-1 shards

| Row | Coverage | Tier/Layer | Location |
| --- | --- | --- | --- |
| Tier0 | Syntax + shellcheck for every new/changed shell/doc surface introduced by ADR 0046 tooling (§17) | `make check-tier0` | existing target, no change |
| Layer-1 lint | `cargo fmt`/`clippy` for every new `d2b-*` crate in §3.2/§3.3 | `make test-lint` | existing target, extended by new crate membership |
| Layer-1 rust | `cargo test --workspace` across every new crate, including the three broker feature passes where a new crate touches `d2b-priv-broker` (none does, per D077 — no Provider process imports the broker) | `make test-rust` | existing target |
| Layer-1 proofs | Any new `proofs/` crate for redb/session invariants (only if a wave's feasibility spike needs a separate proof crate; see `ADR-046-feasibility-proofs`) | `make test-proofs` | existing target |
| Layer-1 flake | `eval-*` checks extended with Zone/resource examples once `ADR046-W5`'s `nix-configuration` lands; `examples/minimal` gains a `d2b.zones.dev.resources.*` block | `make test-flake` | existing target, new fixture |
| Layer-1 drift | Schema/Nix-option/spec-set drift gates from §8 | `make test-drift` | existing target, extended rows |
| Layer-1 policy | Workspace-policy, provider-crate-layout, and telemetry/audit-redaction policy lints (§10.4, §10.9) | `make test-policy` | existing target, extended rows |

### 10.2 Rust unit/property/fuzz/fault/conformance

| Row | Coverage | Location |
| --- | --- | --- |
| Unit | Every DTO/schema/validator introduced by `ADR046-object-001/002`, `ADR046-store-001..003`, `ADR046-api-001/002` — canonical JSON, bounds, redaction, unknown-field rejection | `packages/d2b-contracts/src/v3/**` `#[cfg(test)]`, `packages/d2b-resource-store-redb/src/**` |
| Property | Owner cycle/depth/reparent property tests (`ADR046-object-002`); expected-revision conflict storms; watch replay/no-gap; ResourceRef parse/collision vectors (`ADR046-identities-001`) | `packages/d2b-contracts/src/v3/resource_ref.rs`, `packages/d2b-resource-store-redb/tests/` |
| Fuzz | Canonical offer/record fuzzing carried over from main's `d2b-session` Noise vectors (`ADR046-session-001`); redb key/encoding fuzz for `type_index`/`owner_index`/`revision_log` | `packages/d2b-session/tests/noise_vectors.rs` (ported), `packages/d2b-resource-store-redb/fuzz/` |
| Fault injection | Forced crash at every commit boundary (resource-store-redb performance contract fixture list); controller-spawned Process exits unexpectedly → `phase: Degraded`/`Failed` (D059 `tests/` requirement); disconnect/relist/lease-withdrawal | `packages/d2b-resource-store-redb/tests/fault.rs`; every Provider crate's `tests/*.rs` |
| Conformance | `check_provider_conformance` from the toolkit run against every Provider's declared axis; zero `ConformanceError` (D059) | `packages/d2b-provider-toolkit/tests/conformance.rs`, every `packages/d2b-provider-*/tests/` |

### 10.3 Generated schema/Nix parity

Covered by §8's drift gates. Explicit assertions required per wave: rendered
JSON matches `ResourceTypeSchema`; provider-specific `spec.*` matches the
signed Provider schema; inline secret bytes fail the build (credential-ref
marker enforcement); duplicate/undeclared `d2b.artifacts` IDs fail eval;
`artifactId`/`systemArtifactId` type-mismatch fails the build with resource
name, field name, expected/actual type in the error.

### 10.4 redb 10k/RSS/perf

Exact benchmark fixtures from `ADR-046-resource-store-redb`'s performance
contract, run in `packages/d2b-resource-store-redb/benches/`:

| Fixture | Hard target |
| --- | --- |
| Empty store readiness | <=500 ms |
| Aggregate Zone resource service/store + fixed system-core + system-minijail controllers idle RSS | <=64 MiB |
| p95 local Get/bounded List | <=2 ms |
| p95 crash-safe single-resource mutation | <=10 ms |
| p95 durable commit → matching controller handler start | <=5 ms |
| p95 ready Process commit → launch-attempt start | <=20 ms |
| 10,000 resources | list/get/watch fixture — must meet the above p95s under load |
| 100 live watches | fan-out fixture |
| 1/10/100 concurrently ready Process resources | fast-launch concurrency fixture (`ADR046-reconcile-003`) |
| Expected-revision conflict storm | no silent merge; every stale write returns `resource-conflict` with current revision |
| Owner-trigger fan-in/chain | bounded depth/work budget, no amplification |
| Revision compaction | durable floor advances; below-floor cursor gets `revision-expired` |
| Forced crash at every commit boundary | no partial/ambiguous commit observable after recovery |
| Backup/restore/internal schema upgrade | staged validate → atomic publish → rollback-window retention |
| Repeated open/close and long-reader rejection | no reader starves the single writer |

Failure to meet a hard target changes the Proposed design (per the spec);
it is never resolved by weakening durability, authorization, or audit.

### 10.5 Bus/Noise/auth/RBAC/attachment

Ported verbatim (per `ADR046-session-001/002`) from main's
`d2b-session`/`d2b-session-unix` test suites, plus v3-specific additions:

- All Noise NN/KK/IKpsk2 profile-strictness, transcript-mismatch, and
  bootstrap single-use-consumption vectors (11 functions enumerated in
  `ADR-046-current-code-migration-map` §9.1, e.g.
  `fixed_negotiation_and_all_noise_profiles_are_strict`,
  `bootstrap_is_operation_bound_expiring_single_use_and_redacted`).
- SO_PEERCRED/SCM_RIGHTS/pidfd/object-identity/credit tests (8 functions,
  `ADR-046-current-code-migration-map` §9.2, e.g.
  `duplicate_kernel_objects_are_rejected_and_cleaned_up`).
- New v3 additions: `AuthenticatedSubjectContext` mapping tests per evidence
  class (Unix pathname/socketpair, enrolled KK key, bootstrap IKpsk2, native
  vsock); Role/RoleBinding decision-matrix property tests; revocation-latency
  test (relevant resource revision invalidates cache immediately after
  durable commit); parent/child Zone access tests (disconnected-parent local
  intent, reconnect reauthorization against current revision); attachment
  descriptor validation (encrypted, service/method/request/operation/
  generation bound, CLOEXEC, duplicate-object rejection).

### 10.6 EffectPort/broker

- `ProcessLaunchEffectPort` (ProviderSupervisor) ticket verification,
  package/template/resource-output resolution, and identity/pidfd-evidence
  observation tests (`ADR046-process-001`).
- `VolumeLayoutEffectPort`/`VolumeSourceEffectPort`,
  `NetworkEffectPort`/`DeviceEffectPort` — every call carries only opaque
  resource/intent/template/policy IDs; a policy test
  (`packages/d2b-contract-tests/tests/policy_no_raw_host_path_to_provider.rs`)
  asserts no Provider crate source references a raw host path, broker DTO
  type, or `clone3`/`pidfd_open` symbol directly (D077 enforcement).
- Broker remains sole privileged executor: integration test spawns a
  Provider process under a restricted seccomp/namespace profile that has no
  socket to the broker and asserts the effect call still succeeds only
  through the injected EffectPort.

### 10.7 Process/adoption/reap

- system-minijail: `clone3(CLONE_PIDFD)`, cgroup-at-birth placement, adoption
  identity revalidation (pid/start-time/cgroup/executable/template/
  generation) before `pidfd_open`; ambiguity → Unknown/quarantine, never
  broad kill/reuse.
- system-systemd: InvocationID+cgroup+MainPID/start-time binding, pidfd open
  after unit start, no daemonizing/forking unit type, adoption revalidation
  of all stable identity before pidfd open.
- Shared conformance suite run against both Providers asserting identical
  ResourceType/status/error shape (`ADR046-process-002`).
- Restart/adoption integration: `d2bd`-successor restart while a Process
  Provider's children are running; pidfd re-adoption without kill (mirrors
  this repository's existing `KillMode=process` continuation-event
  invariant in `AGENTS.md`).

### 10.8 State/Volume/ACL/marker/quota/migration

- Volume layout/ACL/no-follow/inheritance/repair/cleanup tests carried
  forward from current storage-lifecycle coverage (`ADR046-primitives-003`),
  plus new: 1,024 layout-entry / 64-view / 64-attachment bound tests (D062);
  `sourcePolicyId` opacity test (raw host path never in spec/status/audit,
  never reaches the Provider process as a literal path — D082).
- Provider state Volume tests: a component receives a state Volume **only**
  when it declares one under the storage-need test (D087); a stateless
  component declares none and receives none, and no empty identity-only Volume
  exists (revised D076); each declared state Volume has a `User/<name>` layout
  principal from a bounded Nix-preprovisioned pool; no
  cross-component/cross-Provider mount sharing (D076/D079 enforcement); status
  bound/schema/conformance tests (total canonical status ≤ 64 KiB,
  provider-specific detail ≤ 32 KiB, condition/list/map cardinality caps,
  `status-oversize` rejection) and status-first restart-revalidation tests
  (controller re-derives observed state from status/core ledger/external
  observation and never treats status as authority); optional-state admission
  tests asserting an unjustified namespace is rejected `component-state-not-
  justified`; no bootstrap state Volume or bootstrap-storage mechanism exists —
  fixed bootstrap components reach Ready from status/the core Operation ledger
  (D086, superseded by D087), and a Guest still bootstraps its own Guest-local
  `volume-local` without a parent-Host dirfd leak.
- Three-layer status shape tests (D088): every resource carries the universal
  `ResourceStatus` base; the ResourceType-common `status.resource` schema
  validates provider-neutral; the optional `status.provider` extension carries
  `providerRef`/`schemaId`/`schemaVersion`/`observedProviderGeneration`/`details`.
  Schema-parity/conformance across implementations: for each multi-implementation
  ResourceType (Guest, Device, Credential, Volume, Process) assert every
  implementation populates the same `status.resource` fields with identical
  shape, and no shared field is duplicated in any `status.provider`
  (`status-provider-overlap` rejection). Provider-extension tests: unregistered
  `schemaId`/`schemaVersion` → `status-provider-schema-invalid`; unknown field in
  `details` → rejected; version mismatch (installed Provider vs written
  extension) → rejected; `status.provider.details` over 32 KiB or over cardinality
  → `status-oversize`; `status.provider` restating a universal/`status.resource`
  field → `status-provider-overlap`. Base-only projection/watch compatibility:
  a generic consumer that requests only the universal base + `status.resource`
  reads and watches successfully with `status.provider` absent, unknown, or from
  a different Provider version, and never parses `details`. Atomic layered write:
  all present layers are committed in one status mutation with one expected
  revision; a partial-layer write is rejected.
- Three-layer spec shape tests (D089): every resource carries the universal
  envelope and a ResourceType base spec at `spec.*` (including `spec.providerRef`);
  the optional canonical `spec.provider = { schemaId, schemaVersion, settings }`
  is the only provider-extension shape (no `providerSettings`/Device `settings`/
  ad hoc extension survives). Base-schema parity across implementations: for each
  multi-implementation ResourceType (Guest, Device, Credential, Volume, Process)
  assert every Provider `ResourceApiBinding` implements the identical base spec
  schema version/fingerprint and accepts the canonical minimal valid base Spec.
  Capability-declared optional rejection: a Provider whose signed capability
  matrix marks an optional base capability unsupported returns provider-neutral
  `unsupported-capability`; a Provider that ignores/reinterprets/renames/
  duplicates/weakens a base field, or requires `spec.provider` for base-required
  behavior, fails conformance. Extension tests: unregistered/version-mismatched
  `spec.provider.schemaId`/`schemaVersion` → `spec-provider-schema-invalid`;
  unknown field in `settings` → rejected; `settings` restating a base field →
  `spec-provider-shadow`; `spec.provider` validated against `spec.providerRef` at
  both Nix build and API admission; over-limit envelope → spec bounds error.
  Spec+status atomic schema binding: the selected Provider's registered
  `spec.provider` and `status.provider` schemas align, and a binding missing
  either base fingerprint is rejected. Generic CLI/controller base-only tests:
  author and reconcile succeed on base spec + base status with `spec.provider`/
  `status.provider` absent or from a newer Provider version.
- Expedited reconcile tests (D090): commit-fails/`Abort` → no external effect;
  controller finishes preflight before commit but gates all effects on
  `CommittedRevisionProof`; effects-gate (no finalizer release / status write
  pre-proof); status-write-delayed → response carries `statusPersistence:
  pending` and last persisted status revision; normally-queued reconcile
  no-ops/rejoins after the expedited pass (idempotency key from
  UID/generation/revision/operationId; no duplicate effect); concurrent
  mutation conflict handling; delete returns event-only Deleted projection /
  not-found; expedited timeout → committed-but-reconcile-pending with the queue
  continuing; restart mid-expedited → no duplicate effect; `expedited-not-
  authorized`/`expedited-quota-exceeded` enforcement and priority-lane fairness.
- Currency and disruptive-upgrade tests (D091): `status.update` reports
  `Current` when converged; a non-disruptive change reconciles without
  `UpgradeRequired`; each trigger (`CoreGenerationChanged`,
  `ProviderGenerationChanged`, `ArtifactChanged`, `ImageOrSystemGenerationChanged`,
  `SpecChanged`, `DependencyChanged`, `SecurityPolicyChanged`) drives the correct
  state/reason; dependency propagation aggregates owned/dependency currency for
  list/get; GPU dependency blocking → `Blocked`/`UpgradeRequired`, planner drains
  dependent Processes/Guests, recycles the GPU realization, restarts dependents;
  state-preservation (durable/state Volumes) and TPM-identity preservation across
  upgrade/recycle; `Replace` only with ownership/state transfer; crash/re-entry
  resumes from the core Operation ledger without duplicating; per-resource
  single-flight serializes reconcile-vs-upgrade; CLI projections (`list
  --updates`, `get`, `upgrade` plan-by-default vs `--recursive --apply`) with
  stable `--json`; `spec.updatePolicy` manual-disruptive default; `spec.provider`
  cannot bypass disruption policy.
- Endpoint resource tests (D092): endpoint ownership/lifecycle (owned `Endpoint`
  child, `producerRef` set, child-first deletion with the producer/owner); no
  raw locator in Endpoint spec/status/CLI (only closed
  class/transport/locality/purpose + bounded fingerprints); ref resolution (a
  consumer `Endpoint/<name>` ref gains a dependency edge and resolves to a
  private transport/FD only via EffectPort/LaunchTicket); provider base-schema
  conformance for the Endpoint ResourceType; endpoint update/recycle (recycle
  with producer); producer restart bumps `endpointGeneration`/`status.update`;
  consumer dependency trigger fires on that bump; unauthorized resolve →
  `endpoint-resolve-denied` with no locator; CLI update visibility (`get
  Endpoint/<name>` shows readiness/currency, no locator); high-churn-handle
  non-resource lint asserts pidfd/fd-index/named-stream/`OwnedTransport`/
  `operationId` are NOT promoted to resources and stay internal; the
  `ProcessSpec`-has-no-inline-`endpoints` lint; standard ResourceType count is 17
  (Endpoint present in the catalog); every retained public `*Id`/`Handle` has a
  documented rationale row.
- Entra identity-Guest login tests (D093), all against a **fake Entrablau Guest
  login service** (no live Entra in CI; a manual real-Guest login is a separate
  non-CI check): fake Guest login success → `interactionState: Authenticated`
  and a subsequent on-demand access-token lease delivered end-to-end (KK) only to
  the exact `consumerRef` consumer (Host/bus see ciphertext); login-required →
  `interactionState: Required`/`AwaitingUser`; cancel (`CancelLogin`) →
  `login-cancelled`; timeout past `loginDeadline` → `login-deadline-exceeded`
  with the durable Credential unchanged; controller/agent restart mid-login
  resumes/re-derives without leaking secrets; login `Endpoint` unavailable or
  generation mismatch → typed error, no token; Host placement of the
  login/token service rejected (`host-placement-rejected`); token/refresh-state
  redaction (no token, URL, cookie, authority-conferring device code, or user
  PII in status/audit/OTEL); end-to-end record only (intermediate controllers
  observe ciphertext); same-Zone `identityGuestRef`/`loginEndpointRef`/
  `consumerRef` accepted and cross-Zone rejected; Nix composition validation
  (identity Guest composes `inputs.entrablau.nixosModules.default`, login
  Endpoint bound, Credential fields same-Zone-consistent, no store path/token);
  identity-Guest TPM/login-state preserved on reset unless explicitly destroyed;
  ACA and Azure VM consumers obtain their access token over KK from the identity
  Guest; `credential-entra` controller does no direct Entra network egress.
- Quota/EmergencyPolicy tests: hierarchical Host/Guest allocation against
  Zone capacity, overcommit blocking, digest/Provider/Host/Guest/Zone/global
  emergency disable and route/session/grant revocation, incident-held
  Volume/Provider state preservation.
- Migration: every `RETAIN`/`ADAPT`/`REPLACE` row in the migration map has a
  test asserting the v3 successor round-trips the retained semantic (e.g.
  `unsafe_local_posture_round_trips_as_closed_typed_fields`-equivalent test
  for the user-only Host no-isolation posture, D042).

### 10.9 Network/Device/credential/interactions

- Network: bridge/namespace/address/DHCP/DNS/NAT/firewall/egress
  reconciliation tests for `network-local`; Host/Guest attachment tests.
- Device: first-probe-failure → `Unknown`, absent-after-3-failures → cleared
  (D063); render-node sharing vs. full/VFIO exclusive arbitration; USBIP
  firewall/export tests; security-key unprivileged relay/frontend tests
  (fixed broker only opens/passes hidraw, never more).
- Credential: raw-token/SignChallenge delivery only over a dedicated
  Noise\_KK end-to-end sensitive ComponentSession (D055/D056/D068); NN/
  IKpsk2 rejection tests for token delivery; zero-token-bytes-in-audit/
  telemetry/store policy lint.
- Interactions (display/audio/clipboard/notification/shell-terminal): one
  ResourceType-conformance + fault-injection pass per Provider dossier,
  per D059's mandatory `tests/` tree.

### 10.10 Container integration and host runNixOSTest KVM/TCG

| Row | Coverage | Command |
| --- | --- | --- |
| Container | Provider controller against a real Zone runtime in a container; resource lifecycle under real broker calls; cleanup-contract scenarios (§0.2 of the migration map) | `make test-integration` (podman; local host/manual pre-PR, per this repo's existing tier) |
| Host runNixOSTest, KVM | Live daemon/broker/socket-activation/host-posture/kernel behavior for every new Zone runtime, Provider process, and cleanup/rollback scenario | `make test-host-integration` (x86_64-linux, KVM) |
| Host runNixOSTest, TCG fallback | Same suite when `/dev/kvm` is absent — slower, still required before Wave exit for waves touching kernel-adjacent behavior (process/adoption, cgroup, virtiofs) | `make test-host-integration` (TCG fallback path, already documented in this repo's `AGENTS.md`) |

### 10.11 Hardware/live/cloud manual

| Row | Coverage | Command |
| --- | --- | --- |
| Hardware | Real GPU/YubiKey/hardware-TPM passthrough for `device-gpu`, `device-security-key`, `device-tpm` | `make test-hardware`, manual, on a host with the devices |
| Live-host | Destructive/stateful checks against a real deployed Zone (store adoption, restart/power-loss, USBIP guestd lifecycle equivalents) | `D2B_LIVE=1 bash tests/integration/live/<name>.sh`, manual, never CI |
| Cloud | `runtime-azure-container-apps`, `runtime-azure-virtual-machine`, `transport-azure-relay`, `credential-managed-identity`, `credential-entra` against real Azure resources | manual tier, gated by `ADR-046-feasibility-proofs`; never run in CI or as a required wave-exit lane — recorded as external evidence only |

### 10.12 Restart/power-loss

- Zone runtime restart: relist owned resources, resume from durable
  checkpoints, revalidate Provider/controller leases, preserve
  Unknown/ambiguous states, no cleanup before owners observe/adopt
  (`ADR-046-core-controllers` "Restart" section).
- Power-loss/forced-crash: covered by §10.4's "forced crash at every commit
  boundary" fixture plus a runNixOSTest scenario that `kill -9`s the Zone
  runtime process mid-write and asserts redb recovers to the last durable
  commit with no torn write observable.
- Store identity fail-closed: a previously provisioned database that is
  missing, replaced, bound to another Zone/UID, newer than the binary
  schema, or internally inconsistent fails closed — never silently
  recreated (`ADR-046-resource-store-redb` "Store identity").

### 10.13 Reset/cutover

Deferred in content to `ADR-046-reset-and-cutover` (forthcoming, required
before `ADR046-W7` per §3.4); this spec fixes only the **gate shape**:
`ADR046-W7`'s exit criteria (§4) additionally require every destructive-reset
test named by that spec to pass on the `ADR046-W7` candidate snapshot before
the release/cutover gate (§15) opens.

### 10.14 CLI/Nix examples/docs

- Every `d2b <verb>` surface in `ADR-046-cli-and-operations` gets one
  `packages/d2b/tests/*.rs` hermetic test (`CARGO_BIN_EXE_*`,
  `D2B_PUBLIC_SOCKET` pointed at a fixture) plus one non-TTY-JSON-output
  test.
- `examples/minimal`, `examples/multi-env`, and `templates/default` gain a
  `d2b.zones.<zone>.resources.*` block once `nix-configuration` lands
  (`ADR046-W5`); `flake.checks.<system>.eval-*` extended accordingly (§10.1).
- Every Provider dossier's required Nix-authoring section (minimal Nix
  snippet + exact rendered canonical JSON, per `docs/specs/README.md`
  "Evidence and current-code fit") is itself a drift-checked fixture: a
  contract test in `packages/d2b-contract-tests/tests/` renders the snippet
  and diffs it against the dossier's committed JSON block.
- `docs/reference/*` pages affected by ADR 0046 land in the same wave as
  their implementation, not deferred — per this repository's existing
  `AGENTS.md` docs-review-role expectation ("Diataxis adherence... AGENTS.md
  updates landing with load-bearing changes").

### 10.15 Mocking shared dependencies for parallel safety

Every wave-parallel slice (§3.2, §3.3) hermetically mocks its not-yet-landed
or concurrently-in-flight peers so Layer-1 tests never require a second
slice's crate to exist:

- Resource store/API: `packages/d2b-provider-toolkit` ships fake
  core/store/bus/supervisor/effect clients (per `ADR-046-provider-model-and-packaging`
  "Toolkit"); every Provider crate's `tests/` tree uses these fakes, never a
  real Zone runtime.
- ComponentSession/bus: an in-process loopback transport substitutes for a
  real Unix/vsock/relay transport in unit/integration tests; only
  `integration/` and host-integration tiers use a real transport.
- EffectPort: `FakeProvider`/fake effect adapters from the conformance kit
  stand in for ProviderSupervisor and the volume/network/device effect
  adapters in every Provider crate's `tests/`.
- Cross-Provider dependencies (manifest `dependency` aliases, e.g. a Guest
  runtime Provider's `volume` alias) resolve to a fake alias target in
  `tests/`, and only to a real installed Provider resource in `integration/`.

This is what makes §3.3's 27-way parallel wave safe: no Provider crate's
hermetic test suite requires another Provider crate to be compiled, merged,
or even to exist yet.

## 11. Heavy-gate: sole use

Every Layer-2/hardware/live/perf-heavy command anywhere in ADR 0046
delivery — `make test-integration`, `make test-host-integration`,
`make test-hardware`, `D2B_LIVE=1 bash tests/integration/live/*.sh`, the
redb benchmark suite (§10.4), and any cloud-tier manual run (§10.11) — MUST
run only through one shared semaphore:

```bash
cargo xtask heavy-gate -- <command> [args...]
```

This is adopted by copy/adapt (per D001/D041) from the equivalent tooling
already proven on this codebase's sibling ADR-0045 lineage: a two-slot
per-UID OFD-locked semaphore under
`${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/d2b-heavy-gates`, nonblocking
acquisition retried every 250 ms for up to 30 minutes, fail-closed (no
`flock` fallback) on unsupported locking or timeout, with the child process
receiving a duplicated locked-FD handle and the wrapper retaining
group-signal/reap ownership exactly as documented. Building this tool (if
not already present at ADR 0046 implementation time) is work item
`ADR046-delivery-001` (§17); every wave's `make heavy-check`,
`make heavy-test-integration`, `make heavy-test-host-integration`, and
`make heavy-test-hardware` targets route through it. "Sole use" means: no
wave, no Provider crate, and no panel/validator role may create a second
ad hoc lock file, a bespoke sleep-and-retry loop, or a per-crate heavy-lane
guard — every heavy lane in every wave shares the exact same two slots, so
concurrent waves' heavy validation cannot silently oversubscribe the shared
Nix store, cargo target, or KVM device.

## 12. Candidate snapshot, validator evidence, ten-role panel, attest/seal/eligibility

### 12.1 Immutable candidate snapshot

For each wave (§3.2), once its stack of PRs is open and has passed the
smallest focused local preflight (`make check-tier0` plus the wave's
directly affected `make test-*` shard), the integrator creates one immutable
snapshot binding:

- the exact base commit and every open PR's head commit in the wave's stack;
- the wave's dependency graph edges (§3.4) and repository set;
- a `content_id` (digest of the wave's integrated tree) and a `candidate_id`
  (digest of `content_id` + dependency graph + repository set);
- a `snapshot_sha256` covering the same inputs, byte-for-byte.

This mirrors `cargo xtask delivery wave help`'s `snapshot` subcommand as
already specified for this codebase's sibling ADR-0045 lineage (built here as
work item `ADR046-delivery-002`, §17, by copy/adapt per D001/D041). Any
content change after the snapshot — including generated output, dependency
metadata, contract fingerprints, or repository-set membership — invalidates
both validator and panel evidence; the wave re-snapshots and both lanes
rerun. A history-only rebase or retarget may reuse panel evidence only when
the canonical proof tool (§12.6) verifies byte-identical integrated content.

### 12.2 CI/local/host validator evidence

Three lanes run concurrently against the exact snapshot, never sequentially
gating each other:

1. **Required GitHub CI**: the existing Layer-1 `check` rollup
   (`.github/workflows/pr-l1-static-fast.yml`, generated from
   `tests/layer1-jobs.json`), extended with the ADR 0046 rows from §10.1.
2. **Final local/host validators**: `make test-integration`,
   `make test-host-integration`, and (for waves touching device Providers)
   `make test-hardware`, all wrapped in the heavy-gate (§11), run by the
   integrator on the development host and imported as evidence via
   `cargo xtask delivery wave validate-import` (built as work item
   `ADR046-delivery-003`, §17).
3. **The ten-role panel** (§12.3), run against the same snapshot.

A pending lane is valid only while the wave's PR stack is open (§13.1); it
never permits merge (§13.3). Command/result evidence is imported into an
external, candidate-ID-addressed state directory — never committed to Git,
copied into generated artifacts, or pasted into a PR body (§12.5).

### 12.3 Ten-role final panel, bound to Gemini 3.1 Pro

Every ADR 0046 wave's binding panel — run exactly once, at wave close,
against the wave's one immutable snapshot, never per implementation round —
uses this repository's existing ten-role default roster (`AGENTS.md` →
"Panel review" → "Default panel"), with every role's provider/model bound by
the wave's `panel-request` record to:

```text
provider: github-copilot
model_version: gemini-3.1-pro-preview
```

| Role | Focus (unchanged from this repository's existing default panel) |
| --- | --- |
| `software` | Shell + Nix shape, daemon instrumentation, sidecar idempotency, metric-exporter error handling |
| `test` | New option-schema coverage, restart-policy gates, manifest/schema drift, invisible-regression risk |
| `nixos` | Module wiring, `lib.mkForce`/`lib.mkDefault` correctness, option declarations, activation ordering |
| `networking` | Network surface changes, firewall posture, DHCP/DNS regressions, bridge isolation, routing invariants |
| `security` | Attack surface, capability sets/syscall filters, authz boundaries, PII/telemetry-label review, retention |
| `rust` | API shape, error propagation, unsafe/FFI boundaries, schema generation, workspace-dependency direction |
| `product` | Operator UX, naming surface, migration/deprecation policy, actionable errors |
| `docs` | Diataxis adherence, CHANGELOG, schema md↔json drift, AGENTS.md updates landing with load-bearing changes |
| `observability` | Metric-label cardinality, span-attribute hygiene, log/audit shape, retention, exporter correctness |
| `kernel` | pidfd, cgroup, namespace, mount, signal, ioctl, filesystem semantics; kernel-version assumptions |

`cargo xtask delivery wave panel-request` writes the candidate-bound request
(binding `candidate_id`/`content_id`/`snapshot_sha256`, the exact ten-role
roster, and the required `gemini-3.1-pro-preview` model). `cargo xtask
delivery wave panel-attest` validates a directory containing exactly one
record per role, each shaped exactly as this repository's sibling
ADR-0045-lineage panel-receipt artifact:

```json
{
  "artifact_kind": "d2b-delivery/panel-receipt",
  "schema_version": 1,
  "role": "software",
  "candidate_id": "<sha256>",
  "content_id": "<sha256>",
  "snapshot_sha256": "<sha256>",
  "model_version": "gemini-3.1-pro-preview",
  "provider": "github-copilot",
  "run_id": "run-001",
  "receipt_locator": "github-copilot://runs/run-001/software",
  "output_sha256": "<sha256>",
  "signoff": true,
  "recommendations": []
}
```

`signoff` is `true` iff `recommendations` is `[]`; any finding requires a
content change, which creates a new snapshot and invalidates every prior
validation/panel record for that wave. Green tests never waive this gate —
every wave, including a documentation-only or single-crate wave, requires
unanimous 10/10 signoff before its exit criteria (§4) are met. Building this
tooling (if not already present) is work item `ADR046-delivery-004`/`-005`
(§17), copy/adapted from the equivalent sibling-lineage tooling per D001/D041.

### 12.4 Seal and merge eligibility

`cargo xtask delivery wave seal` requires all ten panel records
present, unanimous, and bound to the same `candidate_id`/`content_id`/
`snapshot_sha256`, plus every §12.2 validator lane reporting success on that
exact snapshot. `cargo xtask delivery wave merge-eligibility` then confirms,
per PR in the wave's stack: the seal exists, the PR's current base/head
still matches the sealed snapshot's recorded OIDs (or a history-only rebase
has passed the byte-identical proof in §12.6), and every required GitHub
check is green.

### 12.5 No raw evidence or AI metadata in Git/PR

Validation command output, panel transcripts, and attestation payloads
never enter Git, generated source, a PR body, or a release archive. PR
bodies carry only: dependency list, base/head/tree OIDs, `candidate_id`/
`content_id`, and check-status summaries, with optional links to external
evidence. No PR description, commit message, CHANGELOG entry, or shipped doc
names or lists the AI agent, assistant, tool, or model used to author or
review the change (per this repository's existing `AGENTS.md` "AI/tool
attribution" rule, extended here to also cover panel attestation records —
the panel's own `model_version`/`provider` fields exist only inside the
external, non-Git delivery-state directory, never inside a committed file).

### 12.6 Content invalidation and byte-identical history proof

Any content change to the wave's integrated tree — including generated
output, dependency metadata, contract/index content, or repository-set
membership — invalidates both the validator and panel lanes for that wave.
The wave re-snapshots (§12.1) and reruns both lanes. A history-only rebase or
retarget (no tree content change, only a new base commit) may reuse prior
panel records only when a canonical proof tool verifies:

- byte-identical integrated content across the old and new history;
- byte-identical generated artifacts (schemas, `spec-set.json`/
  `work-items.json`, Nix-rendered fixtures);
- byte-identical dependency diff and repository set.

Required CI still reruns on the new history even when the proof succeeds
(the proof only preserves the *panel* record, never the CI requirement).
This tool is work item `ADR046-delivery-006` (§17), copy/adapted from the
sibling-lineage history-proof tool per D001/D041.

## 13. PR opening vs. final lanes; merge order

### 13.1 PR opening

After a slice's candidate passes the smallest focused local preflight
(`make check-tier0` + its directly affected `make test-*` shard), the
integrator immediately opens or updates its PR and creates the wave's
immutable snapshot (§12.1) from that exact open-PR/stack state — it does not
wait for `make test-integration`, `make test-host-integration`, or the panel
to finish first. This is the same ordering this repository's existing
`AGENTS.md` "Landing changes (PR workflow)" section already requires; ADR
0046 adds only the explicit snapshot-creation step at that same point.

### 13.2 Final lanes

GitHub CI, the local/host validators, and the ten-role panel then run
concurrently against that exact snapshot (§12.2/§12.3). The PR may report
these lanes as pending while it stays open; pending is never sufficient to
merge (§13.3).

### 13.3 Merge order

1. Merge proceeds only after `cargo xtask delivery wave merge-eligibility`
   reports eligible (§12.4) for every PR in the wave's stack.
2. Merges follow the wave's Git Town stack root-to-leaf: `ADR046-W0`'s four
   serial steps merge in their stacked order; a wave's parallel slices
   (§3.2/§3.3) merge in any order relative to each other (they are
   file-disjoint by construction, §6), but every slice depending on another
   still-open slice (the intra-track ordering in §3.3) merges after its
   prerequisite.
3. The integrator retargets/rebases any still-open dependent PR immediately
   after its prerequisite merges, and reruns only the smallest relevant
   validation for that retarget (full re-panel only if content changed, per
   §12.6).
4. A wave does not advance to the next wave's entry criteria (§4) until
   every PR in its own stack has merged through GitHub — never through a
   local octopus merge or a direct push to `main` for ADR-scale work
   (per this repository's existing `AGENTS.md` "Finish-of-work invariant"
   and "Stacked PR workflow" sections, which remain binding for ADR 0046).

## 14. Post-wave cleanup (policy only — no deletion performed by this change)

After each wave's every PR merges (§13.3), the integrator:

1. Deletes the merged remote feature branches for that wave's slices.
2. Cleans any slice worktree's real `packages/target/` (if it is not the
   shared-cache symlink) before removing the worktree.
3. Removes the finished local worktrees and deletes their local branches.
4. Runs `nix-collect-garbage` and verifies `git worktree list` contains only
   active work for the next wave.
5. Confirms no abandoned/superseded branch is silently dropped — any
   worktree branch whose tip is unmerged but represents abandoned work is
   flagged for the operator, per this repository's existing `AGENTS.md`
   worktree-audit rule.

This is a policy statement only. This documentation-only change creates no
worktrees, branches, or generated artifacts, so no cleanup step above is
executed as part of landing this spec.

## 15. Release/cutover gate

`ADR046-W7` ("streamline & cutover") does not close, and d2b 3.0 does not
release, until all of:

1. `ADR-046-streamline`, `ADR-046-security-hardening`,
   `ADR-046-reset-and-cutover`, and `ADR-046-feasibility-proofs` are
   `Accepted` and their own work items' `Validation` evidence is imported
   per §12.2.
2. Every `DELETE`/`REPLACE` row in `ADR-046-current-code-migration-map` has
   satisfied its removal-proof test (§9) on the `ADR046-W7` candidate
   snapshot — this is the destructive-cutover gate; d2b 3.0 does not ship
   with both the v3-pre-ADR-0046 code path and its successor coexisting
   indefinitely.
3. The `ADR046-W7` snapshot has passed §10's complete matrix, including the
   manual hardware/live/cloud tiers (§10.11) at least once with recorded
   external evidence (not required to be green in CI, but required to be
   evidenced), and the reset/cutover scenarios (§10.13) defined by
   `ADR-046-reset-and-cutover`.
4. The ten-role panel (§12.3) has returned unanimous signoff on the
   `ADR046-W7` snapshot with zero recommendations, and
   `cargo xtask delivery wave seal` + `merge-eligibility` both pass.
5. `CHANGELOG.md` carries a new version header under the project's existing
   Keep-a-Changelog convention (`AGENTS.md` → "Changelog & Releases"),
   summarized by version with every internal wave/finding process marker
   stripped, per that same file's "Process markers stay out of shipped
   artifacts" rule — ADR 0046's `ADR046-W<n>` tags are exactly such a
   process marker and never appear in the released CHANGELOG section.
6. Every prior wave's post-wave cleanup (§14) has been performed, so the
   release cuts from a tree with no dangling ADR 0046 implementation
   worktrees or branches.

Only after all six hold does the auto-release mechanism already documented
in this repository's `AGENTS.md` ("Auto-release") apply unchanged: a new
version header merged to `main` tags `vX.Y.Z` and builds/releases the host
binaries.

## 16. Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | This repository's `AGENTS.md` "Panel review" (8/N-role phase gate, no candidate snapshot/seal), "Stacked PR workflow for large waves," "Worktrees for parallel agents," `tests/AGENTS.md`/`tests/README.md` Layer-1/Layer-2 taxonomy, and `Makefile` targets (`make check-tier0`, `test-unit`, `test-lint`, `test-rust`, `test-proofs`, `test-flake`, `test-drift`, `test-policy`, `check`, `check-static`, `test`, `test-integration`, `test-host-integration`) |
| Evidence class | The Layer-1/Layer-2 test taxonomy and Makefile targets are `production-reachable` (verified directly in `tests/AGENTS.md`, `tests/README.md`, and this repository's `Makefile` target list); the ten-role panel roster is `production-reachable` (verified verbatim in this repository's own `AGENTS.md`); the candidate-snapshot/`xtask delivery`/seal/attest machinery, `cargo xtask heavy-gate`, and the byte-identical history-proof tool are `ADR-only` in this repository today — they exist as a proven, documented process on this codebase's sibling ADR-0045 lineage and are adopted here by explicit copy/adapt under D001/D041, not invented fresh |
| Behavior retained | Layer-1-first bias, closed drift/meta-gate set, hermetic mocking discipline, commit-before-build convention, no-AI-metadata-in-Git convention, worktree/branch hygiene, `KillMode=process` restart-continuation semantics |
| Required delta | Candidate-snapshot immutability, ten-role panel bound to one fixed model/provider and run exactly once per wave (not per round), `xtask delivery` subcommands, `xtask heavy-gate` semaphore, attest/seal/eligibility/history-proof tooling, the exact `ADR046-W0`–`ADR046-W7` wave graph and its file-overlap/shared-prep contracts |
| Reuse path | Copy/adapt the sibling-lineage `xtask delivery`/`xtask heavy-gate` implementations named in §11/§12; extend (never replace) the existing Layer-1/Layer-2 taxonomy and Makefile targets; extend the existing ten-role panel table unchanged |
| Replacement/deletion | Nothing in this repository's current validation/delivery tooling is removed by this spec; `ADR046-delivery-00x` work items (§17) are additive tooling built alongside, not instead of, the existing `Makefile`/panel-review process, until `ADR046-W7` explicitly retires any tooling the migration map marks `DELETE`/`REPLACE` |
| Feasibility proof | The sibling-lineage candidate-snapshot/panel/seal process is already a proven, currently-running process (see the concurrently active sibling panel-writing agents observed during authoring of this spec); `ADR-046-feasibility-proofs` additionally proves the ADR-0046-specific redb/reconciliation/session/package/state numeric targets cited in §10.4 |
| Future owner | Work items in §17 |

## 17. Implementation work items

### ADR046-delivery-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-001` |
| Dependency/owner | `ADR046-W0`; delivery-tooling integrator |
| Current source | none in this repository; `Makefile` heavy-lane targets do not yet exist |
| Reuse source | sibling-lineage `cargo xtask heavy-gate` implementation (per D001/D041 unrestricted-reuse policy) |
| Reuse action | copy-unchanged, then adapt paths/crate names to this repository's `packages/xtask` layout |
| Destination | `packages/xtask/src/heavy_gate.rs`; `Makefile` targets `heavy-check`, `heavy-test-integration`, `heavy-test-host-integration`, `heavy-test-hardware`, `heavy-cargo-test`, `heavy-flake-check` |
| Detailed design | Two-slot per-UID OFD-locked semaphore, 250 ms nonblocking retry up to 30 minutes, fail-closed on unsupported locking, duplicated locked-FD handoff to child, wrapper-owned group-signal/reap, as specified in §11 |
| Integration | Every heavy lane in §10.4/§10.10/§10.11 routes through this one binary; no wave adds a second lock mechanism |
| Data migration | None — net-new tooling |
| Validation | Unit tests for slot acquisition/timeout/fail-closed paths; integration test spawning two concurrent heavy-gate invocations and asserting the second blocks until the first releases |
| Removal proof | Not applicable (net-new; nothing to remove) |

### ADR046-delivery-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-002` |
| Dependency/owner | `ADR046-W0`; delivery-tooling integrator |
| Current source | none in this repository |
| Reuse source | sibling-lineage `cargo xtask delivery wave snapshot` implementation |
| Reuse action | copy-unchanged, then adapt |
| Destination | `packages/xtask/src/delivery/snapshot.rs` |
| Detailed design | Binds base/head OIDs, dependency graph, repository set into `candidate_id`/`content_id`/`snapshot_sha256` per §12.1 |
| Integration | Called by the integrator immediately after PR opening (§13.1), before any validator/panel lane starts |
| Data migration | None |
| Validation | Unit tests asserting identical inputs produce identical digests and any single-byte content change produces a different `content_id` |
| Removal proof | Not applicable |

### ADR046-delivery-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-003` |
| Dependency/owner | `ADR046-delivery-002`; delivery-tooling integrator |
| Current source | none in this repository |
| Reuse source | sibling-lineage `cargo xtask delivery wave validate-import` implementation |
| Reuse action | copy-unchanged, then adapt |
| Destination | `packages/xtask/src/delivery/validate_import.rs`; external candidate-ID-addressed evidence directory (never under Git) |
| Detailed design | Imports CI/local/host validator command/result evidence, keyed by `candidate_id`, per §12.2 |
| Integration | Consumed by `wave seal` (§ADR046-delivery-005) as one of the seal's required inputs |
| Data migration | None |
| Validation | Test asserting evidence for a stale `candidate_id` is rejected; test asserting raw command output never lands in a tracked file |
| Removal proof | Not applicable |

### ADR046-delivery-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-004` |
| Dependency/owner | `ADR046-delivery-002`; spec-set integrator |
| Current source | `docs/specs/README.md`'s described-but-not-yet-generated `ADR-046-spec-set.json`/`ADR-046-work-items.json` contract |
| Reuse source | none required — this generator is specific to the `docs/specs/ADR-046-*` manifest shape |
| Reuse action | adapt (new generator, following the existing `xtask gen-schemas`/`gen-nix-options` pattern already used for other generated artifacts) |
| Destination | `packages/xtask/src/gen_spec_set.rs`; `docs/specs/ADR-046-spec-set.json`, `docs/specs/ADR-046-work-items.json` |
| Detailed design | Enumerates every `docs/specs/ADR-046-*.md` and `docs/specs/providers/ADR-046-provider-*.md` file, its metadata table, content digest, and every `### ADR046-<spec>-<ordinal>` work item, per §8 |
| Integration | `make test-drift` gains a row running this generator and `git diff --exit-code`; every wave's exit criteria (§4) require it committed as the wave's last commit |
| Data migration | None |
| Validation | Golden-fixture test against a small synthetic spec directory; drift test against the real `docs/specs/` tree |
| Removal proof | Not applicable |

### ADR046-delivery-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-005` |
| Dependency/owner | `ADR046-delivery-002`, `ADR046-delivery-003`; panel-tooling integrator |
| Current source | none in this repository; this repository's existing `AGENTS.md` panel-review process is host-local script tooling (`/etc/nixos/scripts/panel-review.{md,sh}`), not a candidate-bound `xtask` subcommand |
| Reuse source | sibling-lineage `cargo xtask delivery wave panel-request`/`panel-attest` implementation |
| Reuse action | copy-unchanged, then adapt to bind the fixed `gemini-3.1-pro-preview`/`github-copilot` model/provider pair and this repository's existing ten-role roster (§12.3) |
| Destination | `packages/xtask/src/delivery/panel.rs` |
| Detailed design | `panel-request` writes the candidate-bound request naming the exact ten roles and required model; `panel-attest` validates a directory of exactly ten strict 13-field records, rejecting wrong model/candidate binding, duplicate provider/run provenance, or inconsistent `signoff`/`recommendations`, per §12.3 |
| Integration | Every wave's exit criteria (§4) require ten unanimous attested records before `wave seal` |
| Data migration | None |
| Validation | Unit tests for every rejection class (wrong model, missing role, duplicate run_id, `signoff:true` with non-empty `recommendations`); integration test with ten synthetic valid records passing |
| Removal proof | Not applicable |

### ADR046-delivery-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-006` |
| Dependency/owner | `ADR046-delivery-002`, `ADR046-delivery-004`, `ADR046-delivery-005`; delivery-tooling integrator |
| Current source | none in this repository |
| Reuse source | sibling-lineage `cargo xtask delivery wave seal`, `merge-eligibility`, and history/byte-identity proof implementation |
| Reuse action | copy-unchanged, then adapt |
| Destination | `packages/xtask/src/delivery/{seal,eligibility,history_proof}.rs` |
| Detailed design | `seal` requires all ten panel records unanimous and bound to the same candidate/content/snapshot digests plus every validator lane passing; `merge-eligibility` checks each stacked PR's current base/head against the sealed OIDs or a passing history-proof; `history_proof` verifies byte-identical integrated content/generated artifacts/dependency diff/repository set across a rebase, per §12.4/§12.6 |
| Integration | `make check` gains no new required step for ordinary contributors; this tooling is invoked only by the wave integrator per §4/§13 |
| Data migration | None |
| Validation | Unit tests for seal rejection on any missing/mismatched record; integration test proving a history-only rebase with identical content passes `history_proof` and reuses panel evidence, while any content change fails it |
| Removal proof | Not applicable |
