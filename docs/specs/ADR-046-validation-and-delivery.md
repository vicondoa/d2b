# ADR 0046 validation and delivery contract

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-validation-and-delivery` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | ADR 0046 integrator, `xtask` delivery tooling, panel/validator process owners |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-store-redb`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-componentsession-and-bus`, `ADR-046-primitive-resource-composition`, `ADR-046-zone-routing`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-core-controllers`, `ADR-046-resources-network`, `ADR-046-resources-credential`, `ADR-046-provider-state`, `ADR-046-resources-zone-control`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-volume`, `ADR-046-resources-device`, `ADR-046-telemetry-audit-and-support`, `ADR-046-cli-and-operations`, `ADR-046-nix-configuration`, `ADR-046-current-code-migration-map`, every `ADR-046-provider-*` dossier, and the `ADR-046-security-and-threat-model`, `ADR-046-streamline`, `ADR-046-reset-and-cutover`, `ADR-046-feasibility-and-spikes` closing specs |
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
`xtask` subcommands. Per ADR 0046 decision D024, future W0-W8 implementation
(§3) requires a separate request. This spec is the binding contract that
request must follow; it does not itself begin that work, and no cleanup,
branch deletion, or worktree removal described in §14 is performed by this
change.

## 2. Manifest closure gate (Gate 0 - precondition for any implementation wave)

No `ADR046-W*` implementation wave in §3 may open until **all** of the
following are true, per the parent ADR's "Review and acceptance" contract and
`docs/specs/README.md`:

1. Every file in the `docs/specs/ADR-046-*` manifest - the exact 28 top-level
   specs and 27 `docs/specs/providers/ADR-046-provider-*` dossiers - is
   `Status: Accepted`. The five closing specs are
   `ADR-046-feasibility-and-spikes`, `ADR-046-reset-and-cutover`,
   `ADR-046-security-and-threat-model`, `ADR-046-streamline`, and this spec.
2. `ADR-046-decision-register` has zero rows under "Open decisions."
3. `docs/specs/ADR-046-spec-set.json` and `docs/specs/ADR-046-work-items.json`
   exist and enumerate every spec above with matching content digests (§8).
   Every declared Markdown work-item heading is in exact bijection with the
   work-item manifest, its owner/path/prefix and mandatory fields match, and
   its `reuseAction` is one closed scalar. The generator is checked in:
   `spec-registry` (`packages/xtask/src/gen_spec_set.rs`) emits both manifests
   and `implementation-graph` (`packages/xtask/src/implementation_graph.rs`)
   emits the graph, both under the fail-closed `make test-drift` gate.
   `ADR046-delivery-004` and `ADR046-delivery-009` own the follow-on hardening
   of that generator and its fail-closed policy tests.
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
wave boundary under the speculative-readiness rule in §6 - the wave number
below is the latest-safe placement, not the earliest-possible one.

This rule derives a wave only for a spec. `ADR046-W8` (§3.2) has no spec
members and is therefore **not** produced by the layering above: it is a
terminal delivery-process wave whose contents - the tooling and process
friction accumulated while delivering `ADR046-W0`-`ADR046-W7`, in the
categories signoff, build, test, merge, codegen, and disk - are triaged and
fixed at `ADR046-W7` close rather than read off a `Depends on` edge. Its
only prerequisite is `ADR046-W7`'s exit criteria (§4), and it runs that same
§4 template unchanged, including exactly one binding ten-role panel (§12.3).

### 3.2 Wave assignment table

| Wave | Specs (all must be `Accepted`; Gate 0 already covers this) | New/changed crates and modules (destination roots) |
| --- | --- | --- |
| `ADR046-W0` | `ADR-046-terminology-and-identities` → `ADR-046-resource-object-model` → `ADR046-store-001` → `ADR-046-resource-api-and-authorization` (serial contract sub-steps, one integrator branch) | `packages/d2b-contracts/src/v3/{identity,resource_ref,resource,resource_status,resource_schema,error}.rs`; `packages/d2b-resource-store/`; `packages/d2b-resource-store-redb/src/{schema,keys,values,ownership}.rs`; `packages/d2b-controller-toolkit/src/owner_hints.rs`; `packages/d2b-contracts/proto/d2b-resource-v3.proto`; `packages/d2b-resource-api/`; `nixos-modules/{options-zones,resources,index}.nix` |
| `ADR046-W1` | `ADR046-feasibility-001` alongside the engine-neutral `ADR046-reconcile-001`/`ADR046-reconcile-002` toolkit and the production-unwired `ADR046-session-001`/`ADR046-session-002`/`ADR046-bus-001` foundations. These are exactly the six merged work items; the failed RSS result defers the production backend, watch dispatcher, and real-backend reaction benchmark. | `proofs/redb-resource-store-spike/`; `packages/d2b-controller-toolkit/` except the real-backend reaction benchmark; `packages/d2b-core-controller/src/{hints,dependencies,owner_reconcile}.rs`; `packages/d2b-contracts/src/v3/component_session.rs`; `packages/d2b-session/`; `packages/d2b-session-unix/`; `packages/d2b-bus/` |
| `ADR046-W2` | `ADR-046-primitive-resource-composition` ‖ `ADR-046-zone-routing` | `packages/d2b-contracts/src/v3/{host,guest,execution_policy,process,volume,user,network,device,credential}.rs`; `packages/d2b-process/`; `packages/d2b-provider-supervisor/`; `packages/d2b-zone-routing/` |
| `ADR046-W3` | `ADR-046-provider-model-and-packaging` (single spec; strictly serial - every downstream Provider dossier depends on it) | `packages/d2b-provider/`; `packages/d2b-provider-toolkit/`; one `packages/d2b-provider-<base>-<implementation>/` skeleton generator |
| `ADR046-W4` | `ADR-046-components-processes-and-sandbox` ‖ `ADR-046-core-controllers` ‖ `ADR-046-resources-network` ‖ `ADR-046-resources-credential` ‖ `ADR-046-provider-state` (five parallel specs) | `packages/d2b-process/`, `d2b-provider-supervisor/` (process effect ports); `packages/d2b-core-controller/`; `packages/d2b-provider-network-local/` schema half; `packages/d2b-provider-credential-*/` schema half; Volume `stateSchema`/`persistenceClass`/`sensitivityClass` extension |
| `ADR046-W5` | `ADR046-store-004` → `ADR046-store-002` → `ADR046-reconcile-003`, with `ADR046-store-003` → `ADR046-store-005`, alongside `ADR-046-resources-zone-control` ‖ `ADR-046-resources-host-guest-process-user` ‖ `ADR-046-resources-volume` ‖ `ADR-046-resources-device` ‖ `ADR-046-telemetry-audit-and-support` ‖ `ADR-046-cli-and-operations` ‖ `ADR-046-nix-configuration` (production store/watch/reaction and storage integration plus seven parallel specs) | `packages/d2b-resource-store-redb/src/{actor,transaction,revision_log,backup,migration}.rs`; `packages/d2b-resource-api/src/watch.rs`; `packages/d2b-controller-toolkit/benches/reaction.rs`; Process Provider integration tests; `packages/d2b-contracts/src/v3/storage.rs`; `nixos-modules/zone-storage-json.nix`; `docs/reference/schemas/v3/zone-storage.json`; broker Zone-store operation/wire/test destinations; `packages/d2b-provider-system-{core,systemd,minijail}/`; `packages/d2b-provider-volume-{local,virtiofs}/` schema half; `packages/d2b-provider-device-*/` schema half; `packages/d2b-telemetry/`, `d2b-audit/`; `packages/d2b/` CLI; `nixos-modules/resources-*.nix` |
| `ADR046-W6` | All 27 `ADR-046-provider-*` dossiers, grouped into five file-disjoint provider families (§3.3) | One `packages/d2b-provider-<base>-<implementation>/` per Provider (27 crates) |
| `ADR046-W7` | `ADR-046-feasibility-and-spikes` (`ADR046-feasibility-002` through `ADR046-feasibility-011`) ‖ `ADR-046-reset-and-cutover` ‖ `ADR-046-security-and-threat-model` ‖ `ADR-046-streamline` ‖ `ADR-046-validation-and-delivery` | Cross-cutting spec-scoped friction fixes, reset/cutover mechanics, remaining feasibility closure, security closure, and the release-gate contract (§15, evaluated at `ADR046-W8` exit) |
| `ADR046-W8` | None - no spec members (§3.1); the wave's contents are the tooling and process friction fixes accumulated across `ADR046-W0`-`ADR046-W7` (signoff, build, test, merge, codegen, disk), triaged at `ADR046-W7` close | `packages/xtask/`; `tests/tools/`; `packages/d2b-contract-tests/tests/`; `Makefile` |

Waves are numbered `ADR046-W0`…`ADR046-W8` - an ADR-046-scoped namespace,
distinct from this repository's general per-plan `Wn` commit-tag convention
in `AGENTS.md`. Commit subjects for ADR 0046 implementation work use
`( ADR046-W<n> )`, `( ADR046-W<n>fu<m> )`, and
`( ADR046-W<n>fu<m> <S><n> )` following the same severity/ordinal grammar
`AGENTS.md` already defines, so existing tooling and human reviewers read one
consistent tag shape. `ADR046-W8` takes the same grammar with no exception:
`( ADR046-W8 )`, `( ADR046-W8fu<m> )`, `( ADR046-W8fu<m> <S><n> )`.

The cross-wave work-item split is intentionally finer than each owning spec's
default position:

| Work item | Assigned wave | Delivery determination |
| --- | --- | --- |
| `ADR046-store-001` | `ADR046-W0` | The engine-neutral trait, closed errors, schema/codecs, and golden vectors are present; this is the complete W0 store contract. |
| `ADR046-feasibility-001` | `ADR046-W1` | The disposable proof crate and both spike results are present. Functional, watch, conflict, crash-recovery, and latency thresholds passed, but whole-process RSS was 25,216 KiB (24.625 MiB), 640 KiB or about 2.6% above 24,576 KiB; the failed outcome is the backend prep barrier evidence. |
| `ADR046-store-004` | `ADR046-W5` | Only contract modules exist in the crate. The failed RSS gate requires the range-seek, streaming-decode, shared-fan-out design corrections before this production backend item starts, so it moves atomically with its watch consumer. |
| `ADR046-store-002` | `ADR046-W5` | Replay/live watch and API watch destinations are absent. It follows the corrected production backend and exclusively owns watch-budget saturation validation. |
| `ADR046-store-003` | `ADR046-W5` | This is a generated storage-row integration contract, not an engine backend item; all Nix/schema/parity destinations are absent and belong with Nix and broker storage wiring. |
| `ADR046-store-005` | `ADR046-W5` | Backup/migration and broker provisioning/fd-handoff destinations are absent; it follows the storage-row contract and production engine. |
| `ADR046-reconcile-003` | `ADR046-W5` | The real-backend reaction benchmark and Process Provider integration tests are absent. It follows `ADR046-store-002`, keeping its latency and concurrency evidence on the accepted backend/watch path it measures. |

These assignments preserve the existing store dependency edges and add the
explicit `ADR046-store-002` prerequisite for `ADR046-reconcile-003`. In
particular, the eight downstream consumers remain direct dependents of
`ADR046-store-001`:
`ADR046-audit-002`, `ADR046-telem-002`, `ADR046-telem-010`,
`ADR046-telem-011`, `ADR046-zone-control-001`,
`ADR046-zone-control-009`, `ADR046-zone-control-010`, and
`ADR046-zone-control-011`.

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

Within a track, the 3-7 Providers are further parallel (each is its own
crate, its own `ADR-046-provider-<name>.md` dossier, and its own
`tests/`/`integration/` tree per D059). The only intra-track ordering
constraint is `volume-local` before `volume-virtiofs` (D083: volume-virtiofs
never writes Volume layout/spec/ownership fields, but its controller
`Depends on` `ADR-046-resources-volume` and reads Ready Volume rows created
by volume-local in integration tests) and `network-local` before
`device-usbip` (device-usbip's dossier lists `ADR-046-resources-network` as a
dependency for its firewall/export attachment). Both are soft integration-test
orderings, not authoring blockers - the crates themselves may be authored
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

`ADR046-W8` has no row above because it has no spec members (§3.1). Its only
prerequisite is `ADR046-W7`'s exit criteria (§4); its work items are recorded
at `ADR046-W7` close rather than derived from a `Depends on` edge.

### 3.5 Machine-readable implementation graph (D095)

The wave topology in §3.1-§3.4 and the file-overlap graph in §6 are also emitted
as a single generated, committed, machine-readable artifact so no author
re-derives launch order or parallelism from this prose. Per D095:

- **Artifacts.** `docs/specs/ADR-046-implementation-graph.json` (canonical) and
  `docs/specs/ADR-046-implementation-graph.md` (rendered human view: Mermaid DAG,
  the `W0`-`W7` table, shared-prep and parallel groups, this ready-wave
  algorithm, the critical path, and counts). Both are **generated non-member
  artifacts**: they are NOT part of the 55 `ADR-046-spec-set.json` members and do
  not change that count.
- **Contract.** The JSON declares `artifactKind`
  (`d2b-adr-implementation-graph`), `schemaVersion`, `adr` (`0046`), and `status`.
  It carries one `node` for every one of the 55 member specs and every work item
  in `ADR-046-work-items.json`, each mapped **exactly once** to a `wave`
  (`W0`-`W7`; `W8` contributes no node until its work items are recorded at
  `ADR046-W7` close, per §3.1), a file-disjoint `parallelGroup`,
  `owner`/`destinations`, `entryContracts`, `prerequisites`, `blockers`
  (empty in a `Proposed` plan unless an explicit blocker is recorded),
  `exitGate`, and `topologicalRank`. It
  carries typed `edges`: `spec-depends-on` (cross-wave spec dependency),
  `shared-contract` (same-wave spec prep barrier), `work-item-depends-on`,
  `implements-spec` (work item → its spec), and `file-overlap-order` (same-wave
  file contention ordering).
- **Source of truth.** `spec-depends-on`/`shared-contract` edges derive from
  `ADR-046-spec-set.json.members[].dependsOn`; waves derive from §3.1-§3.4;
  work-item mapping and `work-item-depends-on` derive from
  `ADR-046-work-items.json`; the W6 `file-overlap-order` edges derive from §3.3.
- **Generation.** The repository generator is checked in. The
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- spec-registry`
  command (`packages/xtask/src/gen_spec_set.rs`) writes `ADR-046-spec-set.json`
  and `ADR-046-work-items.json` from the exact Markdown bytes, and
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- implementation-graph`
  (`packages/xtask/src/implementation_graph.rs`) reads the two manifests plus
  this section and writes sorted output with no timestamps or host paths. Both
  run under the fail-closed `make test-drift` gate. `ADR046-streamline-001` and
  the duplicate-generator reconciliation in `ADR046-streamline-024` own the
  follow-on consolidation of that generator. Regenerate the graph after any spec
  or work-item edit, always **after** `ADR-046-spec-set.json` and
  `ADR-046-work-items.json` are regenerated.
- **Validation.** Every one of the 55 spec nodes and every work item appears
  exactly once; all edge endpoints resolve to a declared node; the graph is
  acyclic; waves are monotonic (every edge's dependency resolves to an earlier
  wave, or to an explicit same-wave `shared-contract`/`file-overlap-order` prep
  barrier whose `topologicalRank` precedes its consumers); every work item is
  mapped to a wave; a `parallelGroup` never implies ordering absent a
  dependency or file-overlap edge; the JSON is deterministic; and every Mermaid
  node ID is a valid identifier. Graph generation also rejects any `W0` work
  item unless its state is `Merged` and every backtick-delimited destination is
  present. This generation-time check is deliberately specific to `W0`, the
  already-closed baseline. Later waves legitimately carry `Planned` items until
  implementation lands; their state is enforced at entry and seal time instead
  of making the plan itself impossible to generate.
- **Anti-serialization.** The graph embodies §6: all ready, file-disjoint
  parallel groups launch concurrently. A same-wave dependency is a
  `shared-contract`/`file-overlap-order` prep barrier before its specific
  consumers - never a reason to serialize a whole wave.

#### 3.5.1 Ready-wave query

A node is **ready to launch** when every node it lists in `prerequisites` is
`done` (or, before implementation begins, when every prerequisite is in an
earlier wave than the current open wave and any same-wave prep barrier has
landed). Against `ADR-046-implementation-graph.json` the exact query is:

```bash
# All ready nodes: no unfinished prerequisite. $done is a JSON array of done node ids.
jq --argjson done "$DONE" '
  .nodes[]
  | select((.prerequisites - $done) | length == 0)
  | {id, kind, wave, parallelGroup, topologicalRank}
' docs/specs/ADR-046-implementation-graph.json

# Ready and NOT yet launched, grouped by file-disjoint parallelGroup so every
# concurrently-launchable track is visible at once (anti-serialization check):
jq --argjson done "$DONE" --argjson launched "$LAUNCHED" '
  [ .nodes[]
    | select((.prerequisites - $done) | length == 0)
    | select([.id] - $launched | length == 1) ]
  | group_by(.parallelGroup)
  | map({parallelGroup: .[0].parallelGroup, wave: .[0].wave, ready: [.[].id]})
' docs/specs/ADR-046-implementation-graph.json
```

A scope that is ready but absent from `$launched` and has no recorded blocker is
an anti-serialization violation (see §6 and `ADR046-streamline-013`).

## 4. Per-wave entry/exit criteria

Every wave (`ADR046-W0`…`ADR046-W8`) uses this template. `ADR046-W8` is no
exception: having no spec members satisfies its spec-scoped entry and exit
clauses vacuously, but every remaining clause - snapshot immutability,
validator lanes, exactly one binding ten-role panel, seal, and merge
eligibility - applies to it unchanged.

Waves are **pipelined**, not strictly serial. A wave's implementation may
begin before its predecessor's panel completes, under all four of these
conditions:

1. At least five of the predecessor's ten roster reviews have returned.
2. The predecessor's integration tests pass on its converged tree.
3. The successor issues no panel request, produces no seal, and merges
   nothing until the predecessor is sealed at 10/10 unanimity with zero
   recommendations **and** merged to the integration lineage.
4. The successor rebases onto the updated integration lineage **before** its
   own panel runs, so the panel binds to a snapshot that already contains
   every predecessor finding.

Panel, seal, and merge therefore remain **strictly ordered** between waves;
only implementation start is pipelined. There is no partial-wave advance in
the sense that matters: a wave is never *delivered* early, and its evidence is
never accepted early.

Rework is the accepted price of the pipeline. When a predecessor finding
invalidates work the successor already started, that rework is absorbed by the
wave that started early. It MUST NOT be cited as grounds to weaken, shorten,
or partially accept the predecessor's panel - which would trade a bounded,
known cost for an unbounded, unknown one.

**Entry criteria (all required):**

1. Gate 0 (§2) has passed. Entry does **not** require the predecessor's work
   items to be `Merged`; that condition binds at this wave's panel request and
   seal instead (§12.3, §12.4), which is what makes the pipelined start above
   executable. Items in the wave being entered may remain `Planned`; the seal
   gate below is what requires their promotion after implementation.
2. Every destination path this wave's work items name (§3.2, §7) is free of
   an open, unresolved contention flag from an earlier wave.
3. The wave's Git Town stack (§5) has been proposed against the exact parent
   commit named in its dependency edges (§3.4), not against a stale `v3`.
4. The heavy-gate semaphore (§11) is available (not held past
   its 30-minute timeout by a stale prior-wave validation run).
5. The fast hermetic suite (§10.16) passes within its execution budgets on the
   wave's entry tree; it is the required default inner loop for every change
   in the wave and must be green before any `integration/` lane is scheduled.

**Exit criteria (all required):**

1. Every spec's work items assigned to this wave show `Validation` evidence
   satisfying §10's applicable matrix rows, imported per §12.2, including the
   §10.16 runtime-ledger artifact showing every enforced aggregate crate
   process-CPU budget met, with the pinned test census complete and per-test
   wall-clock diagnostics reported.
2. The immutable candidate snapshot (§12.1) for this wave's integrated tree
   has all required CI, local, and host validator lanes reporting (pending is
   acceptable only while the PR is open, per §13; not at wave close).
3. The ten-role panel (§12.3) has returned unanimous `signoff: true` against
   that exact snapshot, with zero outstanding `recommendations`.
4. `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave seal`
   (§12.4) has verified that every work item assigned to this wave is `Merged`,
   then produced a sealed record binding this wave's
   `candidate_id`/`content_id`/`snapshot_sha256`.
5. `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-eligibility`
   reports eligible for every
   PR in the wave's stack, and each has merged root-to-leaf through GitHub
   (§13).
6. Post-wave cleanup (§14) is recorded as pending for the integrator (not
   necessarily executed before advancing - advancing needs the merge, not the
   worktree teardown).

No wave may begin implementation subagent dispatch before its entry criteria
hold; no wave may be marked delivered before its exit criteria hold. This
mirrors and tightens this repository's existing `AGENTS.md` "Phase gate"
rule (`## Panel review` → `### Phase gate`): where that rule allows a panel
per implementation round, ADR 0046 restricts the **binding** panel to exactly
one occurrence per wave, run only against the wave's single immutable final
snapshot (§12), never against interim implementation rounds within the wave.

Every wave's work reaches `v3` through pull requests that pass the gates
above. Direct commits to the integration lineage, and local merges that
bypass the panel/seal/eligibility sequence, are prohibited regardless of how
small or how mechanical the change looks.

## 5. Git Town stack shape and worktree/branch ownership

Per D001, the protected `v3` branch is this work's integration branch. Every
ADR 0046 slice branches from and merges back into `v3`; the v3 line never
merges to `main`, so `main` is never a slice branch's base or target.

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
2. **Stack only real dependencies.** A slice branch targets `v3` if every
   one of its `Depends on` specs already merged to `v3`; it targets the
   exact prerequisite PR branch if that prerequisite is still open but
   dependency-complete-enough per §6's speculative rule. `ADR046-W0`'s four
   serial steps are one branch each, stacked linearly
   (`adr046-w0-identities` → `adr046-w0-object-model` → `adr046-w0-store` →
   `adr046-w0-api`), proposed with
   `git town propose --stack --non-interactive --no-browser`.
3. **`ADR046-W1`/`ADR046-W2`/`ADR046-W4`/`ADR046-W5`/`ADR046-W6` parallel
   slices** each branch from the exact merged (or, speculatively, exact
   open) tip of their prerequisite branch and target `v3` once that
   prerequisite merges - never targeting an unrelated parallel sibling slice.
4. **The integrator owns**: shared-prep commits (§7), Cargo.toml workspace
   member list and `flake.nix` output additions, `docs/specs/ADR-046-spec-set.json`
   / `ADR-046-work-items.json` regeneration, cross-slice conflict resolution,
   root-to-leaf merge order (§13.3), branch retargeting after a lower PR
   merges, and post-wave cleanup (§14). The integrator is not the default
   implementation sink for any slice that can be assigned independently
   (mirrors the sibling repository's anti-serialization invariant, item 4).
5. **PR bodies** contain only dependency, base/head/tree, `candidate_id`/
   `content_id`, and check-status summaries, per §12.5 - never raw
   validation output, panel transcripts, or AI/tool/model attribution.
6. **Reviewers and panel roles** inspect the plan/diff and supplied evidence;
   they do not re-run tests/builds/evals unless the integrator explicitly
   asks, per this repository's existing `AGENTS.md` panel-prompt rule.
7. Slice worktrees are removed only after their commits are integrated and
   their per-worktree `packages/target/` is cleaned, per §14.

## 6. Speculative readiness and the anti-serialization file-overlap graph

### 6.1 Speculative-start rule

A slice's implementation branch **may** open before its assigned wave number
in §3.2 closes, provided:

1. every spec it `Depends on` (§3.4) already has `Merged` work-item state
   (not merely "wave complete" - the precise edge, not the coarse wave), and
2. no destination path it will write (per its work items' `Destination`
   field) is currently claimed by another **still-open** branch, per the
   contention index in §6.2/§7.

For example, `resources-network` (computed wave W4) and `resources-credential`
(also W4) may each open as soon as `provider-model-and-packaging` (W3) merges -
they need nothing from `components-processes-and-sandbox` or
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
   conflict - avoiding possible conflicts is not, by itself, grounds to
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
| `packages/Cargo.toml` workspace member list | every new crate across `ADR046-W0`-`ADR046-W6` | integrator-only trailing commit per merged slice; never edited inside a slice's own PR diff except to add that slice's own single new member line, which the integrator rebases to the current tail before merge |
| `flake.nix` package/output list | every new Provider crate (`ADR046-W6`) | integrator-only trailing commit, batched per track (§3.3), same rule as Cargo.toml |
| `nixos-modules/index.nix`, `nixos-modules/default.nix` | `ADR046-identities-002` (zones/resources), every `ADR-046-provider-*` Nix authoring section (`ADR046-W5`/`ADR046-W6`) | `ADR046-W0` lands the base zones/resources wiring; each `ADR046-W6` Provider slice appends its own resource-type Nix module import as a single line, rebased by the integrator at merge time, never touching another Provider's import line |
| `packages/d2b-contract-tests/tests/workspace_policy.rs` | every Provider crate-layout assertion (D059/`ADR046-pstate-011`-equivalent gates), one row per Provider | integrator batches one appended assertion per merged `ADR046-W6` slice; a slice's own PR adds only its own assertion function, appended after the current last function, never reordering existing ones |
| `docs/specs/ADR-046-spec-set.json`, `docs/specs/ADR-046-work-items.json` | regenerated after every spec status/work-item-state change (§8) | integrator-only; regenerated and committed as the last commit of each wave, never inside a slice's own PR |
| `packages/d2b-core-controller/src/rbac.rs`, `authz_audit.rs` | `resource-api-and-authorization` (W0-adjacent api-002 work item), `resources-zone-control` (Role/RoleBinding schema), `telemetry-audit-and-support` (audit hooks) | `resources-zone-control` (W5) lands the concrete Role/RoleBinding schema atop the W0 `rbac.rs` skeleton; `telemetry-audit-and-support` (W5, parallel) adds only its own `authz_audit.rs` audit-emission hooks, a distinct file, so this is a false-positive overlap once split at the file (not module) level - recorded here so the integrator does not accidentally serialize two already-disjoint files under one shared symbol name |
| `CHANGELOG.md` `## [Unreleased]` block | every slice in every wave (`ADR046-W0`-`ADR046-W8`), because the changelog gate requires release notes for any code change | no slice edits `CHANGELOG.md`; each slice writes one `changelog.d/<branch>.md` fragment carrying standard Keep a Changelog `### <Section>` headings (see `changelog.d/README.md`), which no other slice touches, and the integrator runs `cargo run --manifest-path packages/Cargo.toml -p xtask -- changelog-fold` at wave close to collate every fragment into `## [Unreleased]` by section and delete the consumed fragments. The `test-changelog` gate accepts either a `CHANGELOG.md` entry or a fragment, so slices never need the shared file |

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
   exists (already true - all 28 top-level specs and 27 dossiers exist at
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
   For D115's Zone storage row, the implementation slice writes only the Rust
   DTO and Nix emitter. After that slice merges, the integrator runs the schema
   generator and owns the rendered-contract parity test under
   `packages/d2b-contract-tests/tests/`; the implementation slice never writes
   that forbidden cross-cutting destination.
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
   table, §0/§1-§9) are deleted only in the wave whose successor spec closes
   them - for example, the per-realm PID1 broker/controller systemd units
   (§5.2 of the migration map) are deleted only after `ADR046-W5`'s
   `resources-zone-control`/`core-controllers` successors are integrated and
   the removal-proof test passes; `d2b-realm-router` session types are
   deleted only after `ADR046-W1`'s `componentsession-and-bus` successor
   routes every v3 peer path.
5. A `REPLACE`-disposition row (e.g. `d2b-realm-router/src/router.rs` →
   Zone-local resource routing) follows the same rule but may retain its old
   file as a dead, test-gated stub for one wave beyond its successor's
   landing if - and only if - a still-open sibling slice's integration test
   fixture references it; the stub's removal is then a follow-up commit in
   the same wave, not deferred indefinitely.
6. This spec's own `ADR046-W7` ("streamline & cutover") is the wave that
   performs bulk final deletion of every remaining `RETAIN`-until-parity row,
   gated by `ADR-046-reset-and-cutover`'s destructive-cutover mechanics and
   verified by the release/cutover gate in §15, which is evaluated at
   `ADR046-W8` exit. No deletion happens in this documentation-only change.

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
| Layer-1 rust | `cargo test --workspace` across every new crate, including the three broker feature passes where a new crate touches `d2b-priv-broker` (none does, per D077 - no Provider process imports the broker) | `make test-rust` | existing target |
| Layer-1 proofs | Any new `proofs/` crate for redb/session invariants (only if a wave's feasibility spike needs a separate proof crate; see `ADR-046-feasibility-and-spikes`) | `make test-proofs` | existing target |
| Layer-1 flake | `eval-*` checks extended with Zone/resource examples once `ADR046-W5`'s `nix-configuration` lands; `examples/minimal` gains a `d2b.zones.dev.resources.*` block | `make test-flake` | existing target, new fixture |
| Layer-1 drift | Schema/Nix-option/spec-set drift gates from §8 | `make test-drift` | existing target, extended rows |
| Layer-1 policy | Workspace-policy, provider-crate-layout, and telemetry/audit-redaction policy lints (§10.4, §10.9) | `make test-policy` | existing target, extended rows |

### 10.2 Rust unit/property/fuzz/fault/conformance

| Row | Coverage | Location |
| --- | --- | --- |
| Unit | Every DTO/schema/validator introduced by `ADR046-object-001/002`, `ADR046-store-001..005`, `ADR046-api-001/002` - canonical JSON, bounds, redaction, unknown-field rejection | `packages/d2b-contracts/src/v3/**` `#[cfg(test)]`, `packages/d2b-resource-store-redb/src/**` |
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

These rows are completion gates, not existing evidence. SPIKE-01 and SPIKE-02
have both been executed. SPIKE-02 passed every profile; SPIKE-01 passed
correctness, watch delivery, group commit and crash recovery but **failed its
RSS gate** at a measured median of 25,216 KiB against a 24,576 KiB threshold.
That failure is why `ADR046-store-004`, `ADR046-store-002` and
`ADR046-reconcile-003` are scheduled in W5 rather than W1, and why the revised
physical-schema plan in `ADR-046-resource-store-redb` is binding on them.

Executing the spikes does not satisfy the rows below. They remain future
completion gates for the production redb backend, watch and dispatcher
integration, and backup/migration work, and the unchanged RSS gate must be
re-run against the real backend before any of that is accepted. D128 continues
to permit only the engine-neutral contract and the hermetic small-scale scope
it enumerates.

| Fixture | Hard target |
| --- | --- |
| Empty store readiness | <=500 ms |
| Aggregate Zone resource service/store + fixed system-core + system-minijail controllers idle RSS | <=64 MiB |
| p95 local Get/bounded List | <=2 ms |
| p95 crash-safe single-resource mutation | <=10 ms |
| p95 durable commit → matching controller handler start | <=5 ms |
| p95 ready Process commit → launch-attempt start | <=20 ms |
| 10,000 resources | list/get/watch fixture - must meet the above p95s under load |
| 100 live watches | fan-out fixture |
| 1/10/100 concurrently ready Process resources | fast-launch concurrency fixture (`ADR046-reconcile-003`) |
| Expected-revision conflict storm | no silent merge; every stale write returns `resource-conflict` with current revision |
| Owner-trigger fan-in/chain | bounded depth/work budget, no amplification |
| Revision compaction | durable floor advances; below-floor cursor gets `revision-expired` |
| Forced crash at every commit boundary | no partial/ambiguous commit observable after recovery |
| Backup/restore/internal schema upgrade | staged validate → atomic publish → rollback-window retention |
| Repeated open/close and long-reader rejection | no reader starves the single writer |

The RSS row is not a contract-only store pass criterion. After SPIKE-01 runs,
`ADR046-store-004` production backend work records the resource service/store
median at the 10,000-resource/100-watch fixture and fails above 24 MiB; that
evidence blocks backend completion.

`ADR046-store-004` also carries the production-registration gates that the
contract item cannot satisfy, because the contract ships no backend. Sealing
the wave that lands it requires, in addition to the spike evidence and the
resident-set-size row: conformance evidence that a registered backend mutates
only through verified admission and exposes no independent write path, and a
recorded security review of each registered backend. The seal must not close
without both. The admission seal prevents a caller forging authorization and
binds evidence to one store, but a backend, once registered, is trusted, so
these are the review obligations that stand in place of a structural guarantee.
 The work that lands the fixed controllers
separately fails above 22 MiB for `Provider/system-core` and 12 MiB for
`Provider/system-minijail`. Provider integration alone may record the
aggregate row as passing, after measuring all three processes live and at or
below 64 MiB; the unallocated 6 MiB is variance headroom.

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
  durable commit); parent/child Zone access tests (disconnected child-local
  outbound intent, reconnect reauthorization against current revision); attachment
  descriptor validation (encrypted, service/method/request/operation/
  generation bound, CLOEXEC, duplicate-object rejection).

### 10.6 EffectPort/broker

- `ProcessLaunchEffectPort` (ProviderSupervisor) ticket verification,
  package/template/resource-output resolution, and identity/pidfd-evidence
  observation tests (`ADR046-process-001`).
- `VolumeLayoutEffectPort`/`VolumeSourceEffectPort`,
  `NetworkEffectPort`/`DeviceEffectPort` - every call carries only opaque
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
  never reaches the Provider process as a literal path - D082).
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
  justified`; no bootstrap state Volume or bootstrap-storage mechanism exists -
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
  `ProcessSpec`-has-no-inline-`endpoints` lint; standard ResourceType count is 19
  (Endpoint present in the catalog); every retained public `*Id`/`Handle` has a
  documented rationale row; schema vectors accept exactly
  `visibility=owner|provider|zone` and reject aliases; controller admission
  tests cover all three scopes plus each
  `consumerPolicy.allowedSubjects`/`allowedProviderComponents`/
  `allowedOperations` mismatch; a docs drift test parses every Endpoint
  example and fails if visibility is outside the enum, if a finer restriction
  is encoded as another visibility value, or if `consumerPolicy` is a
  scalar/array alias. ResourceExport visibility is explicitly outside this
  Endpoint-only drift check.
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

### 10.9a Cross-Zone sharing (ResourceExport/ResourceImport, D096)

Hermetic (fake ZoneLink/stream/clock/adapter) fast tests and slower integration
(real bounded encrypted streams) tests cover, per
[`ADR-046-resources-zone-control.md` §8A.7](ADR-046-resources-zone-control.md):

- signed projection factory binds qualified semantic/provider-neutral Service
  type, Binding type, allowed owner-Service backing refs, allowed Binding target
  refs, projection schema/fingerprint, and factory fingerprint; absent/unsigned/
  tampered/mismatched metadata fails closed at Provider install, Nix build, and
  API admission;
- `ResourceExport.resourceRef` accepts only the owner Service; Device, Endpoint,
  Binding, Credential, backend, and cross-Zone targets reject;
- one import creates exactly one same-qualified-type projection Service with
  `ownerRef: ResourceImport/<name>`; it creates no Device/Endpoint/Binding;
- one or more separately authored same-Zone Bindings reference that Service plus
  an allowed Guest/User/Zone and own Process/Endpoint children; Binding spec is
  desired consumer intent only and observations appear only in status; import
  never exports, auto-creates, or auto-deletes Binding;
- opt-in required on both sides; unauthorized Zone and capability/fingerprint
  mismatches reject;
- exact frozen type/Provider mapping:
  `audio.d2bus.org.AudioService` + `audio.d2bus.org.AudioBinding` with
  `audio-pipewire`, `security-key.d2bus.org.SecurityKeyService` +
  `security-key.d2bus.org.SecurityKeyBinding` with `device-security-key`,
  `telemetry.d2bus.org.TelemetryService` +
  `telemetry.d2bus.org.TelemetryBinding` with `observability-otel`, and
  policy-gated `usb.d2bus.org.UsbService` + `usb.d2bus.org.UsbBinding` with
  `device-usbip`;
- canonical minimal Service/Binding base admission succeeds without
  `spec.provider`; export/import preserves semantic type across independently
  selected conformant implementations; every Core projection has ResourceImport
  ownership, `providerRef`, semantic base/import fields, and no `spec.provider`;
  provider/adapter identity changes leave the semantic factory fingerprint
  unchanged while signed-descriptor authentication still detects substitution;
  common semantic observations exist only under `status.resource` and
  implementation observations only under `status.provider`; mismatched factories and
  PipeWire/CTAPHID/OTEL/USBIP detail in base spec/status/conditions/errors/
  fingerprints reject;
- quota/fairness/deadline enforcement; reconnect revalidation and revocation
  degrade the projection Service; D091 update propagation owner Service →
  export → import → projection Service → Binding → children; finalizer waits
  visibly for Bindings to be deleted/retargeted;
- audio: speaker mix with per-Zone volume/quota and microphone
  exclusivity/consent/fair-queue; security-key: CTAP serialization with one
  exclusive per-device lease/deadline/cancel; observability: one SigNoz ingest
  with many producer Zones under quota/backpressure/redaction/cardinality;
  USBIP: all Provider/Zone/export/device policy gates required; one physical
  token selected by USBIP and security-key resolves to a byte-identical
  Core-derived `(Host, physical-usb-backing, opaqueKeyDigest)` tuple, and the
  second claimant receives `physical-usb-backing-conflict` before any effect;
- only those four Provider families are admitted (USBIP policy-gated); all other
  Providers and every Binding are non-exportable;
- Nix canonical resources and projection names are inspectable/byte-stable even
  when transparent sugar lowers to them; CLI renders import → projection
  Service → Binding → owned Process/Endpoint without hidden nodes;
- **no FD, secret, Credential, backing/remote Ref, raw path/locator, device/
  socket handle, token, or payload bytes crosses a Zone or appears in status/
  CLI/audit**. High-churn sessions/streams stay internal; intermediaries see
  ciphertext.

All metadata, admission, graph, cleanup, and update-propagation cases are fast,
deterministic, parallel-safe, hermetic tests using fake adapters/streams/clocks.
Real cross-Zone audio, security-key, observability, and policy-gated USBIP
streams run only in the slower integration tier and prove the same invariants
with the production encrypted named-stream implementation.

### 10.10 Container integration and host runNixOSTest KVM/TCG

| Row | Coverage | Command |
| --- | --- | --- |
| Container | Provider controller against a real Zone runtime in a container; resource lifecycle under real broker calls; cleanup-contract scenarios (§0.2 of the migration map) | `make test-integration` (podman; local host/manual pre-PR, per this repo's existing tier) |
| Host runNixOSTest, KVM | Live daemon/broker/socket-activation/host-posture/kernel behavior for every new Zone runtime, Provider process, and cleanup/rollback scenario | `make test-host-integration` (x86_64-linux, KVM) |
| Host runNixOSTest, TCG fallback | Same suite when `/dev/kvm` is absent - slower, still required before Wave exit for waves touching kernel-adjacent behavior (process/adoption, cgroup, virtiofs) | `make test-host-integration` (TCG fallback path, already documented in this repo's `AGENTS.md`) |

### 10.11 Hardware/live/cloud manual

| Row | Coverage | Command |
| --- | --- | --- |
| Hardware | Real GPU/YubiKey/hardware-TPM passthrough for `device-gpu`, `device-security-key`, `device-tpm` | `make test-hardware`, manual, on a host with the devices |
| Live-host | Destructive/stateful checks against a real deployed Zone (store adoption, restart/power-loss, USBIP guestd lifecycle equivalents) | `D2B_LIVE=1 bash tests/integration/live/<name>.sh`, manual, never CI |
| Cloud | `runtime-azure-container-apps`, `runtime-azure-virtual-machine`, `transport-azure-relay`, `credential-managed-identity`, `credential-entra` against real Azure resources | manual tier, gated by `ADR-046-feasibility-and-spikes`; never run in CI or as a required wave-exit lane - recorded as external evidence only |

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
  schema, or internally inconsistent fails closed - never silently
  recreated (`ADR-046-resource-store-redb` "Store identity").

### 10.13 Reset/cutover

Deferred in content to `ADR-046-reset-and-cutover` (forthcoming, required
before `ADR046-W7` per §3.4); this spec fixes only the **gate shape**:
`ADR046-W7`'s exit criteria (§4) additionally require every destructive-reset
test named by that spec to pass on the `ADR046-W7` candidate snapshot, and
the release/cutover gate (§15) re-requires them on the `ADR046-W8` snapshot
before it opens.

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
  their implementation, not deferred - per this repository's existing
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

### 10.16 Hermetic execution budgets, placement, and the runtime ledger (D094)

The fast hermetic suite (`src/` `#[cfg(test)]` units and crate `tests/*.rs`)
is the default inner loop, required on every change; all slower coverage
lives only in `integration/`. Measurements below exclude compilation. The
runtime ledger enforces aggregate per-crate process CPU after a warm build;
per-test libtest wall-clock p95s are advisory diagnostics because unrelated
machine load can inflate them while the process is descheduled.

| Measurement | Threshold and enforcement |
| --- | --- |
| Individual normal hermetic test | advisory wall-clock p95 diagnostic threshold of <=50 ms; no wall-clock sleep |
| Per Provider crate `cargo test -p d2b-provider-<base>-<implementation> --lib --tests` | enforced aggregate process-CPU p95 budget of <=3 s |
| All 27 Provider hermetic suites, sharded | future target of <=30 s aggregate wall |
| All 27 Provider hermetic suites, single host | future target of <=60 s |
| Each Layer-1 hermetic shard (`make test-rust` split) | future target of <=60 s |
| Classified bounded crypto/property exception | named per test; capped case count; declared higher advisory threshold |

The runtime-ledger gate today enforces only the per-crate process-CPU row over
its pinned closed census, presently `d2b-core` and exactly 190 test IDs. The
per-test wall-clock row is advisory only. The exact pin makes a vanished or
extra test fail rather than silently shrinking the measured set. The
multi-suite aggregate and per-shard rows are targets for the deferred
follow-up `runtime-ledger-full-census-and-real-shards`, which grows the census
to a real multi-crate shard inventory; the gate has no shard dimension until
that lands.

**Placement rules (a violation must move, never gain a sleep/timeout/`#[ignore]`):**

- `src/` units and `tests/` are in-process only - deterministic fake
  clock/RNG; fake `ResourceClient`/EffectPort/broker/transport/credential/
  systemd; in-memory or tiny-temp bounded redb fixtures; parallel-safe with
  no global mutable or shared ports/paths; exact bounded property case counts.
  No sleep/retry on wall clock, process spawn, containers, network, DBus,
  systemd, broker daemon, Nix eval/build, KVM, USB/GPU/TPM hardware, live
  cloud, or filesystem trees beyond tiny temp fixtures.
- `integration/` owns any real process, socket rendezvous, container, Nix
  eval/build, guest VM/KVM/TCG, broker/systemd/DBus, real filesystem
  quota/mount/namespace, hardware, or live cloud/Entrablau. It may be slower
  but still has a lane timeout/budget, parallel isolation, and fake external
  services by default; live/hardware remain a separate manual tier.

**Runtime ledger + timing gate.** A machine-readable test-runtime ledger and
a timing gate reuse the existing `xtask`/Make tooling (no new top-level
`tests/*.sh` gate): they record the reference runner, repetition count, and
per-test wall-clock and per-crate process-CPU p95s. Only aggregate crate
process CPU is budget-enforced. Per-test wall-clock measurements are compared
with diagnostic thresholds and remain advisory; the tool's own check output
is authoritative for their exact report formatting and selection. The gate
holds no baseline and makes no historical-regression claim, and there is no
shard dimension today. Growing the census to a real multi-crate shard
inventory with a per-shard budget plus a genuine cross-machine reference
baseline is the deferred follow-up
`runtime-ledger-full-census-and-real-shards`.
`make test-rust` and the Layer-1 hermetic shards run concurrently; expensive
integration lanes take the sole heavy-gate slot (§11). CI caches compile
outputs, but correctness never depends on the cache, and cold compile time is
tracked and optimized separately via shared cache and dependency discipline.

**Legacy retirement.** When ADR 0046 replaces a behavior, the minimum
reusable semantic assertions migrate into the new hermetic suite and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and
manifest entries are deleted once successor coverage and the §9 removal proof
pass - old and new suites never run indefinitely. Every current-code
migration/replacement work item names the exact old test selectors/files with
a keep/adapt/move/delete disposition and a removal gate, and updates
`tests/layer1-jobs.json`, closed gate manifests, flake/matrix/Nix-unit pins,
generated ledgers, and CI workflow shards. Policy self-tests assert that an
intentional sleep/process/network test in a hermetic tier is rejected, an
over-budget aggregate crate CPU measurement fails the gate, a per-test
threshold breach remains advisory, an incomplete or shrunk census fails
closed, parallel isolation holds, and a retired legacy selector is absent.

## 11. Heavy-gate: sole use

Every Layer-2/hardware/live/perf-heavy command anywhere in ADR 0046
delivery - `make test-integration`, `make test-host-integration`,
`make test-hardware`, `D2B_LIVE=1 bash tests/integration/live/*.sh`, the
redb benchmark suite (§10.4), and any cloud-tier manual run (§10.11) - MUST
run only through one shared semaphore:

```bash
cargo run --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- <command> [args...]
```

Invoke `xtask` this way from the repository root throughout this document so
that root-relative path arguments and any wrapped `make` command resolve
against the repository root; this is the form `xtask --help` emits.
`cargo run --manifest-path` keeps the working directory at the repository root
but, because cargo config discovery is cwd-based, does not pick up the
`sccache` rustc-wrapper declared in `packages/.cargo/config.toml`. That
wrapper is immaterial here: the xtask binary build is trivial and the wrapped
`make` targets run their own cargo invocations from `packages/`, where the
wrapper still applies. If you specifically want the wrapper for the xtask
build itself, run `cd packages && cargo xtask ...` instead and pass any file
arguments relative to `packages/`.

This is adopted by copy/adapt (per D001/D041) from the equivalent tooling
already proven on this codebase's sibling ADR-0045 lineage: a two-slot
per-UID OFD-locked semaphore in the fixed, system-provisioned
`/run/d2b-heavy-gates/uid-<uid>/` namespace. The root and per-uid directory
are root-owned and non-writable by unprivileged users; the two `slot-*`
files are pre-created, owned by the target uid, and mode `0600`. Acquisition
is nonblocking, retried every 250 ms for up to 30 minutes, and fail-closed
(with no `flock` fallback) on unsupported locking or timeout. The child
receives a duplicated locked-FD handle and the wrapper retains
group-signal/reap ownership exactly as documented.

There is no fallback namespace. An absent or malformed root is an
environment error whose diagnostic names `make heavy-gate-provision` as the
remediation. The NixOS module uses a systemd-tmpfiles rule for the fixed root
and provisions configured lifecycle users' root-owned per-uid directories
and two private slots after numeric UIDs are available; hosts not consuming
the module use the Make target. A user-owned temporary root was vulnerable
in two independent ways: a foreign uid could squat it and force the
foreign-owner check to deny service, while the owning uid could rename it
and obtain a fresh two-slot pool that defeated the semaphore. A conditional
`/run/user/<uid>` location is also rejected: it is not always present and
its uid-owned namespace permits the same rename and split-pool attack.

Building this tool (if not already present at ADR 0046 implementation time)
is work item `ADR046-delivery-001` (§17); every wave's `make heavy-check`,
`make heavy-test-integration`, `make heavy-test-host-integration`, and
`make heavy-test-hardware` targets route through it. "Sole use" means: no
wave, no Provider crate, and no panel/validator role may create a second
ad hoc lock file, a bespoke sleep-and-retry loop, or a per-crate heavy-lane
guard - every heavy lane in every wave shares the exact same two slots, so
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

This mirrors `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave help`'s `snapshot` subcommand as
already specified for this codebase's sibling ADR-0045 lineage (built here as
work item `ADR046-delivery-002`, §17, by copy/adapt per D001/D041). Any
content change after the snapshot - including generated output, dependency
metadata, contract fingerprints, or repository-set membership - invalidates
both validator and panel evidence; the wave re-snapshots and both lanes
rerun. A history-only rebase or retarget may reuse panel evidence only when
the canonical proof tool (§12.6) verifies byte-identical integrated content.

Before creating a snapshot for any wave after `W0`, the command reads
`ADR-046-implementation-graph.json` and `ADR-046-work-items.json` from the
candidate's exact integrated Git tree. It rejects entry when any item assigned
to an earlier wave is not `Merged`; it does not reject `Planned` items in the
wave being entered.

Each repository requires at least one
`--pull-request LOGICAL_ID=NUMBER:HEAD_REF` mapping, and the repeated mappings
must name the complete expected pull-request set. The repository's `--head`
(or its default) must resolve to one of those mapped heads. This binding is
what prevents a later merge target from silently omitting a parallel slice:

```bash
cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave snapshot \
    --program <name> --wave <id> \
    --repo <logical-id>=<checkout-root> \
    --base <logical-id>=<base-revision> \
    --pull-request <logical-id>=<positive-number>:<head-ref>
```

Every persisted delivery artifact and workflow result uses
`DELIVERY_SCHEMA_VERSION` 2, introduced when
`expected_pull_requests` became required candidate material.

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
   `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave validate-import` (built as work item
   `ADR046-delivery-003`, §17).
3. **The ten-role panel** (§12.3), run against the same snapshot.

A pending lane is valid only while the wave's PR stack is open (§13.1); it
never permits merge (§13.3). Command/result evidence is imported into an
external, candidate-ID-addressed state directory - never committed to Git,
copied into generated artifacts, or pasted into a PR body (§12.5).

### 12.3 Ten-role final panel, bound to Gemini 3.1 Pro Preview

Every ADR 0046 wave's binding panel - run exactly once, at wave close,
against the wave's one immutable snapshot, never per implementation round -
uses this repository's existing ten-role default roster (`AGENTS.md` →
"Panel review" → "Default panel"), with every role's provider/model bound by
the wave's `panel-request` record to:

```text
provider: github-copilot
model_version: gemini-3.1-pro-preview
reasoning_effort: xhigh
```

The panel model is deliberately **not** the model that writes the code. The
implementation lanes for this program run on `gpt-5.6-sol`; binding the
reviewing roster to a different model means a lane cannot both author a change
and attest to it, and `panel-attest` rejects any record carrying the coding
model. Keep the two pins distinct when either is changed.

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

`cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-request` writes the candidate-bound request
(binding `candidate_id`/`content_id`/`snapshot_sha256`, the exact ten-role
roster, and the required `gemini-3.1-pro-preview` model at reasoning effort `xhigh`).
`cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave panel-attest` validates a directory containing
exactly one record per role, each shaped exactly as this repository's sibling
ADR-0045-lineage panel-receipt artifact:

```json
{
  "artifact_kind": "d2b-delivery/panel-receipt",
  "schema_version": 2,
  "role": "software",
  "candidate_id": "<sha256>",
  "content_id": "<sha256>",
  "snapshot_sha256": "<sha256>",
  "model_version": "gemini-3.1-pro-preview",
  "provider": "github-copilot",
  "reasoning_effort": "xhigh",
  "run_id": "run-001",
  "receipt_locator": "github-copilot://runs/run-001/software",
  "output_sha256": "<sha256>",
  "signoff": true,
  "recommendations": []
}
```

`signoff` is `true` iff `recommendations` is `[]`; any finding requires a
content change, which creates a new snapshot and invalidates every prior
validation/panel record for that wave. Green tests never waive this gate -
every wave, including a documentation-only or single-crate wave, requires
unanimous 10/10 signoff before its exit criteria (§4) are met. Building this
tooling (if not already present) is work item `ADR046-delivery-004`/`-005`
(§17), copy/adapted from the equivalent sibling-lineage tooling per D001/D041.

### 12.4 Seal, merge target, and merge eligibility

`cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave seal` requires all ten panel records
present, unanimous, and bound to the same `candidate_id`/`content_id`/
`snapshot_sha256`, plus every §12.2 validator lane reporting success on that
exact snapshot. It also reads the implementation graph and work-item state
manifest from the snapshot's integrated Git tree and rejects the seal unless
every item assigned to the current wave is `Merged`. The error names the item
and the required state transition. `merge-eligibility` repeats this current-wave
check so an eligibility result cannot bypass stale delivery state.

`cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-target` then captures the wave's current
pull-request stack into a canonical `merge-target.json` under the candidate.
The step performs no network I/O: the integrator produces the input out of
band from `gh pr view --json` or `gh api` in the same step that merges (the
same freshness window a direct API call inside the process would have), then
installs it:

```bash
cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-target \
    --seal   <state>/<wave>/<candidate>/seal.json \
    --target ./merge-target.json \
    --repo   <logical-id>=<checkout-root>
```

The `MergeTarget` document is a `d2b-delivery/merge-target` artifact:

```json
{
  "artifact_kind": "d2b-delivery/merge-target",
  "schema_version": 2,
  "material": { "...": "the wave's re-derived integrated material" },
  "pull_requests": [
    {
      "repository": "<logical repository id>",
      "number": 42,
      "base_ref": "<base branch>",
      "base_oid": "<base commit object id>",
      "head_ref": "<head branch>",
      "head_oid": "<head commit object id>",
      "required_checks": [ { "name": "<check>", "conclusion": "success" } ]
    }
  ]
}
```

`material` has the same shape the snapshot recorded, re-derived after any
rebase. Each pull request `number` MUST be a positive integer (the real
pull-request number); `0` is rejected. `pull_requests` is bounded (at most
64 pull requests, each with at most 128 required checks). Only a `success` conclusion permits a merge; any
pending, failure, neutral, skipped, cancelled, stale, timed-out,
action-required, or startup-failure check, a pull request with no required
checks, a sealed repository with no open pull request, or a base not reachable
from the sealed base fails closed. The step validates the shape,
canonicalizes `material`, and writes the canonical `merge-target.json` so the
gate's input is produced by a supported command rather than dropped in by
hand.

`cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-eligibility` then confirms,
per PR in the wave's stack: the seal exists, the PR's current base/head
still matches the sealed snapshot's recorded OIDs (or a history-only rebase
has passed the byte-identical proof in §12.6), and every required GitHub
check is green. It reads the captured `merge-target.json` when no `--target`
path is given.

### 12.5 No raw evidence or AI metadata in Git/PR

Validation command output, panel transcripts, and attestation payloads
never enter Git, generated source, a PR body, or a release archive. PR
bodies carry only: dependency list, base/head/tree OIDs, `candidate_id`/
`content_id`, and check-status summaries, with optional links to external
evidence. No PR description, commit message, CHANGELOG entry, or shipped doc
names or lists the AI agent, assistant, tool, or model used to author or
review the change (per this repository's existing `AGENTS.md` "AI/tool
attribution" rule, extended here to also cover panel attestation records -
the panel's own `model_version`/`provider` fields exist only inside the
external, non-Git delivery-state directory, never inside a committed file).

### 12.6 Content invalidation and byte-identical history proof

Any content change to the wave's integrated tree - including generated
output, dependency metadata, contract/index content, or repository-set
membership - invalidates both the validator and panel lanes for that wave.
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
immutable snapshot (§12.1) from that exact open-PR/stack state - it does not
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

1. Merge proceeds only after `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave merge-eligibility`
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
   every PR in its own stack has merged through GitHub - never through a
   local octopus merge or a direct push to `v3` for ADR-scale work
   (per this repository's existing `AGENTS.md` "Finish-of-work invariant"
   and "Stacked PR workflow" sections, which remain binding for ADR 0046).

## 14. Post-wave cleanup (policy only - no deletion performed by this change)

After each wave's every PR merges (§13.3), the integrator:

1. Deletes the merged remote feature branches for that wave's slices.
2. Cleans the slice worktree's per-worktree `packages/target/` before
   removing the worktree; sccache retains the compiled outputs, so the
   next worktree's build stays cheap.
3. Removes the finished local worktrees and deletes their local branches.
4. Runs `nix-collect-garbage` and verifies `git worktree list` contains only
   active work for the next wave.
5. Confirms no abandoned/superseded branch is silently dropped - any
   worktree branch whose tip is unmerged but represents abandoned work is
   flagged for the operator, per this repository's existing `AGENTS.md`
   worktree-audit rule.

This is a policy statement only. This documentation-only change creates no
worktrees, branches, or generated artifacts, so no cleanup step above is
executed as part of landing this spec.

## 15. Release/cutover gate

The release/cutover gate is evaluated at **`ADR046-W8` exit**, not at
`ADR046-W7` exit. `ADR046-W7` performs the destructive cutover (§9), but
`ADR046-W8` (friction closure, §3.1/§3.2) is the program's terminal wave and
therefore produces the tree that actually ships; gating on `ADR046-W7` would
release a candidate that a later wave still modifies. `ADR046-W8` does not
close, and d2b 3.0 does not release, until all of:

1. `ADR-046-streamline`, `ADR-046-security-and-threat-model`,
   `ADR-046-reset-and-cutover`, `ADR-046-feasibility-and-spikes`, and this
   validation spec are `Accepted` and their own work items' `Validation`
   evidence is imported per §12.2.
2. Every `DELETE`/`REPLACE` row in `ADR-046-current-code-migration-map` has
   satisfied its removal-proof test (§9) on the `ADR046-W8` candidate
   snapshot - the removal proofs `ADR046-W7` established must still hold on
   the shipping tree. This is the destructive-cutover gate; d2b 3.0 does not
   ship with both the v3-pre-ADR-0046 code path and its successor coexisting
   indefinitely.
3. The `ADR046-W8` snapshot has passed §10's complete matrix, including the
   manual hardware/live/cloud tiers (§10.11) at least once with recorded
   external evidence (not required to be green in CI, but required to be
   evidenced), and the reset/cutover scenarios (§10.13) defined by
   `ADR-046-reset-and-cutover`.
4. The ten-role panel (§12.3) has returned unanimous signoff on the
   `ADR046-W8` snapshot with zero recommendations, and
   `cargo run --manifest-path packages/Cargo.toml -p xtask -- delivery wave seal` + `merge-eligibility` both pass.
5. `CHANGELOG.md` carries a new version header under the project's existing
   Keep-a-Changelog convention (`AGENTS.md` → "Changelog & Releases"),
   summarized by version with every internal wave/finding process marker
   stripped, per that same file's "Process markers stay out of shipped
   artifacts" rule - ADR 0046's `ADR046-W<n>` tags are exactly such a
   process marker and never appear in the released CHANGELOG section.
6. Every prior wave's post-wave cleanup (§14) has been performed, so the
   release cuts from a tree with no dangling ADR 0046 implementation
   worktrees or branches.

Only after all six hold does the auto-release mechanism already documented
in this repository's `AGENTS.md` ("Auto-release") apply, with `v3` as the
integration branch in place of `main`: a new version header merged to `v3`
tags `vX.Y.Z` and builds/releases the host binaries.

## 16. Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | This repository's `AGENTS.md` "Panel review" (8/N-role phase gate, no candidate snapshot/seal), "Stacked PR workflow for large waves," "Worktrees for parallel agents," `tests/AGENTS.md`/`tests/README.md` Layer-1/Layer-2 taxonomy, and `Makefile` targets (`make check-tier0`, `test-unit`, `test-lint`, `test-rust`, `test-proofs`, `test-flake`, `test-drift`, `test-policy`, `check`, `check-static`, `test`, `test-integration`, `test-host-integration`) |
| Evidence class | The Layer-1/Layer-2 test taxonomy and Makefile targets are `production-reachable` (verified directly in `tests/AGENTS.md`, `tests/README.md`, and this repository's `Makefile` target list); the ten-role panel roster is `production-reachable` (verified verbatim in this repository's own `AGENTS.md`); the candidate-snapshot/`xtask delivery`/seal/attest machinery, `cargo xtask heavy-gate`, and the byte-identical history-proof tool have since landed in this repository under `packages/xtask` and are `production-reachable` (copy/adapted from this codebase's sibling ADR-0045 lineage under D001/D041, not invented fresh); their remaining work is hardening, not creation |
| Behavior retained | Layer-1-first bias, closed drift/meta-gate set, hermetic mocking discipline, commit-before-build convention, no-AI-metadata-in-Git convention, worktree/branch hygiene, `KillMode=process` restart-continuation semantics |
| Required delta | The `xtask delivery` subcommands, the `xtask heavy-gate` semaphore, and the attest/seal/eligibility/history-proof tooling have landed; the remaining delta is process contract rather than net-new tooling: candidate-snapshot immutability hardening, the ten-role panel bound to one fixed model/provider and run exactly once per wave (not per round), and the exact `ADR046-W0`-`ADR046-W8` wave graph and its file-overlap/shared-prep contracts |
| Reuse path | Copy/adapt the sibling-lineage `xtask delivery`/`xtask heavy-gate` implementations named in §11/§12; extend (never replace) the existing Layer-1/Layer-2 taxonomy and Makefile targets; extend the existing ten-role panel table unchanged |
| Replacement/deletion | Nothing in this repository's current validation/delivery tooling is removed by this spec; `ADR046-delivery-00x` work items (§17) are additive tooling built alongside, not instead of, the existing `Makefile`/panel-review process, until `ADR046-W7` explicitly retires any tooling the migration map marks `DELETE`/`REPLACE` |
| Feasibility proof | The sibling-lineage candidate-snapshot/panel/seal contract supplies the reuse design named in §11/§12; `ADR-046-feasibility-and-spikes` owns the ADR-0046-specific redb/reconciliation/session/package/state numeric proofs cited in §10.4 |
| Future owner | Work items in §17 |

## 17. Implementation work items

### ADR046-delivery-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-001` |
| Dependency/owner | `ADR046-W0`; delivery-tooling integrator |
| Current source | `packages/xtask/src/heavy_gate.rs` and the `Makefile` heavy-lane targets have since landed in this repository (copy/adapted from the sibling-lineage source below); remaining effort is hardening, not creation |
| Reuse source | sibling-lineage `cargo xtask heavy-gate` implementation (per D001/D041 unrestricted-reuse policy) |
| Reuse action | adapt |
| Destination | `packages/xtask/src/heavy_gate.rs`; `Makefile` targets `heavy-check`, `heavy-test-integration`, `heavy-test-host-integration`, `heavy-test-hardware`, `heavy-cargo-test`, `heavy-flake-check` |
| Detailed design | Two-slot per-UID OFD-locked semaphore in the fixed `/run/d2b-heavy-gates/uid-<uid>/` namespace: root-owned non-writable root and per-uid directories, two pre-created target-uid-owned mode-`0600` slot files, no fallback, and a provisioning error naming `make heavy-gate-provision` when the namespace is absent or malformed. The NixOS module provisions the root through systemd-tmpfiles and the configured lifecycle-user slots after numeric UIDs exist; the Make target provisions hosts that do not consume the module. Acquisition uses a 250 ms nonblocking retry up to 30 minutes, fails closed on unsupported locking, duplicates the locked FD into the child, and retains wrapper-owned group-signal/reap semantics, as specified in §11. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy-unchanged, then adapt paths/crate names to this repository's `packages/xtask` layout. |
| Integration | Every heavy lane in §10.4/§10.10/§10.11 routes through this one binary; no wave adds a second lock mechanism |
| Data migration | None - net-new tooling |
| Validation | Unit tests for slot acquisition/timeout/fail-closed paths; integration test spawning two concurrent heavy-gate invocations and asserting the second blocks until the first releases |
| Removal proof | Not applicable (net-new; nothing to remove) |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-002` |
| Dependency/owner | `ADR046-W0`; delivery-tooling integrator |
| Current source | `packages/xtask/src/delivery/snapshot.rs` has since landed in this repository (copy/adapted from the sibling-lineage source below); remaining effort is hardening, not creation |
| Reuse source | sibling-lineage `cargo xtask delivery wave snapshot` implementation |
| Reuse action | adapt |
| Destination | `packages/xtask/src/delivery/snapshot.rs` |
| Detailed design | Binds base/head OIDs, dependency graph, repository set, and the complete expected pull-request number/head set into `candidate_id`/`content_id`/`snapshot_sha256` per §12.1. Every repository requires at least one `--pull-request LOGICAL_ID=NUMBER:HEAD_REF` mapping, and the selected head must be one of those mapped heads. Persisted delivery artifacts and workflow results use `DELIVERY_SCHEMA_VERSION` 2 because `expected_pull_requests` is required candidate material. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy-unchanged, then adapt. |
| Integration | Called by the integrator immediately after PR opening (§13.1), before any validator/panel lane starts |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Unit tests asserting identical inputs produce identical digests and any single-byte content change produces a different `content_id` |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-003` |
| Dependency/owner | `ADR046-delivery-002`; delivery-tooling integrator |
| Current source | `packages/xtask/src/delivery/evidence.rs` (the `validate-import` step) has since landed in this repository (copy/adapted from the sibling-lineage source below); remaining effort is hardening, not creation |
| Reuse source | sibling-lineage `cargo xtask delivery wave validate-import` implementation |
| Reuse action | adapt |
| Destination | `packages/xtask/src/delivery/validate_import.rs`; external candidate-ID-addressed evidence directory (never under Git) |
| Detailed design | Imports CI/local/host validator command/result evidence, keyed by `candidate_id`, per §12.2 Primary reuse disposition: `adapt`. Preserved source-plan detail: copy-unchanged, then adapt. |
| Integration | Consumed by `wave seal` (§ADR046-delivery-005) as one of the seal's required inputs |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Test asserting evidence for a stale `candidate_id` is rejected; test asserting raw command output never lands in a tracked file |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-004` |
| Dependency/owner | `ADR046-delivery-002`; spec-set integrator |
| Current source | `packages/xtask/src/gen_spec_set.rs` (invoked as `cargo run --manifest-path packages/Cargo.toml -p xtask -- spec-registry`) has since landed and now generates the `ADR-046-spec-set.json`/`ADR-046-work-items.json` contract described in `docs/specs/README.md`; remaining effort is hardening, not creation |
| Reuse source | none required - this generator is specific to the `docs/specs/ADR-046-*` manifest shape |
| Reuse action | adapt |
| Destination | `packages/xtask/src/gen_spec_set.rs`; `docs/specs/ADR-046-spec-set.json`, `docs/specs/ADR-046-work-items.json` |
| Detailed design | Enumerates every `docs/specs/ADR-046-*.md` and `docs/specs/providers/ADR-046-provider-*.md` file, its metadata table, bytewise-sorted `workItemPrefixes` registry, content digest, and every `### ADR046-<registered-prefix>-<ordinal>` work item, per §8. Each prefix belongs globally to exactly one member, and generation resolves ownership only through the registry rather than by splitting IDs or filenames. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt (new generator, following the existing `xtask gen-schemas`/`gen-nix-options` pattern already used for other generated artifacts). |
| Integration | `make test-drift` gains a row running this generator and `git diff --exit-code`; every wave's exit criteria (§4) require it committed as the wave's last commit |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Golden-fixture test against a small synthetic spec directory; drift test against the real `docs/specs/` tree |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-005` |
| Dependency/owner | `ADR046-delivery-002`, `ADR046-delivery-003`; panel-tooling integrator |
| Current source | `packages/xtask/src/delivery/panel.rs` (the `panel-request`/`panel-attest` subcommands) has since landed as the candidate-bound path (copy/adapted from the sibling-lineage source below); this repository's existing `AGENTS.md` panel-review process remains host-local script tooling (`/etc/nixos/scripts/panel-review.{md,sh}`). Remaining effort is hardening, not creation |
| Reuse source | sibling-lineage `cargo xtask delivery wave panel-request`/`panel-attest` implementation |
| Reuse action | adapt |
| Destination | `packages/xtask/src/delivery/panel.rs` |
| Detailed design | `panel-request` writes the candidate-bound request naming the exact ten roles and required model; `panel-attest` validates a directory of exactly ten strict 14-field records, rejecting wrong model/candidate binding, duplicate provider/run provenance, or inconsistent `signoff`/`recommendations`, per §12.3 Primary reuse disposition: `adapt`. Preserved source-plan detail: copy-unchanged, then adapt to bind the fixed `gemini-3.1-pro-preview` model at reasoning effort `xhigh`/`github-copilot` provider pair and this repository's existing ten-role roster (§12.3). |
| Integration | Every wave's exit criteria (§4) require ten unanimous attested records before `wave seal` |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Unit tests for every rejection class (wrong model, missing role, duplicate run_id, `signoff:true` with non-empty `recommendations`); integration test with ten synthetic valid records passing |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-006` |
| Dependency/owner | `ADR046-delivery-002`, `ADR046-delivery-004`, `ADR046-delivery-005`; delivery-tooling integrator |
| Current source | `packages/xtask/src/delivery/{seal,eligibility,history_proof}.rs` (the `seal`, `merge-target`, `merge-eligibility`, and byte-identical history-proof steps) have since landed in this repository (copy/adapted from the sibling-lineage source below); remaining effort is hardening, not creation |
| Reuse source | sibling-lineage `cargo xtask delivery wave seal`, `merge-eligibility`, and history/byte-identity proof implementation |
| Reuse action | adapt |
| Destination | `packages/xtask/src/delivery/{seal,eligibility,history_proof}.rs` |
| Detailed design | `seal` requires all ten panel records unanimous and bound to the same candidate/content/snapshot digests plus every validator lane passing; `merge-eligibility` checks each stacked PR's current base/head against the sealed OIDs or a passing history-proof; `history_proof` verifies byte-identical integrated content/generated artifacts/dependency diff/repository set across a rebase, per §12.4/§12.6 Primary reuse disposition: `adapt`. Preserved source-plan detail: copy-unchanged, then adapt. |
| Integration | `make check` gains no new required step for ordinary contributors; this tooling is invoked only by the wave integrator per §4/§13 |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Unit tests for seal rejection on any missing/mismatched record; integration test proving a history-only rebase with identical content passes `history_proof` and reuses panel evidence, while any content change fails it |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-007` |
| Dependency/owner | `ADR046-W0`; delivery-tooling integrator |
| Current source | the test-runtime ledger has since landed as `packages/xtask/src/test_runtime_ledger.rs`, invoked by `make test-runtime-ledger` against the pinned `tests/runtime-ledger-census.json` and run as the `test-runtime-ledger` Layer-1 job; this codebase's earlier `tests/tools/` timing logs (`d2b-static-timing.$$/`) remain ad hoc and are not this candidate-bound ledger, so remaining effort is the deferred follow-up `runtime-ledger-full-census-and-real-shards` (grow the census to a real multi-crate shard inventory and add a cross-machine reference baseline), not creation and not a historical-regression gate |
| Reuse source | existing `xtask`/`libtest --format=json` timing output; no new test framework |
| Reuse action | adapt |
| Destination | `packages/xtask/src/test_runtime_ledger.rs`; a `make`-invokable timing gate reusing `make test-rust`/Layer-1 crate targets |
| Detailed design | After a warm build, records advisory per-test libtest wall-clock p95s and enforced aggregate per-crate process-CPU p95s. Process CPU excludes time descheduled behind unrelated machine load. The gate enforces each crate CPU budget and the exact closed census, presently one crate and 190 test IDs, so a vanished or extra test fails; a per-test diagnostic-threshold breach does not. It emits a machine-readable artifact with no baseline, shard dimension, or historical-regression claim, and its own check output is authoritative for exact advisory-report formatting and selection. The full census across a real multi-crate shard inventory with a per-shard budget and a cross-machine reference baseline are the deferred follow-up `runtime-ledger-full-census-and-real-shards`; the placement lint rejects a hermetic-tier test that sleeps, spawns a process, or touches network/containers/DBus/systemd/broker/Nix/KVM/hardware/live cloud, and the deterministic-clock/sleep lint rejects wall-clock sleep/retry in `src/`/`tests/`. |
| Integration | Every wave's entry/exit criteria (§4) consume the ledger artifact; `make test-rust` and Layer-1 shards run concurrently; no new top-level `tests/*.sh` gate is added |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Policy self-tests: intentional sleep/process/network behavior in a hermetic test is rejected; a per-test wall-clock threshold breach remains advisory; an over-budget aggregate crate process-CPU p95 fails; an incomplete, expanded, or shrunk exact census fails closed; parallel isolation holds under shuffled/parallel execution; a retired legacy selector is absent from `tests/layer1-jobs.json`, closed gate manifests, and CI shards |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-008` |
| Dependency/owner | `ADR046-streamline-001`, `ADR046-W0`; delivery-tooling integrator |
| Current source | the implementation-graph generator (`packages/xtask/src/implementation_graph.rs`, `xtask implementation-graph`) has since landed and emits the committed `ADR-046-implementation-graph.{json,md}`, regenerated and `git diff --exit-code`d by the `tests/unit/gates/drift-check.sh` drift gate and run in Layer 1; launch order and parallelism were previously derived only from this spec's §3/§6 prose, so remaining effort is hardening, not creation |
| Reuse source | `ADR-046-spec-set.json`, `ADR-046-work-items.json`, and §3.1-§3.4/§3.5 of this spec; no new framework |
| Reuse action | adapt |
| Destination | `docs/specs/ADR-046-implementation-graph.json`, `docs/specs/ADR-046-implementation-graph.md` (generated by `ADR046-streamline-001`'s `xtask implementation-graph`); the artifact contract, generation, validation, and ready-wave query are owned by §3.5 of this spec |
| Detailed design | Owns the D095 implementation-graph contract: `artifactKind`/`schemaVersion`/`adr`/`status`; one node per member spec and per work item mapped exactly once to a `W0`-`W7` wave and a file-disjoint `parallelGroup`, with `owner`/`destinations`/`entryContracts`/`prerequisites`/`blockers`/`exitGate`/`topologicalRank`; work-item nodes additionally embed the manifest's exact `detailedDesign` and `validation` text byte-for-byte; typed `spec-depends-on`/`shared-contract`/`work-item-depends-on`/`implements-spec`/`file-overlap-order` edges; the §3.5.1 ready-wave query; and the anti-serialization invariant that every ready file-disjoint group launches concurrently while a same-wave dependency is a prep barrier, not whole-wave serialization. The graph is a generated non-member artifact and does not change the 55-member `ADR-046-spec-set.json` count. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (D095 artifact contract). |
| Integration | Consumed by §4 wave entry/exit and §6 anti-serialization checks and by `ADR046-streamline-013`; a `tests/unit/gates/` drift gate regenerates and `git diff --exit-code`s the graph after any spec/work-item edit |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Every 55 spec node and every work item present exactly once; all edge endpoints resolve; graph acyclic; waves monotonic (dependencies earlier or explicit same-wave prep barrier); parallel groups claim no ordering absent a dependency/file-overlap edge; deterministic JSON with no timestamps/host paths; every Mermaid node ID valid; the ready-wave query returns the expected concurrently-launchable groups on a seeded fixture |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-delivery-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-delivery-009` |
| Dependency/owner | `ADR046-delivery-004`, `ADR046-delivery-008`; spec-set policy-test owner |
| Current source | The fail-closed completeness, identity, and closed-action contract in `docs/specs/README.md`; the checked-in `packages/xtask/src/gen_spec_set.rs` generator and `packages/d2b-contract-tests/tests/policy_adr046_work_items.rs` policy test, hardened by this item |
| Reuse source | `ADR046-delivery-004` generator shape and the existing `d2b-contract-tests` fixture-driven policy-test pattern |
| Reuse action | adapt |
| Destination | `packages/xtask/src/gen_spec_set.rs`; `packages/d2b-contract-tests/tests/policy_adr046_work_items.rs`; generated spec-set, work-item, and implementation-graph drift checks |
| Detailed design | Parse every normative member's exact level-three `### ADR046-<registered-prefix>-<ordinal>` headings and tables; `##` or `####` item declarations are invalid. Require an exact Markdown/manifest bijection; exact `specId` and `specPath`; a bytewise-sorted, nonempty `workItemPrefixes` list for each item-owning member; global one-member ownership for every registered prefix; registry-based ID ownership; three-digit nonzero ordinals; every mandatory field exactly once and nonempty; one closed scalar `reuseAction`; and `reuseSource: null` for `create`. Reject dropped, extra, malformed, wrong-level, duplicate, ambiguous, heuristic-split, unregistered-prefix, or unconsumed items before writing any artifact. Validate all dependency endpoints, DAG acyclicity, wave monotonicity, and single-wave parallel groups before atomically publishing all generated files. |
| Integration | `make test-policy` runs negative fixtures; `make test-drift` regenerates all ADR 0046 artifacts and requires a clean diff; `ADR046-delivery-008` consumes only a manifest that passed this policy |
| Data migration | None - documentation/build-policy contract only |
| Validation | Fixtures fail for a dropped heading, `##`/`####` item heading, extra manifest row, duplicate ID, duplicate cross-member prefix, unsorted/empty required prefix registry, wrong owner/path/prefix, heuristic-only prefix match, two-digit/zero ordinal, missing/duplicate mandatory field, free-form/compound action, `create` with a reuse source, dangling dependency, cyclic DAG, backward-wave dependency, and cross-wave parallel group; the exact 55-spec real tree passes with every item once |
| Removal proof | Not applicable; the policy remains the permanent generated-artifact closure gate |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |
