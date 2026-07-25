# ADR 0046 streamline contract

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-streamline` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | ADR 0046 integrator; `packages/xtask` owner; `packages/d2b-contract-tests` policy-lint owner |
| Depends on | `ADR-046-decision-register`, `ADR-046-provider-model-and-packaging`, `ADR-046-provider-state`, `ADR-046-primitive-resource-composition`, `ADR-046-resources-volume`, `ADR-046-zone-routing`, `ADR-046-current-code-migration-map`, `ADR-046-terminology-and-identities`, `ADR-046-reset-and-cutover` |
| Supersedes | None |

## Purpose

This spec is the normative contract for the tooling and process changes that
close the authoring friction actually observed while producing the
`docs/specs/ADR-046-*` set. It defines no new Zone/Provider/resource runtime
behavior. Every improvement below traces to a specific, citable repository
artifact: a commit, a duplicated diff, a decision-register entry that exists
because an invented field had to be walked back, or a measured repetition
count across the committed dossiers. No item in this spec is speculative or
grounded in general best practice; an item that cannot be tied to one of the
evidence citations in [Observed friction evidence](#observed-friction-evidence)
does not belong here and was deliberately left out.

This is a documentation-only contract. It defines future
`packages/xtask`, `packages/d2b-contract-tests`, `tests/tools/`, and
`AGENTS.md`/`tests/AGENTS.md` changes as implementation work items per
[Implementation work items](#implementation-work-items). It does not itself
add a build, script, or test - that requires the separate implementation
request every other ADR 0046 work item requires per D024.

**Scope boundary.** This spec's tooling is a meta-process artifact for
authoring and delivering the ADR 0046 spec set and its future `ADR046-W0`-`ADR046-W8`
implementation work items. It is unrelated to, and does not reintroduce, any
v3 Zone/Provider runtime architecture excluded elsewhere in the set - in
particular `ADR-046-zone-routing`'s explicit exclusion of "ADR45 delivery
seals (xtask delivery wave / panel / seal process)" from *runtime* reuse is
about Zone socket/spawn/credential architecture, not about this repository's
own authoring workflow. Only the external delivery-tooling *shape* (generated
command indexes, panel-record schemas, disk/worktree hygiene, anti-serialization
reporting) is evaluated for reuse here, and only as process tooling that ships
outside the Zone runtime.

## Ordering: final wave, with explicit shared prep

Per `docs/specs/README.md` "Parallel authoring" and the parent ADR's
"Foundation specs are authored first... all ready file-disjoint... specs are
authored in parallel", tooling that changes how specs are *written* must not
sit downstream of the specs it is meant to fix. Per this repository's own
`AGENTS.md` "Integrator-prep-first pattern (W3 onwards)", the same rule that
governs code waves applies here: prep that a wave's downstream work depends on
lands before that wave opens, not after.

This spec therefore splits its work items into two tranches, not one flat
"implementation wave":

- **Tier A - shared prep.** Items that operate purely on already-committed
  Markdown (schemas, extractors, lints, generators, scaffolds, process/
  reporting tooling) carry no risk to in-flight spec content: they are
  additive, read-only over the tree, and require no unresolved decision. These
  land now, before the ADR 0046 documentation set's own pre-panel gate (D014),
  because every Tier A item's underlying friction already occurred during
  documentation authoring - waiting for a future implementation wave would let
  the same friction recur in the remaining spec-set iteration and in the
  Provider dossiers that still receive corrections.
- **Tier B - the streamline wave.** Items that can only run meaningfully
  against real Rust crates, real controller processes, or real generated
  code - none of which exist yet, since D024 confirms ADR 0046 has delivered
  documentation only - are deferred. This tier is the actual "implementation
  wave after all product waves": it lands only after every ResourceType,
  core-controller, and Provider dossier work item across the `ADR046-W0`-`ADR046-W8`
  implementation range reaches its own Validation-complete state (per each
  spec's own "Implementation work items" acceptance rows) and the
  `ADR-046-reset-and-cutover` cutover engine is integrated, immediately before
  that implementation delivery's own pre-panel gate.

No Tier A item blocks, gates, or gains veto power over any dossier or
foundation spec's own authoring; Tier A tooling only reports, generates, and
validates. No Tier B item is scheduled ahead of the product code it depends
on. This is the same anti-serialization posture this repository's Panel
review / stacked-PR sections already apply to code waves, extended to the
documentation-authoring tooling that governs this ADR's own delivery.

## Observed friction evidence

Every citation below is a commit SHA, file path, or line count already in this
repository's history (`git log --all`) or the committed `docs/specs/` tree, no
session transcript or tool/agent attribution. Counts were reproduced with
`git log --all --oneline --grep=... -i | wc -l`, `git rev-list --count`, and
`grep -c` against the exact tree state.

### F1 - Dossier branches authored against stale foundation commits

Every per-dossier Provider branch that exists in this repository diverges from
the shared foundation at the exact same commit,
`3ca4daf598ee3e33084bccd4cc5497ee78d87bac` ("docs: define d2b 3.0 Network
resources") - confirmed by `git merge-base <branch>
adr0046-provider-resource-control-plane` returning that SHA for
`adr0046-provider-device-gpu`, `adr0046-provider-volume-virtiofs`,
`adr0046-provider-volume-local`, `adr0046-provider-transport-vsock`, and
`adr0046-provider-runtime-azure-container-apps`. The integrator branch kept
moving after that point through `f76787b5` ("system-core and virtiofs Provider
dossiers"), `c8f47656` ("normalize ADR 0046 cleanup contracts"), `038b4169`,
`b63ce675`, `3b21a7cf`, and the two foundation-harmonize commits
`38ab1d4cd3697df19765d9a238764d98ff562b0d` /
`dfea59933251fbfb13eb70613642f5215bbc1f74` - an identical-diff pair (same
parent `b4e0d6c7`, same tree, same author timestamp, two different commit
objects on two different branches) that reconciles Provider `ResourceSpec`,
`ProviderStateSet`, and bootstrap-exception wording the stale dossier branches
had already diverged from. Every dossier branch therefore had to receive a
second, independent correction pass instead of restacking cleanly onto one
prep commit. This is the exact failure mode this repository's own
`AGENTS.md` "Integrator-prep-first pattern (W3 onwards)" section exists to
prevent for code waves; it was not applied to this documentation wave.

### F2 - Invented Process/Volume fields later rejected

The decision register records the rejection of specific invented fields as
normative corrections, not hypothetical risk:

- `ADR-046-decision-register` D075: "Canonical Provider ResourceSpec is
  exactly `{ artifactId; config }`" - the corresponding foundation-harmonize
  commits (`38ab1d4c`/`dfea5993`) exist because earlier dossier text (per
  `git log --all -S"rootConfig" -- docs/specs`, 15+ matching commits) carried
  an authored `rootConfig`/manifest/package/status field set that had to be
  collapsed back to `artifactId`/`config` across every dossier.
- `2af904efdee589454d7b6ee1353acf46db06549b` ("fix readiness fields,
  sourcePolicyId enum, Volume quota structure, mount required ( W1 )") and
  `367eb867ccc85eb9f5e5946cb84e1c479428313e` ("fix environmentClass enum,
  readiness.probe, Volume schema/quota regressions ( W1 )") each retract an
  invented custom `readiness`/`restart`/budget shape back to the common
  status/ExecutionPolicy fields the foundation already defined.
- `200aaa2d5ea7e9854d4c41ca396f6b92f001e618` ("empty/read-only state Volumes,
  sourcePolicyId, required mounts ( W0 )") retracts an invented optional-mount
  shape back to `mounts` being `required: true` for state Volumes.
- D079 exists because dossier text had authored numeric `hostUid`/`hostGid`
  fields directly on the public `SandboxSpec` (`git log --all -S"hostUid" --
  docs/specs` and `-S"hostGid"` both match `3b21a7cf`, `38ab1d4c`, `dfea5993`,
  `158a019c`, `42383089`, `e112a9a2`, `b4da8460`); D079 replaces them with the
  frozen semantic `mappingClass: process-principal-root` field precisely
  because the numeric fields leaked host identity into a public spec.

### F3 - ProviderStateSet representation and bootstrap-cycle confusion

`ProviderStateSet` churned through at least four distinct representations
before D076/D086 froze it: a `ResourceType`, a stored row, a non-Volume
"compartment" concept, and finally "the logical, query-time grouping of
ordinary Volume resources owned by a Provider... never a ResourceType or a
stored row." Evidence: `52a43339` ("correct ProviderStateSet to
framework-created Volume with provider-state extension schema"),
`f00696d1` ("correct ProviderStateSet to ordinary Volume resources with
provider-state extension schema ( W1 )"), `de303739` ("correct
ProviderStateSet representation in ADR-046 virtiofs dossier ( W3 )"),
`3ff0fa9c` ("fix ProviderStateSet representation in
ADR-046-provider-shell-terminal ( W0 )"), and `bd0e9545`/`2755448d`/`bac0c9d7`
("align Provider state contracts" / "tighten Provider state quota policy").
`git log --all --oneline -S"ProviderStateSet" -- docs/specs | wc -l` returns
20+ distinct commits correcting the same concept across separate dossiers.
The bootstrap-cycle confusion (which component may create its own state
Volume before any `volume-local` controller exists to create it) produced a
parallel churn: `5c287f51` ("add bootstrap-state exception to volume-local
dossier"), `7632ebec` ("add bootstrap-state exception to
Provider/system-minijail dossier"), and `24598e5c` ("scope bootstrap exception
to volume-local controller per execution domain") before D086 froze the
single per-execution-target, non-resource, closed exception. D087 later
removed that exception entirely by making resource `status`/the core Operation
ledger the default state surface and Provider state Volumes optional, so no
bootstrap state Volume and no bootstrap-storage mechanism remain.

### F4 - One-resource/one-controller Volume-vs-virtiofs ownership conflict

`a7c0ceebbddca8e98482bba3e2231a5ed3cb19ed` ("docs(specs): redesign
volume-virtiofs dossier around Export ResourceType") is titled
"Controller-ownership bug fix and full architectural realignment: One
resource instance has one controller (Volume.providerRef=volume-local).
volume-virtiofs cannot reconcile or write Volume resources." This followed an
earlier design where `volume-virtiofs` wrote Volume rows directly, violating
the single-owner-controller invariant the resource object model already
required. `bf8c7c04` ("correct Volume ownership - Core ProviderDeployment
creates/deletes, not controller"), `dc3f3349` ("core ProviderDeployment owns
state Volume lifecycle, not transport controller"), and `d4677af5` ("Volume
lifecycle owned by core ProviderDeployment, not controller") show the same
one-owner violation recurring across three different Provider families before
D083 froze `volume-local` as sole Volume-row owner and `volume-virtiofs` as an
attachment-status-only Provider.

### F5 - Transport Providers incorrectly owning ZoneLink

D081 ("Transport Providers are carriage services only; they never own
ZoneLink") exists because earlier transport dossier text had the Provider
itself reading/writing/finalizing `ZoneLink`. Evidence:
`f2ed0c93` ("ZoneLink FD-attachment prohibition; add route_class to
OpenTransport ( W1 )"), `14590cc5` ("rewrite transport-unix Provider dossier;
correct ownership model ( W1 )"), `94746f20` ("rewrite Provider
transport-vsock dossier as service carriage Provider ( W0 )"), and
`d5944e67`/`6799e193` ("final alignment corrections to transport-azure-relay
dossier ( W1 )" / "apply contract corrections to transport-azure-relay
dossier"). Four of the five frozen transport/observability/activation
Providers (D049) each required a dedicated ownership-correction commit before
converging on the core-owns-ZoneLink / Provider-returns-opaque-handle split.

### F6 - Direct-broker boundary drift

D077 ("No Provider process imports/calls the broker... Providers validate/
decide semantics and call injected async typed EffectPort traits") exists
because dossier text repeatedly drifted toward direct broker/syscall/
filesystem access before landing on the EffectPort adapter boundary. The two
foundation-harmonize commits (`38ab1d4c`/`dfea5993`) and the `ACA EffectPort
traits in d2b-contracts provider-effects` commit (`54ee8df4`) exist because
individual Provider dossiers (ACA, GPU, TPM, minijail) had each independently
proposed a Provider-owned broker call site before the shared EffectPort
pattern was frozen once and propagated.

### F7 - Qualified ResourceType grammar drift

D080 froze the single qualification grammar `<provider-name>.d2bus.org.<Type>`
for Provider-specific ResourceTypes. Before that freeze, dossiers used
inconsistent forms; `git log --all --oneline -S"qualified" -- docs/specs`
shows the grammar question surfacing independently in
`caa74ca0` ("display-wayland dossier v6 cross-dossier corrections"),
`36592596` ("major scope/schema rewrite of activation-nixos dossier"), and
`83e623d7` ("add credential-managed-identity Provider dossier") before the
foundation-harmonize commits fixed the grammar as one frozen decision (D080)
referenced by every later dossier instead of re-derived per dossier.

### F8 - Open-question leakage before the shared decision register

Individual specs invented their own per-spec decision-tracking ID prefixes
instead of using one canonical register from the start:
`b1db144a32894ed7f3edbfd7dcb864d9d89fc6e1` ("resolve all 30 DR-NC items in
ADR-046-nix-configuration ( W0 )") used prefix `DR-NC-###`;
`d20e229fbf323eee6de7173095b53d0b317b94f1` ("resolve D-NETWORK-001/002/003 in
ADR-046-resources-network ( W0 )") used prefix `D-NETWORK-###`;
`b38cd984b6d42dacd6a951941b829081b6296cde` ("resolve DR-CLI-001..005 ( W1 )")
used prefix `DR-CLI-###`. Three different naming schemes (`DR-NC-`,
`D-NETWORK-`, `DR-CLI-`) for the same "decision-required" concept existed
simultaneously across three specs before consolidation into the single
`ADR-046-decision-register` D0xx numbering the README's "Decision-required
protocol" now mandates. `ADR-046-decision-register.md`'s own "Open decisions"
section ("No unresolved foundation decision is currently recorded") is the
end state; the 30+3+5 per-spec IDs that had to be resolved and folded in are
the friction this closes.

### F9 - `sourcePolicy`/host-path drift

D082 froze `sourcePolicyId` as the only way a Volume `source.settings`
references a Provider-declared host-path allowlist entry, never a raw path.
`git log --all --oneline -S"sourcePolicyId" -- docs/specs | wc -l` returns 33
matching commits; `200aaa2d`, `2af904ef`, `715ef273` ("AttachmentSpec
executionRef, read-only state views/mounts, sourcePolicyId, per-component
Volumes"), `dbc92230` ("remove SSH path; add Volume base quota fields"), and
`7ea8970b` ("activation-nixos: source.settings sourcePolicyId; dual quota;
reset destroys Volume") each independently reintroduce and then retract a
raw-path or SSH-path field on Volume `source.settings` before D082 froze the
opaque-ID indirection as the one normative form.

### F10 - Empty/no-SHA handoffs forcing duplicate reconciliation

`38ab1d4cd3697df19765d9a238764d98ff562b0d` and
`dfea59933251fbfb13eb70613642f5215bbc1f74` are byte-identical patches (same
parent `b4e0d6c7d01ef43338229a9e19e8cd98c6f057f1`, same tree, same author
timestamp) landed independently on two different branches
(`adr0046-foundation-harmonize` and the integrator branch). Likewise
`05fefffe3e434f0fa00699ebbc36f90479e0eb1c` and
`0150475f211367a74de359157b2f42960b114e53` ("harmonize Provider dossiers") are
an identical 24-file, 588-insertion/402-deletion diff landed independently on
two branches. Neither pair carries a cross-reference to the other commit's
SHA. A handoff record binding an assigned file set, a base SHA, and a test/
validation result to each corrective pass would have let the second landing
reuse the first's result instead of independently re-deriving and
re-committing the same 24-file reconciliation.

### F11 - Correction commits after branches diverged, creating avoidable conflicts

Of the 2,478 commits reachable from any `adr0046-*` branch,
`git log --all --oneline --grep="fix\|correct\|realign\|harmonize" -i | wc -l`
returns 768 (≈31%). Restricted to one representative dossier branch,
`adr0046-provider-device-gpu` has 12 commits since its divergence point, of
which `8be36a63`, `e112a9a2`, `1ac3b1d3`, `d884c272`, and `4e82fefe` (5 of 12,
≈42%) are corrective. Because the correcting commits landed on the same
already-diverged branch rather than through a rebase onto a reconciled
foundation commit, later Nix-authoring-alignment sweeps (`827e874f`/`6bcb60b3`
"align Nix authoring contracts") had to re-touch files the corrective commits
had already changed, which is exactly the shape of an avoidable cherry-pick/
rebase conflict this spec's [ADR046-streamline-010](#adr046-streamline-010--stale-basecurrent-parent-reconcile-helper)
targets.

### F12 - Manual global `rg`-shaped scans instead of generated lints

`05fefffe`/`0150475f` ("harmonize Provider dossiers") each touch 24 files (588
insertions, 402 deletions) in one commit - a full-tree sweep, not a targeted
per-dossier fix. `a7c0ceeb`'s "full architectural realignment" is the same
shape. No `packages/d2b-contract-tests/tests/policy_*.rs`-equivalent lint
exists over `docs/specs/` today (`packages/d2b-contract-tests/tests/`
contains `policy_docs.rs` and 20+ other `policy_*.rs` files, none of which
scan `docs/specs/providers/*.md` for the vocabulary/ownership/finalizer/
phase/source-policy invariants this ADR's own decision register freezes).
Every one of these sweeps had to be re-derived by reading every dossier file
rather than run as one generated check.

### F13 - Repetitive per-dossier state-Volume boilerplate

`grep -l "ProviderStateSet" docs/specs/providers/*.md | wc -l` returns 27 (all
committed Provider dossiers). `grep -c
"persistenceClass\|stateSchema\|sensitivityClass" docs/specs/providers/*.md`
sums to 314 matching lines across those 27 files (`docs/specs/providers/`
totals 44,006 lines). The same canonical state-Volume field block -
`kind: state`, `persistenceClass: persistent`, nonzero `quotaBytes`/
`maxBytes`/`maxInodes`, `identityMarker`, `sourcePolicyId`, `mounts` with
`required: true` - is hand-authored in each dossier rather than emitted from
one canonical snippet, which is also why F2/F3/F9's per-dossier drift was
possible: each hand-copy was an independent chance to diverge.

### F14 - Inability to validate Markdown-embedded YAML against canonical schemas

Every ResourceType/Provider spec's "Nix authoring and configuration cleanup"
section embeds fenced YAML/JSON `ResourceSpec` examples (README.md
"Required metadata" + "Nix authoring" sections). No tool extracts and
schema-validates those fenced blocks today; every field-name/shape drift in
F2/F3/F9 was caught by a human reviewer reading prose, not by a validator
comparing the fenced example against `ADR-046-decision-register` D057/D058's
canonical rendering rules.

### F15 - Current-source old Realm/Workload terminology confusing target authors

`ADR-046-terminology-and-identities.md` and `ADR-046-current-code-migration-map.md`
exist specifically to map old `Realm`/`Workload` vocabulary (still the live
vocabulary in this repository's own root `AGENTS.md`, e.g. "daemon-only
control plane", `d2b-realm-core`, `d2b-realm-router`) onto new `Zone`/`Guest`
vocabulary. D050 ("Rename the non-host execution parent from Workload to
Guest and add Host as a separate ResourceType") is itself evidence that an
early dossier round used `Workload` as the target-side name before the
rename; `9eba2987` ("add guest-local and host-backed-guest placement modes
for Guest state") and `af7a33fe` ("specify Zone, ZoneLink, Provider, Role,
and RoleBinding ResourceTypes") both post-date the D050 rename and had to
re-terminology-check every Guest-state reference against it.

### F16 - Tracking database not automatically bound to the git tree

This ADR's authoring session state (todo/task tracking) is not itself part of
the committed tree and carries no enforced binding to a base SHA, branch, or
commit range; the only cross-check available today is manual comparison
between a plan and `git log`. `ADR-046-decision-register`'s own
"Implementation work items" section (`ADR046-decisions-001`) has no such
binding either, and no other spec's work-item table declares one.

### F17 - Docs-only tier0 catches syntax but not schema/cross-link/work-item consistency

`tests/tools/tier0-first-pass.sh` runs `bash -n` and `shellcheck` over
`tests/`, `scripts/`, and `harness/ubuntu/*.sh` only (confirmed by reading the
script: "Pure host-local checks only: bash -n on tracked shell scripts...
shellcheck... Intentionally excludes nix eval, cargo fmt/clippy/test"). None
of the churn in F1-F14 is shell-script content; `make check-tier0` passes
unconditionally for any `docs/specs/*.md` change, including every one of the
768 correction commits counted in F11, because there is no fast documentation-
schema/cross-link/work-item-ID gate in the tier0 tranche.

## Metrics

Every metric below is derived from artifacts this spec's Tier A tooling
produces (registry, lints, handoff manifests, task-DB import) so it can be
computed mechanically, not estimated.

| Metric | Definition | Source | Current baseline (this evidence ledger) |
| --- | --- | --- | --- |
| Correction rounds | Count of commits matching `fix\|correct\|realign\|harmonize` (case-insensitive) reachable from any `adr0046-*` branch, divided by total commits in the same set | `git log --all --oneline --grep=... -i \| wc -l` over `git rev-list --all --count` restricted to `adr0046-*` refs | 768 / 2,478 ≈ 31% (see F11) |
| Schema violations | Count of fenced-YAML/JSON blocks under `docs/specs/**` failing [ADR046-streamline-003](#adr046-streamline-003--markdown-fenced-yamljson-extractor-and-schema-validator) validation, per commit | `xtask spec-schema-check --format json` violation count | Not measured before this tool exists (F14); target 0 at every commit once adopted |
| Conflict count | Count of `git rebase`/`git town sync` conflict hunks reported by [ADR046-streamline-010](#adr046-streamline-010--stale-basecurrent-parent-reconcile-helper) per dossier restack | Reconcile-helper JSON report `conflicts[]` length | Not measured before this tool exists; F1/F11 show at least 5 dossier branches diverged from a foundation point superseded by 6+ later foundation commits |
| Handoff completeness | Fraction of integrator-merged commits whose commit trailer references a [ADR046-streamline-011](#adr046-streamline-011--agent-handoff-manifest) manifest with non-empty `assigned_files`, `commit_sha`, `test_result`, and `base_sha` | Task-DB↔git import report | 0% before adoption (F10: the two identical-diff commit pairs carry no such trailer) |
| Ready/launched ratio | Count of todo-tracked scopes with all dependencies `done` ("ready"), divided by count of scopes with an open worktree/branch ("launched"), per [ADR046-streamline-013](#adr046-streamline-013--anti-serialization-readylaunchedblocker-report) | Anti-serialization report | Not measured before this tool exists |
| Time-to-green | Wall-clock time from a dossier branch's first commit to its `xtask spec-schema-check` + cross-spec lint passing with zero violations | Task-DB timestamps + lint pass timestamp | Not measured before this tool exists; F1/F11 imply multiple hours-to-days of re-derivation per affected dossier |
| Disk usage | Aggregate size of `packages/target/` and stale worktrees reported by [ADR046-streamline-018](#adr046-streamline-018--worktreedisktarget-cleanup-reporting), never auto-deleted | `du -sh` per reported path, summed | Not measured before this tool exists |

## Permanent methodology after Accepted

Once ADR 0046 reaches Accepted, the following graduate from this spec into
permanent, repository-wide methodology rather than remaining a one-time
cleanup:

| Item | Destination after Accepted | Rationale |
| --- | --- | --- |
| ADR046-streamline-001 (spec registry/graph) | New `AGENTS.md` "ADR spec-set tooling" subsection referencing `xtask spec-registry` | Every future ADR-scale spec set (not only ADR 0046) benefits from a generated dependency graph instead of manifest hand-maintenance |
| ADR046-streamline-003 (Markdown YAML/JSON extractor+validator) | `tests/AGENTS.md` Layer-1 type 5 (policy lint) entry `policy_spec_schema.rs`; permanent CI gate | Directly closes F14; becomes as permanent as any other drift gate |
| ADR046-streamline-004 (dossier scaffold/template) | `docs/specs/providers/TEMPLATE.md` plus `xtask new-provider-dossier`; referenced from `docs/specs/README.md` "Required metadata" | Every future Provider dossier (not only the 27 already committed) starts from the corrected shape, closing F13 permanently |
| ADR046-streamline-005 (cross-spec vocabulary/ownership lint) | `packages/d2b-contract-tests/tests/policy_spec_vocabulary.rs`, permanent | Directly closes F2/F5/F6/F7/F9; must never regress once frozen |
| ADR046-streamline-008 (work-item schema/ID/dependency validator) | `packages/d2b-contract-tests/tests/policy_work_items.rs`, permanent | Every future spec's work-item table is validated the same way, closing F16's cross-check gap |
| ADR046-streamline-009 (Provider catalog/index generator) | `docs/specs/ADR-046-provider-catalog.md` (generated, committed), regenerated by drift gate | Same status as any other generated-artifact drift gate in `tests/unit/gates/` |
| ADR046-streamline-010 (stale-base reconcile helper) | `AGENTS.md` "Integrator-prep-first pattern" section, extended with a `git town`-based dossier-restack subsection | F1/F11 show this is not ADR-0046-specific; any future stacked documentation wave needs it |
| ADR046-streamline-011 (agent handoff manifest) | `AGENTS.md` "Worktrees for parallel agents" section, extended with a mandatory handoff-manifest requirement | F10 is a general parallel-authoring risk, not unique to this ADR |
| ADR046-streamline-012 (task DB↔git import) | `AGENTS.md`, new short subsection cross-referencing the session DB schema | Closes F16 permanently for any future plan-driven wave |
| ADR046-streamline-013 (anti-serialization report) | `AGENTS.md`, folded into "Stacked PR workflow for large waves" as a required per-round report | Generalizes the existing informal rule into a checked artifact |
| ADR046-streamline-016 (pre-panel zero-open-decision gate) | `AGENTS.md` "Panel review" phase-gate preconditions | Closes F8 permanently: no future ADR-scale spec set reaches its panel with leaked per-spec decision-ID prefixes |
| ADR046-streamline-018 (worktree/disk/target cleanup reporting) | `AGENTS.md` "Disk hygiene contract"-equivalent section for this repository | Same operational need this repository's sibling documents already recognize for code waves |
| ADR046-streamline-019 (terminology helper) | `docs/reference/` how-to entry plus `xtask terminology-check` | F15 recurs whenever new dossiers reference current-source Realm/Workload evidence |

Tier B items (ADR046-streamline-002, -006, -007, -014, -015, -017) become
permanent only once their owning implementation crates exist and their tests
are wired into the taxonomy `tests/AGENTS.md` defines (types 2-5 as
applicable); they are listed under [Implementation work items](#implementation-work-items)
with their eventual test-type destination, not promoted early.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `tests/tools/tier0-first-pass.sh` (shell-only tier0 gate); `packages/xtask/src/main.rs` `gen-schemas`/`gen-cli-schemas`/`gen-error-codes`/`gen-daemon-api` generator pattern, now also `spec-registry` and `implementation-graph`; `packages/d2b-contract-tests/tests/policy_*.rs` (20+ existing policy lints, now including `policy_adr046_spec_literals.rs` and `policy_adr046_work_items.rs` over `docs/specs/`); `tests/tools/gen-migration-ledger.sh` (existing ledger-generation precedent); this repository's own `AGENTS.md` "Stacked PR workflow for large waves" and "Integrator-prep-first pattern (W3 onwards)" sections (precedent for prep-before-wave, not yet applied to this documentation wave, per F1) |
| Evidence class | The `xtask gen-*`/`policy_*.rs`/ledger-generation patterns are `implemented-and-reachable` for code artifacts today; the extension to `docs/specs/**` Markdown has partially landed - `xtask spec-registry` and `xtask implementation-graph` emit `ADR-046-spec-set.json`/`ADR-046-work-items.json`/`ADR-046-implementation-graph.{json,md}` and the `docs/specs/` policy lints are `implemented-and-reachable`, while the Provider-dossier scaffold and the tier0 Markdown pre-check remain `ADR-only` until their work items below land |
| Behavior retained | Generated-artifact-plus-drift-gate shape (`xtask gen-* + git diff --exit-code`); policy-lint-as-Rust-test shape; ledger-file-as-source-of-truth shape; explicit non-destructive cleanup reporting posture already required for code (`packages/d2bd` storage lifecycle "never a broad `/run/d2b` sweep") is extended here to worktree/disk cleanup |
| Required delta | `xtask spec-registry` now reads `docs/specs/**` Markdown as structured data and the generated spec registry and implementation graph exist; the remaining delta is the Provider-dossier scaffold (every one of the 27 committed dossiers was hand-authored from scratch, which is why F13's boilerplate exists) plus the `gen-provider-catalog`/`spec-schema-check` subcommands and the tier0 Markdown pre-check |
| Reuse path | The `spec-registry` and `implementation-graph` subcommands already extend `packages/xtask/src/main.rs`'s `gen-*` dispatch using the same `run_task`/drift-gate wiring proven for `gen-schemas`; the remaining `gen-provider-catalog` and `spec-schema-check` subcommands follow the same pattern; the `docs/specs/` policy lints already extend `packages/d2b-contract-tests/tests/policy_*.rs` using the same test-harness pattern the existing 20+ policy lints use |
| Replacement/deletion | Nothing is replaced; `tests/tools/tier0-first-pass.sh` gains an additional fast Markdown-schema pre-check alongside its existing `bash -n`/shellcheck checks rather than being rewritten (closes F17 additively) |
| Feasibility proof | Every Tier A item operates on data that already exists in the committed tree (the 27 dossiers, the decision register, the work-item tables); a spike is unnecessary because the extraction/validation inputs are fixed Markdown tables and fenced blocks with a documented shape (README.md "Required metadata", "Nix authoring", "Implementation work items") |
| Future owner | Work items below |

## Tests

Every test type below follows the taxonomy `tests/AGENTS.md` defines; this
spec introduces no new top-level `tests/*.sh` gate.

### Type 1 - eval cases (`tests/unit/nix/cases/`)

Not applicable: this spec's tooling operates on `docs/specs/**` Markdown and
Rust, not NixOS module evaluation.

### Type 2 - unit tests (`packages/<crate>/src/**`)

| Test | Asserts |
| --- | --- |
| `spec_registry::parse_metadata_table` | Every `docs/specs/ADR-046-*.md` and `docs/specs/providers/ADR-046-*.md` file's leading metadata table parses to the exact `Spec ID`/`Parent`/`Status`/`Version`/`Baseline`/`Owners`/`Depends on`/`Supersedes` fields README.md requires; a malformed table is a parse error naming the offending file/row |
| `spec_registry::acyclic_dependency_graph` | The `Depends on` edges across every parsed spec form a DAG; an introduced cycle fails with the exact cycle path |
| `spec_schema_check::extract_fenced_blocks` | Every fenced ` ```yaml `/` ```json ` block under a "Nix authoring and configuration cleanup" heading is extracted with its owning spec/section location preserved for error messages |
| `spec_schema_check::canonical_field_set` | A fenced ResourceSpec example whose top-level `spec` field name is not in the canonical `ResourceTypeSchema` for its declared `type` fails with the exact unknown field name (regression test seeded from the F2 `rootConfig`/`hostUid`/`hostGid` examples) |
| `work_item_validator::unique_ids` | Every `ADR046-<spec>-<ordinal>` work item ID across the whole tree is unique; a duplicate fails naming both locations |
| `work_item_validator::required_fields` | Every work item table has all eleven README.md-required fields (or this spec's nine bespoke tooling fields) non-empty and non-placeholder (`TBD`, `TODO`, `N/A` without justification text fail) |
| `decision_id_validator::single_prefix` | Every "decision-required" marker anywhere under `docs/specs/**` uses the `D0xx`/`DR0xx`-equivalent single canonical prefix this spec fixes, not a per-spec prefix (regression test seeded from the F8 `DR-NC-`/`D-NETWORK-`/`DR-CLI-` examples) |
| `provider_catalog::frozen_family_membership` | Every dossier under `docs/specs/providers/` maps to exactly one D043-D049 frozen Provider family; an unmapped or duplicate-mapped dossier fails |

### Type 3 - integration tests (`packages/<crate>/tests/*.rs`)

| Test | Asserts |
| --- | --- |
| `xtask_spec_registry_regenerates_clean` | Running `cargo run -p xtask -- spec-registry` twice in a row produces byte-identical output (determinism) |
| `xtask_new_provider_dossier_scaffold` | `cargo run -p xtask -- new-provider-dossier --name <x>` emits a dossier skeleton whose metadata table, "Nix authoring" section, `src`/`tests`/`integration`/`README.md` work-item rows, and "Implementation work items" heading are all present and pass `spec_schema_check`/`work_item_validator` on first generation |
| `reconcile_helper_reports_stale_base` | Given two fixture branches sharing a synthetic foundation commit where one branch's tip is behind the foundation's current tip, the reconcile helper reports the exact commit range the stale branch is missing, without performing any git mutation |
| `handoff_manifest_rejects_incomplete_record` | A handoff manifest JSON missing any of `assigned_files`/`commit_sha`/`test_result`/`base_sha` is rejected with the exact missing field named |

### Type 4 - contract tests (`packages/d2b-contract-tests/tests/*.rs`)

| Test | Asserts |
| --- | --- |
| `spec_registry_json_schema_matches_doc` | The generated `docs/specs/ADR-046-spec-set.json`/`ADR-046-work-items.json` shape matches this and the parent `docs/specs/README.md`'s documented fields exactly (drift gate) |
| `provider_catalog_matches_frozen_families` | The generated `docs/specs/ADR-046-provider-catalog.md` table matches `ADR-046-decision-register` D043-D049 membership exactly |

### Type 5 - policy lints (`packages/d2b-contract-tests/tests/policy_*.rs`)

| Test | Asserts |
| --- | --- |
| `policy_spec_vocabulary` | No `docs/specs/**` file uses a Provider-specific ResourceType name outside the D080 `<provider-name>.d2bus.org.<Type>` grammar; no file authors a numeric `hostUid`/`hostGid` field on a public spec (F2/F7 regression guard) |
| `policy_spec_ownership` | No dossier other than `volume-local` writes Volume layout/spec/ownership fields (D083); no transport dossier reads/writes/finalizes `ZoneLink` (D081) (F4/F5 regression guard) |
| `policy_spec_effectport_boundary` | No Provider dossier's detailed-design section names a direct broker/syscall/filesystem/systemd-socket call; every privileged effect is expressed as an injected typed `EffectPort` call (D077) (F6 regression guard) |
| `policy_spec_source_policy` | No Volume `source.settings` example anywhere under `docs/specs/**` contains a raw absolute host path; every `local-path`/`block-image` source example uses `sourcePolicyId` (D082) (F9 regression guard) |
| `policy_spec_finalizer_phase` | Every ResourceType/Provider spec's status section uses only the common phase enum `Pending|Ready|Succeeded|Degraded|Failed|Deleted|Unknown` (D037) and the fixed finalizer/deletion ordering (D084); no spec invents a parallel phase or deletion-ordering scheme |
| `policy_no_leaked_decision_prefix` | No file under `docs/specs/**` contains a "decision-required" marker using any ID prefix other than the canonical `ADR-046-decision-register` `D0xx` numbering (F8 regression guard) |
| `policy_test_placement` | Every `src/` `#[cfg(test)]` module and crate `tests/*.rs` file is hermetic (D094): no process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build, KVM, hardware, live cloud, or non-tiny filesystem tree, and no `#[ignore]`; such needs must move to `integration/` |
| `policy_test_determinism` | No hermetic-tier test uses wall-clock sleep/retry (D094); a deterministic fake clock/RNG is used instead, except for explicitly allow-listed crypto/property tests with a declared per-test budget and capped case count |

### Type 6 - flake checks (`tests/unit/smoke/`)

Not applicable: no NixOS module surface is introduced.

### Type 9 - container (`tests/integration/containers/`, `make test-integration`)

| Test | Asserts |
| --- | --- |
| `spec-tooling-fresh-clone.sh` | A rootless-podman fixture clones a synthetic fixture repo shaped like `docs/specs/`, runs `spec-registry`/`spec-schema-check`/`policy_spec_*` against it, and confirms all four Tier A generators/lints run without any dependency beyond the pinned toolchain |

### Type 10-12

Not applicable: Tier A tooling requires no VM, live host, or hardware; Tier B
items requiring these are specified per-item in
[Implementation work items](#implementation-work-items).

## Implementation work items

Each item below uses the bespoke field set this meta-spec requires instead of
the standard ResourceType/Provider work-item table, because none of these
items reuse or replace v3 runtime code - they are new tooling with no current
v3 source to extract from.

### ADR046-streamline-001 - Generated spec registry, dependency graph, and implementation DAG

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-001` |
| Tier | A (shared prep) |
| Observed friction evidence | F1, F8, F11: dossier branches diverged from a superseded foundation commit with no generated way to detect it; three different per-spec decision-ID prefixes existed with no generated cross-reference; launch order and parallelism were re-derived from prose per round |
| Desired behavior | A generated, deterministic JSON registry enumerating every `docs/specs/ADR-046-*.md` and `docs/specs/providers/ADR-046-*.md` file's metadata-table fields and `Depends on` edges, rendered as an acyclic dependency graph; and, per D095, a generated machine-readable implementation DAG (`ADR-046-implementation-graph.json` + human view `ADR-046-implementation-graph.md`) mapping every member spec and every work item to a `W0`-`W7` launch wave, a file-disjoint parallel group, typed edges, and a topological rank |
| Destination | `docs/specs/ADR-046-spec-set.json`, `docs/specs/ADR-046-work-items.json`, `docs/specs/ADR-046-implementation-graph.json`, `docs/specs/ADR-046-implementation-graph.md` (generated, committed non-member artifacts; named in `docs/specs/README.md`) |
| Owner/dependencies | `packages/xtask` owner; no dependency on any other streamline item |
| Dependency/owner | `packages/xtask` owner; no dependency on any other streamline item |
| Current source | The generators have since landed in this repository: `packages/xtask/src/gen_spec_set.rs` (`xtask spec-registry`) and `packages/xtask/src/implementation_graph.rs` (`xtask implementation-graph`) emit the committed `ADR-046-spec-set.json`, `ADR-046-work-items.json`, and `ADR-046-implementation-graph.{json,md}`, regenerated and `git diff --exit-code`d by the `tests/unit/gates/drift-check.sh` drift gate and run in Layer 1; there is no pre-ADR45 baseline to extract from, so remaining effort is hardening, not creation |
| Reuse action | create |
| Implementation shape | New `cargo run -p xtask -- spec-registry` subcommand parsing every spec's metadata table + work-item tables with a Markdown-table parser, emitting `ADR-046-spec-set.json` and `ADR-046-work-items.json`; a companion `cargo run -p xtask -- implementation-graph` reads those two manifests plus the `ADR-046-validation-and-delivery.md` §3 wave topology and emits `ADR-046-implementation-graph.json` and its rendered `.md`; both wired into the existing `gen-*`/`run_task` dispatch pattern in `packages/xtask/src/main.rs` |
| Detailed design | New `cargo run -p xtask -- spec-registry` subcommand parsing every spec's metadata table + work-item tables with a Markdown-table parser, emitting the two manifest JSON files; the `implementation-graph` generator then maps every member spec and every work item exactly once to a wave and file-disjoint parallel group, emits typed edges (`spec-depends-on`, `work-item-depends-on`, `implements-spec`, `shared-contract`, `file-overlap-order`), computes topological rank, and renders the Mermaid/table human view; output is deterministic with sorted keys and no timestamps or host paths |
| Integration | `tests/unit/gates/` drift gate (`xtask spec-registry`/`xtask implementation-graph` + `git diff --exit-code`) added to the existing drift-gate set; the graph is consumed by ADR046-streamline-005/006/008/013 and the ready-wave query in `ADR-046-validation-and-delivery` instead of each re-parsing Markdown independently |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `spec_registry::parse_metadata_table`, `spec_registry::acyclic_dependency_graph`, `xtask_spec_registry_regenerates_clean`, `spec_registry_json_schema_matches_doc`, `implementation_graph::every_spec_and_work_item_mapped_once`, `implementation_graph::acyclic_and_wave_monotonic`, `implementation_graph::parallel_groups_are_file_disjoint`, `implementation_graph_regenerates_clean` |
| Adoption timing | Immediately; first Tier A item, since every other generator/lint below consumes its output |
| Removal/supersession | None; this is the foundational generator for the remaining items |
| Removal proof | None - net-new; no prior owner to remove; this is the foundational generator for the remaining items |

### ADR046-streamline-002 - Canonical schema/snippet generator

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-002` |
| Tier | A for the doc-level canonical field-set source; B for generating from real Rust `ResourceTypeSchema` types once they exist |
| Observed friction evidence | F2, F13: `rootConfig`/`hostUid`/`hostGid`/custom readiness-restart fields were independently invented per dossier; 314 lines of near-identical state-Volume boilerplate exist across 27 dossiers |
| Desired behavior | One canonical, versioned field-set definition per primitive (`Provider`, `Process`, `EphemeralProcess`, `Volume`, `ProviderStateSet` extension fields, qualified-ref grammar) that every dossier's fenced example is generated from or checked against, instead of hand-copied |
| Destination | `docs/specs/schemas/*.schema.json` (Tier A: hand-authored-once canonical source checked into the tree, matching the frozen D010/D032/D075/D076/D080 decisions exactly); `packages/d2b-core/src/resource_schema/*.rs` (Tier B: the eventual Rust source of truth once ResourceType implementation exists, at which point the Tier A JSON becomes generated from Rust instead of hand-authored) |
| Owner/dependencies | ADR046-streamline-001; `d2b-core` owner (Tier B only) |
| Dependency/owner | ADR046-streamline-001; `d2b-core` owner (Tier B only) |
| Current source | Decision-register D010/D032/D075/D076/D080 prose is the Tier A source; no existing generated ResourceType schema source until Tier B |
| Reuse action | create |
| Implementation shape | Tier A: author the schema JSON once per primitive directly from the already-frozen decision-register entries (D010, D032, D075, D076, D080); Tier B: `xtask gen-spec-schemas` derives the same JSON from real Rust `#[derive(JsonSchema)]`-equivalent types once they land, replacing the hand-authored Tier A source without changing its consumers |
| Detailed design | Tier A: author the schema JSON once per primitive directly from the already-frozen decision-register entries (D010, D032, D075, D076, D080); Tier B: `xtask gen-spec-schemas` derives the same JSON from real Rust `#[derive(JsonSchema)]`-equivalent types once they land, replacing the hand-authored Tier A source without changing its consumers Primary reuse disposition: `create`. Preserved source-plan detail: net-new Tier A schema source; later replace source with generated output from real d2b-core ResourceType types without changing consumers. |
| Integration | Consumed by ADR046-streamline-003 as the validation target and by ADR046-streamline-004's scaffold as the snippet source |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `spec_schema_check::canonical_field_set` |
| Adoption timing | Tier A ships now; Tier B graduation happens only after the owning implementation work item for `d2b-core` resource schemas reaches Validation-complete |
| Removal/supersession | Tier A hand-authored JSON is superseded (not deleted) by Tier B generated JSON once real Rust types exist; consumers are unaffected since the file shape is identical |
| Removal proof | Tier A hand-authored JSON is superseded (not deleted) by Tier B generated JSON once real Rust types exist; consumers are unaffected since the file shape is identical |

### ADR046-streamline-003 - Markdown fenced-YAML/JSON extractor and schema validator

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-003` |
| Tier | A |
| Observed friction evidence | F14: no tool extracts and validates fenced ResourceSpec examples against a canonical schema; every field-name drift in F2/F3/F9 was caught by manual review |
| Desired behavior | Every fenced ` ```yaml `/` ```json ` block under a spec's "Nix authoring and configuration cleanup" heading is extracted, matched to its declared `type`, and schema-validated against ADR046-streamline-002's canonical field set, with documented exclusions for blocks explicitly marked as illustrating a rejected/historical shape (current-evidence exclusions, e.g. the "current-code fit" tables' own citation blocks are not ResourceSpec examples and are excluded by heading context, not by an ad hoc allowlist) |
| Destination | `packages/xtask/src/bin/spec_schema_check.rs`; wired as a Layer-1 policy lint at `packages/d2b-contract-tests/tests/policy_spec_schema.rs` |
| Owner/dependencies | ADR046-streamline-001, ADR046-streamline-002 |
| Dependency/owner | ADR046-streamline-001, ADR046-streamline-002 |
| Current source | None - net-new spec Markdown/schema lint; no existing fenced ResourceSpec extractor |
| Reuse action | create |
| Implementation shape | Reuse the CommonMark parser already available to the Rust toolchain (the same class of dependency `xtask`'s existing `gen-*` commands already use for structured generation) to walk fenced blocks; classify each block by its enclosing heading (`## Nix authoring and configuration cleanup` vs. any other heading) to apply the current-evidence exclusion without a manual per-block marker |
| Detailed design | Reuse the CommonMark parser already available to the Rust toolchain (the same class of dependency `xtask`'s existing `gen-*` commands already use for structured generation) to walk fenced blocks; classify each block by its enclosing heading (`## Nix authoring and configuration cleanup` vs. any other heading) to apply the current-evidence exclusion without a manual per-block marker Primary reuse disposition: `create`. Preserved source-plan detail: net-new; reuse only the established xtask dispatch pattern and Rust Markdown-parser dependency class. |
| Integration | Added to `tests/tools/tier0-first-pass.sh` as an additional fast pre-check (closes F17) and to the standing `packages/d2b-contract-tests` policy-lint suite |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `spec_schema_check::extract_fenced_blocks`, `spec_schema_check::canonical_field_set`, `policy_spec_schema` (Type 4/5) |
| Adoption timing | Immediately, before the ADR 0046 documentation set's own pre-panel gate, so any remaining spec iteration is caught before, not after, panel |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-004 - Provider dossier scaffold/template

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-004` |
| Tier | A |
| Observed friction evidence | F13: 27 dossiers each hand-authored the same ProviderStateSet/Volume boilerplate (314 duplicated lines) and D059's `src`/`tests`/`integration`/`README.md` requirement from scratch |
| Desired behavior | A committed template enumerating every required section (metadata table, "Nix authoring and configuration cleanup", `src`/`tests`/`integration`/`README.md` work-item rows per D059, "Current-code fit", "Tests", "Implementation work items") and the canonical state-Volume snippet from ADR046-streamline-002, so a new or corrected dossier starts compliant instead of converging over several correction commits |
| Destination | `docs/specs/providers/TEMPLATE.md` (committed, non-normative reference); `packages/xtask/src/bin/new_provider_dossier.rs` (`cargo run -p xtask -- new-provider-dossier --name <provider-name>`) |
| Owner/dependencies | ADR046-streamline-001, ADR046-streamline-002 |
| Dependency/owner | ADR046-streamline-001, ADR046-streamline-002 |
| Current source | None - net-new provider-dossier scaffold; canonical snippets come from ADR046-streamline-002 |
| Reuse action | create |
| Implementation shape | Scaffold generator emits the template pre-filled with the requesting Provider's name, D080 qualification-grammar examples, and the canonical state-Volume snippet; does not attempt to author Provider-specific semantic sections (those remain the dossier author's normative content) |
| Detailed design | Scaffold generator emits the template pre-filled with the requesting Provider's name, D080 qualification-grammar examples, and the canonical state-Volume snippet; does not attempt to author Provider-specific semantic sections (those remain the dossier author's normative content) Primary reuse disposition: `create`. Preserved source-plan detail: net-new scaffold; reuse canonical schema/snippet source from ADR046-streamline-002. |
| Integration | Referenced from `docs/specs/README.md` "Required metadata" section (a follow-up doc edit outside this task's scope, tracked as a required cross-reference) |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `xtask_new_provider_dossier_scaffold`; the emitted scaffold must independently pass `spec_schema_check`/`work_item_validator` on first generation |
| Adoption timing | Immediately; used for any future new Provider dossier and for re-basing an existing dossier onto the corrected shape during its next revision |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-005 - Cross-spec vocabulary/ownership/finalizer/phase/source-policy lint

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-005` |
| Tier | A |
| Observed friction evidence | F2, F4, F5, F7, F9: qualified-ResourceType grammar drift, Volume-vs-virtiofs ownership conflicts, ZoneLink ownership drift, sourcePolicy/path drift, all caught by manual full-tree sweeps (F12) rather than a generated check |
| Desired behavior | One Rust policy lint enumerating and enforcing every frozen cross-cutting invariant already in the decision register (D080 grammar, D081 ZoneLink ownership, D082 sourcePolicyId, D083 Volume/virtiofs ownership, D084 finalizer ordering, D037 phase enum) across all committed specs in one pass |
| Destination | `packages/d2b-contract-tests/tests/policy_spec_vocabulary.rs`, `policy_spec_ownership.rs`, `policy_spec_finalizer_phase.rs`, `policy_spec_source_policy.rs` |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | Decision-register D080/D081/D082/D083/D084/D037 invariants and ADR046-streamline-001 registry output; no existing cross-spec lint |
| Reuse action | create |
| Implementation shape | Four focused lint files (one invariant family per file, matching the existing `packages/d2b-contract-tests/tests/policy_*.rs` one-concern-per-file convention) each scanning the registry output plus raw Markdown text for the specific violation patterns named in F2/F4/F5/F7/F9 |
| Detailed design | Four focused lint files (one invariant family per file, matching the existing `packages/d2b-contract-tests/tests/policy_*.rs` one-concern-per-file convention) each scanning the registry output plus raw Markdown text for the specific violation patterns named in F2/F4/F5/F7/F9 Primary reuse disposition: `create`. Preserved source-plan detail: net-new policy lints over generated registry and raw Markdown. |
| Integration | Standing Layer-1 policy-lint suite; runs on every PR touching `docs/specs/**` |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `policy_spec_vocabulary`, `policy_spec_ownership`, `policy_spec_finalizer_phase`, `policy_spec_source_policy` (all Type 5) |
| Adoption timing | Immediately, before the ADR 0046 documentation set's own pre-panel gate |
| Removal/supersession | None; graduates to permanent per [Permanent methodology](#permanent-methodology-after-accepted) |
| Removal proof | None - net-new; no prior owner to remove; graduates to permanent per [Permanent methodology](#permanent-methodology-after-accepted) |

### ADR046-streamline-006 - ProviderStateSet/status-first/one-controller graph checker

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-006` |
| Tier | B |
| Observed friction evidence | F3: ProviderStateSet churned through multiple representations and the bootstrap-cycle exception was independently re-scoped three times (`5c287f51`, `7632ebec`, `24598e5c`) before D076/D086 froze it; D087 later removed the mandatory-state model in favor of a status-first, optional state Volume |
| Desired behavior | A runtime-level checker that, given a Zone's actual resource/controller registration graph, verifies every Provider component's state Volumes are *declared* (a component with no declared namespace owns no state Volume and no empty identity-only Volume exists), no ResourceType or stored row named `ProviderStateSet` exists, and no bootstrap state Volume or bootstrap-storage mechanism exists (the fixed `volume-local`/`system-core`/`system-minijail` bootstrap set declares no state Volume and reaches Ready from `status`/the core Operation ledger per D087) |
| Destination | `packages/d2b-resource-store-redb/tests/provider_state_graph.rs` (or the eventual crate implementing Zone resource storage) |
| Owner/dependencies | The Zone resource-store implementation work item (not yet filed; blocked on `ADR046-W0`-`ADR046-W8` implementation request per D024); ADR046-streamline-001 for the doc-level invariant source |
| Dependency/owner | The Zone resource-store implementation work item (not yet filed; blocked on `ADR046-W0`-`ADR046-W8` implementation request per D024); ADR046-streamline-001 for the doc-level invariant source |
| Current source | None - real Zone resource-store/controller-registration graph not implemented yet; doc-level invariant source is ADR046-streamline-005 |
| Reuse action | create |
| Implementation shape | A graph-walk over the real controller-registration/resource-ownership index (not Markdown) asserting the D076/D086/D087 invariants; the doc-level half of this check (dossier text describing the invariant correctly) is covered now by ADR046-streamline-005's `policy_spec_ownership` |
| Detailed design | A graph-walk over the real controller-registration/resource-ownership index (not Markdown) asserting the D076/D086/D087 invariants; the doc-level half of this check (dossier text describing the invariant correctly) is covered now by ADR046-streamline-005's `policy_spec_ownership` Primary reuse disposition: `create`. Preserved source-plan detail: net-new future runtime graph checker. |
| Integration | Runs as a Type 3 integration test against the real resource-store crate once it exists |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New test asserting: zero `ProviderStateSet` rows in the store; every state Volume corresponds to a declared component namespace; no empty identity-only Volume; no bootstrap state Volume or bootstrap-storage mechanism exists |
| Adoption timing | Streamline wave (Tier B); lands after the Zone resource-store and core-controller implementation work items reach Validation-complete |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-007 - EffectPort/broker and worker-bus boundary lint

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-007` |
| Tier | B |
| Observed friction evidence | F6: dossier text repeatedly drifted toward direct broker/syscall access before D077's EffectPort pattern was frozen and propagated |
| Desired behavior | A source-level lint (the same shape as the existing `packages/d2b-contract-tests/tests/policy_broker_dispositions.rs`/`policy_broker_schema.rs`) scanning real Provider crate source for any direct broker import, raw socket/DTO use, or unmediated host path/device/systemd-socket open, and for any Worker binary reaching for a `ResourceClient`/d2b-bus/Credential/CLI/child-spawn capability D078 reserves to controllers/services |
| Destination | `packages/d2b-contract-tests/tests/policy_effectport_boundary.rs`, `policy_worker_bus_boundary.rs` |
| Owner/dependencies | The Provider-toolkit/EffectPort implementation work item (not yet filed; blocked on the `ADR046-W0`-`ADR046-W8` implementation request); ADR046-streamline-001 for the doc-level invariant source |
| Dependency/owner | The Provider-toolkit/EffectPort implementation work item (not yet filed; blocked on the `ADR046-W0`-`ADR046-W8` implementation request); ADR046-streamline-001 for the doc-level invariant source |
| Current source | Existing policy-broker lint pattern in packages/d2b-contract-tests/tests/policy_broker_dispositions.rs and policy_broker_schema.rs; no Provider crate source exists yet |
| Reuse action | adapt |
| Implementation shape | Static source scan (import-graph/symbol-use analysis) over compiled Provider crates, mirroring the existing `policy_broker_dispositions.rs` pattern already proven against `packages/d2bd`/`packages/d2b-priv-broker` |
| Detailed design | Static source scan (import-graph/symbol-use analysis) over compiled Provider crates, mirroring the existing `policy_broker_dispositions.rs` pattern already proven against `packages/d2bd`/`packages/d2b-priv-broker` Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt existing policy-broker lint pattern to Provider/Worker boundary checks. |
| Integration | Standing Layer-1 policy lint once Provider crates exist |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New tests asserting zero direct-broker-import matches in any Provider crate; zero disallowed-capability matches in any Worker binary |
| Adoption timing | Streamline wave (Tier B); lands after the first Provider-toolkit implementation work item reaches Validation-complete, so there is real Provider crate source to lint |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-008 - Work-item schema and unique-ID/dependency validator

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-008` |
| Tier | A |
| Observed friction evidence | F8, F16: three separate per-spec decision-ID prefixes existed simultaneously with no generated cross-check; no work-item table is validated for required-field completeness or ID uniqueness today |
| Desired behavior | Every `ADR046-<spec>-<ordinal>` work item ID is unique across the whole tree; every table has all required fields (the eleven README.md fields, or this spec's nine bespoke fields) non-empty and non-placeholder; every `Dependency/owner` reference to another work item ID resolves |
| Destination | `packages/d2b-contract-tests/tests/policy_work_items.rs` |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | None as a reuse source - net-new validator; `packages/d2b-contract-tests/tests/policy_adr046_work_items.rs` has since landed and enforces this, so the remaining effort is hardening, not creation |
| Reuse action | create |
| Implementation shape | Consumes `docs/specs/ADR-046-work-items.json` (generated by ADR046-streamline-001); flags duplicate IDs, missing/placeholder fields, and dangling cross-references |
| Detailed design | Consumes `docs/specs/ADR-046-work-items.json` (generated by ADR046-streamline-001); flags duplicate IDs, missing/placeholder fields, and dangling cross-references Primary reuse disposition: `create`. Preserved source-plan detail: net-new validator consuming ADR046-streamline-001 output. |
| Integration | Standing Layer-1 policy lint |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `work_item_validator::unique_ids`, `work_item_validator::required_fields` |
| Adoption timing | Immediately, before the ADR 0046 documentation set's own pre-panel gate (D014's "set cannot become Accepted while it contains... a work item without exact v3 source and future destination paths" precondition) |
| Removal/supersession | None; graduates to permanent |
| Removal proof | None - net-new; no prior owner to remove; graduates to permanent |

### ADR046-streamline-009 - Provider catalog/index generator

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-009` |
| Tier | A |
| Observed friction evidence | F7, F15: no single generated index of the D043-D049 frozen Provider families exists; cross-dossier consistency checks (e.g. "is this the fourth or fifth interaction Provider") are done by manual counting today |
| Desired behavior | A generated, committed table listing every frozen Provider family (D043-D049), its dossier file, its ResourceTypes owned, and its qualification-grammar examples, kept in sync with the dossier tree by a drift gate |
| Destination | `docs/specs/ADR-046-provider-catalog.md` (generated, committed) |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | None - no existing generated Provider catalog/index |
| Reuse action | create |
| Implementation shape | `cargo run -p xtask -- gen-provider-catalog` reads the registry and decision-register D043-D049 rows and renders the table; drift-gated like the existing `gen-schemas`/`gen-migration-ledger` pattern |
| Detailed design | `cargo run -p xtask -- gen-provider-catalog` reads the registry and decision-register D043-D049 rows and renders the table; drift-gated like the existing `gen-schemas`/`gen-migration-ledger` pattern Primary reuse disposition: `create`. Preserved source-plan detail: net-new generator consuming registry and decision-register rows. |
| Integration | `tests/unit/gates/` drift gate |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `provider_catalog::frozen_family_membership`, `provider_catalog_matches_frozen_families` |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-010 - Stale-base/current-parent reconcile helper

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-010` |
| Tier | A |
| Observed friction evidence | F1, F11: every checked dossier branch diverged at the same superseded foundation commit; corrective commits landed on already-diverged branches instead of through a clean restack, producing avoidable Nix cherry-pick conflicts in later Nix-authoring-alignment sweeps |
| Desired behavior | A read-only helper reporting, for a given dossier branch, the exact commit range between its divergence point and the current foundation-spec tip, and whether any of those commits touch a file the branch also touches (a likely-conflict signal), without performing any git mutation itself |
| Destination | `tests/tools/reconcile-stale-base.sh` (reporting only) plus a documented `git town sync`/`git town` restack procedure this report feeds into, since this repository does not yet use Git Town and F1/F11 show plain rebase/cherry-pick was insufficient to prevent duplicate reconciliation |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | None - no existing stale-base/current-parent reconcile helper |
| Reuse action | adapt |
| Implementation shape | `git merge-base <branch> <foundation-tip>` plus `git diff --name-only` intersection reporting; emits a JSON report (branch, divergence SHA, commits-behind count, file-overlap list) consumed by ADR046-streamline-013's ready/launched/blocker report |
| Detailed design | `git merge-base <branch> <foundation-tip>` plus `git diff --name-only` intersection reporting; emits a JSON report (branch, divergence SHA, commits-behind count, file-overlap list) consumed by ADR046-streamline-013's ready/launched/blocker report Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new read-only git report. |
| Integration | Referenced from `AGENTS.md` once graduated (see [Permanent methodology](#permanent-methodology-after-accepted)); used manually before opening or restacking any future dossier branch |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `reconcile_helper_reports_stale_base` |
| Adoption timing | Immediately; this is the single highest-value item given F1 shows 100% of checked dossier branches were affected |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-011 - Agent handoff manifest

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-011` |
| Tier | A |
| Observed friction evidence | F10: two independent byte-identical 1-commit and 24-file reconciliation passes landed on separate branches with no cross-reference to each other's SHA or test result |
| Desired behavior | Every parallel-authoring handoff (a sub-agent or contributor returning a completed spec/dossier slice to the integrator) is accompanied by a manifest naming the exact assigned file set, the commit SHA it produced, the validation/test result obtained, and the base SHA it started from, so a second corrective pass can detect it is duplicating already-reconciled work instead of re-deriving it |
| Destination | `packages/xtask/src/bin/handoff_manifest.rs` (schema/validator only); manifest instances are per-round artifacts referenced from PR bodies, not committed to the tree (consistent with this repository's "Screenshot and visual artifact hygiene"-style external-evidence posture) |
| Owner/dependencies | None - no prerequisite work item; owned by this spec |
| Dependency/owner | No prerequisite; `packages/xtask` schema/validator owner |
| Current source | None - no existing agent handoff manifest schema or validator |
| Reuse action | create |
| Implementation shape | A small JSON schema (`assigned_files: [string]`, `commit_sha: string`, `test_result: {command, exit_code}`, `base_sha: string`) plus a validator rejecting incomplete records; no attribution field for any AI/tool/model, consistent with this repository's commit/PR-body attribution rule |
| Detailed design | A small JSON schema (`assigned_files: [string]`, `commit_sha: string`, `test_result: {command, exit_code}`, `base_sha: string`) plus a validator rejecting incomplete records; no attribution field for any AI/tool/model, consistent with this repository's commit/PR-body attribution rule Primary reuse disposition: `create`. Preserved source-plan detail: net-new schema and validator. |
| Integration | Referenced from `AGENTS.md` "Worktrees for parallel agents" once graduated |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `handoff_manifest_rejects_incomplete_record` |
| Adoption timing | Immediately, for any remaining ADR 0046 spec-set round and for the future `ADR046-W0`-`ADR046-W8` implementation phase's parallel scopes |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-012 - Task DB↔git consistency import

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-012` |
| Tier | A |
| Observed friction evidence | F16: authoring session task tracking carries no enforced binding to the git tree; the only available cross-check today is manual comparison |
| Desired behavior | An importer that reads the session task-tracking database and cross-checks every "done" task against a resolvable commit SHA reachable from the current branch, flagging any "done" task with no corresponding commit and any commit range with no corresponding tracked task |
| Destination | `tests/tools/import-task-db-consistency.sh` |
| Owner/dependencies | ADR046-streamline-011 (shares the manifest schema for commit-SHA binding) |
| Dependency/owner | ADR046-streamline-011 (shares the manifest schema for commit-SHA binding) |
| Current source | None - no existing task DB to git consistency importer |
| Reuse action | create |
| Implementation shape | Reads the session database's todo table, resolves each `done` row's expected file set against `git log --name-only` for the current branch, and reports mismatches; read-only, no database or git mutation |
| Detailed design | Reads the session database's todo table, resolves each `done` row's expected file set against `git log --name-only` for the current branch, and reports mismatches; read-only, no database or git mutation Primary reuse disposition: `create`. Preserved source-plan detail: net-new read-only importer/checker. |
| Integration | Run manually at the end of each authoring round and before any pre-panel gate |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New test seeding a synthetic task DB with one orphaned "done" row and confirming the importer flags it |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-013 - Anti-serialization ready/launched/blocker report

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-013` |
| Tier | A |
| Observed friction evidence | F1, F11: dossier branches were opened against a stale foundation and left to accumulate independent correction commits rather than being promptly restacked once the foundation moved, which is exactly the serialization failure mode this repository's own "Stacked PR workflow for large waves" section warns against for code waves |
| Desired behavior | A report enumerating every dossier/spec scope's readiness (dependencies satisfied), launch state (open worktree/branch), and any recorded blocker, so a scope that is ready but not launched is visible immediately rather than discovered after correction commits have already accumulated |
| Destination | `tests/tools/anti-serialization-report.sh` |
| Owner/dependencies | ADR046-streamline-001, ADR046-streamline-010 |
| Dependency/owner | ADR046-streamline-001, ADR046-streamline-010 |
| Current source | None - no existing anti-serialization ready/launched/blocker report |
| Reuse action | adapt |
| Implementation shape | Cross-references the spec registry's dependency graph (ADR046-streamline-001) against the set of currently open `adr0046-*` branches and the reconcile helper's (ADR046-streamline-010) staleness report; emits a per-scope ready/launched/blocked classification |
| Detailed design | Cross-references the spec registry's dependency graph (ADR046-streamline-001) against the set of currently open `adr0046-*` branches and the reconcile helper's (ADR046-streamline-010) staleness report; emits a per-scope ready/launched/blocked classification Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new report combining registry, branch, and staleness data. |
| Integration | Run at the start of each authoring round and after any foundation-spec change |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New test seeding a synthetic registry + branch list with one ready-but-unlaunched scope and confirming it is reported |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-014 - Bounded parallel test/fake-dependency harness

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-014` |
| Tier | B |
| Observed friction evidence | F11: corrective commits on already-diverged branches had no fast way to verify only their own scope was affected without running the full doc/Rust suite; this generalizes to the future implementation phase where per-Provider-crate test runs must not require every other Provider crate to build first |
| Desired behavior | A bounded-parallelism test harness that can run one Provider crate's (or one dossier's) tests/lints against fake/stub dependencies for every other declared Provider, so a single scope's correction round does not require building or testing the entire `ADR046-W0`-`ADR046-W8` implementation surface |
| Destination | `tests/tools/run-layer.sh` extension (this repository already has `tests/tools/run-layer.sh` and `layer1-jobs.py` bounded-parallelism precedent) plus fake `EffectPort`/`ResourceClient` stub crates under `packages/d2b-provider-toolkit-fakes/` |
| Owner/dependencies | The Provider-toolkit implementation work item (blocked on the `ADR046-W0`-`ADR046-W8` implementation request) |
| Dependency/owner | The Provider-toolkit implementation work item (blocked on the `ADR046-W0`-`ADR046-W8` implementation request) |
| Current source | Existing bounded-parallel test-runner precedent in tests/tools/run-layer.sh and layer1-jobs.py; Provider toolkit fake crates are net-new |
| Reuse action | adapt |
| Implementation shape | Extend the existing `layer1-jobs.py` bounded-parallel-shard pattern with a per-Provider-crate shard definition; fake dependency crates implement the same `EffectPort`/`ResourceClient` trait surface with in-memory stand-ins |
| Detailed design | Extend the existing `layer1-jobs.py` bounded-parallel-shard pattern with a per-Provider-crate shard definition; fake dependency crates implement the same `EffectPort`/`ResourceClient` trait surface with in-memory stand-ins Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt bounded-parallel shard pattern; add net-new fake EffectPort/ResourceClient crates. |
| Integration | `make test-unit`/`make check` shard addition once Provider crates exist |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New integration test proving a single Provider crate's test run succeeds with zero other Provider crates built |
| Adoption timing | Streamline wave (Tier B); lands after the Provider-toolkit implementation work item reaches Validation-complete |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-015 - Conflict-aware generated-artifact regeneration

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-015` |
| Tier | A for the doc-level generators (001, 002 Tier A, 009); B for regenerating real Rust-derived artifacts once implementation code exists |
| Observed friction evidence | F1, F11, F12: independent branches each re-derived the same 24-file reconciliation by hand instead of one branch regenerating the shared artifact and the others rebasing onto it |
| Desired behavior | Every generator this spec introduces (registry, provider catalog, schema JSON) detects when its own output would conflict with a concurrent branch's uncommitted regeneration (by comparing input hashes) and reports the conflict instead of silently overwriting, so two branches never independently re-derive the same generated artifact |
| Destination | Shared `packages/xtask` regeneration-conflict-detection helper consumed by every `gen-*`/`spec-registry` subcommand |
| Owner/dependencies | ADR046-streamline-001, ADR046-streamline-009, ADR046-streamline-002 (Tier A part) |
| Dependency/owner | ADR046-streamline-001, ADR046-streamline-009, ADR046-streamline-002 (Tier A part) |
| Current source | Existing packages/xtask gen-* and drift-gate pattern; no existing generated-artifact conflict detector |
| Reuse action | adapt |
| Implementation shape | Each generator hashes its input set before writing; if a concurrent regeneration on a sibling branch (detected via the reconcile helper's file-overlap report) would produce a different hash for the same output path, the generator refuses to overwrite and reports the divergent input set |
| Detailed design | Each generator hashes its input set before writing; if a concurrent regeneration on a sibling branch (detected via the reconcile helper's file-overlap report) would produce a different hash for the same output path, the generator refuses to overwrite and reports the divergent input set Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt existing generator/drift-gate pattern with net-new input-hash conflict detection. |
| Integration | Wired into every drift gate this spec and ADR046-streamline-001/009 introduce |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New test simulating two divergent input sets producing conflicting output for the same generated file and confirming the conflict is reported, not silently resolved |
| Adoption timing | Tier A part ships alongside ADR046-streamline-001/009; the Tier B part (real generated Rust artifacts) ships in the streamline wave once code generation exists |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-016 - Pre-panel zero-open-decision gate

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-016` |
| Tier | A |
| Observed friction evidence | F8: three per-spec decision-ID prefixes leaked into three specs before consolidation; D014/README.md already require zero unresolved decisions before panel, but no automated gate checks it today |
| Desired behavior | A gate that fails closed if any file under `docs/specs/**` contains a "decision-required" marker, a placeholder (`TBD`/`TODO` without accompanying justification prose), or a per-spec decision-ID prefix other than the canonical `ADR-046-decision-register` numbering, run immediately before the documentation set's pre-panel snapshot |
| Destination | `packages/d2b-contract-tests/tests/policy_no_leaked_decision_prefix.rs`; invoked as a required precondition script `tests/tools/pre-panel-gate.sh` |
| Owner/dependencies | ADR046-streamline-001, ADR046-streamline-008 |
| Dependency/owner | ADR046-streamline-001, ADR046-streamline-008 |
| Current source | D014/docs/specs/README.md zero-open-decision requirement; no existing automated pre-panel gate |
| Reuse action | create |
| Implementation shape | Scans registry output plus raw Markdown for `decision-required`, `TBD`, `TODO`, and any decision-ID-shaped token (`[A-Z]+-\d+`) not matching the canonical `D\d+` register numbering |
| Detailed design | Scans registry output plus raw Markdown for `decision-required`, `TBD`, `TODO`, and any decision-ID-shaped token (`[A-Z]+-\d+`) not matching the canonical `D\d+` register numbering Primary reuse disposition: `create`. Preserved source-plan detail: net-new policy lint and precondition script. |
| Integration | Required precondition before requesting the ADR 0046 documentation set's panel round (D014) |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `policy_no_leaked_decision_prefix` |
| Adoption timing | Immediately; this is a precondition for the ADR 0046 documentation set's own remaining panel gate, not deferred tooling |
| Removal/supersession | None; graduates to permanent panel-gate precondition |
| Removal proof | None - net-new; no prior owner to remove; graduates to permanent panel-gate precondition |

### ADR046-streamline-017 - External evidence command planning

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-017` |
| Tier | A |
| Observed friction evidence | F11, F17: correction commits and full-tree sweeps happened without a documented, reusable command sequence a reviewer or future author could re-run to reproduce the same evidence; each investigation (as in this spec's own [Observed friction evidence](#observed-friction-evidence)) was ad hoc |
| Desired behavior | A committed, reusable list of exact `git`/`grep`/`xtask` commands that reproduce every metric and evidence citation in this spec (and any future evidence-grounded spec), so panel reviewers and future authors can independently reproduce the evidence without re-deriving the search strategy |
| Destination | `docs/specs/ADR-046-streamline-evidence-commands.md` (a follow-up artifact outside this task's file scope; tracked here as a required future addition, not authored by this spec) |
| Owner/dependencies | None - no prerequisite work item; owned by this spec |
| Dependency/owner | No prerequisite; `docs/specs` evidence-command documentation owner |
| Current source | This spec's Observed friction evidence command prose; no reusable evidence-command artifact exists yet |
| Reuse action | adapt |
| Implementation shape | A plain Markdown list of the exact commands used to produce each F1-F17 citation above (already reproduced verbatim in this spec's prose), organized by friction ID, so re-running them is copy-paste rather than re-derivation |
| Detailed design | A plain Markdown list of the exact commands used to produce each F1-F17 citation above (already reproduced verbatim in this spec's prose), organized by friction ID, so re-running them is copy-paste rather than re-derivation Primary reuse disposition: `adapt`. Preserved source-plan detail: extract/adapt the already-cited evidence commands into a net-new documentation artifact. |
| Integration | Referenced from PR bodies as external evidence per this repository's "PR bodies contain... check-status summaries only... may link to external evidence" convention |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Manual reviewer re-run of at least one command per friction ID during panel, confirmed to reproduce the cited count/SHA |
| Adoption timing | Immediately, as a follow-up documentation artifact; does not gate any other item in this spec |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-018 - Worktree/disk/target cleanup reporting

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-018` |
| Tier | A |
| Observed friction evidence | Generalizes this repository's existing disk-hygiene posture (already required for code waves) to the documentation-authoring worktrees this ADR's dossier-branch-per-scope workflow creates (F1 shows 30+ `adr0046-*` branches/worktrees existed concurrently) |
| Desired behavior | A report enumerating every `adr0046-*` worktree, its per-worktree `packages/target/` size, and its branch's merge/staleness state, with **no automatic destructive deletion**; cleanup remains an explicit human/integrator-approved action |
| Destination | `tests/tools/worktree-disk-report.sh` |
| Owner/dependencies | ADR046-streamline-010 (shares branch-staleness detection) |
| Dependency/owner | ADR046-streamline-010 (shares branch-staleness detection) |
| Current source | Existing disk-hygiene operator guidance; no ADR 0046 worktree disk report exists yet |
| Reuse action | adapt |
| Implementation shape | `git worktree list --porcelain` plus `du -sh` per worktree's `packages/target/`; output is a report only, never a `git worktree remove`/`rm -rf` invocation |
| Detailed design | `git worktree list --porcelain` plus `du -sh` per worktree's `packages/target/`; output is a report only, never a `git worktree remove`/`rm -rf` invocation Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt existing disk-hygiene reporting concepts into a net-new non-mutating script. |
| Integration | Run manually before requesting garbage collection or worktree removal, consistent with this repository's existing "Disk hygiene contract"-equivalent operator guidance for code waves |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New test confirming the reporting script's exit code and output never include a mutating command string, and confirming it correctly flags a fixture worktree with a real (non-symlink) `packages/target/` |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-019 - Docs source-evidence old→new terminology helper

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-019` |
| Tier | A |
| Observed friction evidence | F15: `Realm`/`Workload` remains the live vocabulary in this repository's current-source `AGENTS.md`/`d2b-realm-*` crates while target ADR 0046 specs use `Zone`/`Guest`; D050's rename shows at least one dossier round used the old target name before the rename propagated |
| Desired behavior | A helper cross-referencing every current-source symbol/crate/option cited by a spec's "Current-code fit" table against `ADR-046-terminology-and-identities`/`ADR-046-current-code-migration-map`'s old→new mapping, flagging any spec that uses a target-side name not yet present in either mapping document |
| Destination | `packages/xtask/src/bin/terminology_check.rs` (`cargo run -p xtask -- terminology-check`) |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | ADR-046 terminology and current-code migration-map specs; no automated old-to-new terminology helper exists yet |
| Reuse action | create |
| Implementation shape | Parses every "Current-code fit" table's "Current anchor" cell for symbol/crate names, cross-references against the migration map's disposition rows, and flags any current-source citation absent from the map |
| Detailed design | Parses every "Current-code fit" table's "Current anchor" cell for symbol/crate names, cross-references against the migration map's disposition rows, and flags any current-source citation absent from the map Primary reuse disposition: `create`. Preserved source-plan detail: net-new terminology checker over existing mapping specs. |
| Integration | Standing Layer-1 policy lint once wired; also usable ad hoc when authoring a new dossier's evidence section |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | New test seeding a fixture spec citing a current-source symbol absent from the migration map and confirming it is flagged |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-020 - Hermetic test placement lint

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-020` |
| Tier | A |
| Observed friction evidence | D094: without a lint, a slow scenario needing a real process/container/network silently lands in a `src/`/`tests/` hermetic tier and inflates the inner loop |
| Desired behavior | A policy lint asserting every `src/` `#[cfg(test)]` module and crate `tests/*.rs` file is hermetic - no process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build, KVM, USB/GPU/TPM hardware, live cloud, or filesystem tree beyond tiny temp fixtures; any such need must move to `integration/`, never gain a sleep, larger timeout, or `#[ignore]` |
| Destination | `packages/d2b-contract-tests/tests/policy_test_placement.rs` |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | D094 test-placement requirement; no existing hermetic test-placement policy lint |
| Reuse action | create |
| Implementation shape | Scans hermetic-tier Rust sources for banned API surfaces (`std::process::Command`, socket/container/DBus/systemd helpers, `#[ignore]`) and for `integration/`-only markers appearing outside `integration/` |
| Detailed design | Scans hermetic-tier Rust sources for banned API surfaces (`std::process::Command`, socket/container/DBus/systemd helpers, `#[ignore]`) and for `integration/`-only markers appearing outside `integration/` Primary reuse disposition: `create`. Preserved source-plan detail: net-new policy lint. |
| Integration | `make test-policy` row; no new top-level `tests/*.sh` gate |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Fixture crate with an intentional process-spawning hermetic test is rejected naming the file/line; a correct crate passes |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-021 - Deterministic-clock/sleep lint

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-021` |
| Tier | A |
| Observed friction evidence | D094: wall-clock `sleep`/retry in a hermetic test both breaks the p95 ≤50 ms budget and makes the test flaky/non-parallel-safe |
| Desired behavior | A lint rejecting wall-clock sleep/retry (`std::thread::sleep`, `tokio::time::sleep` without a paused test clock, real `Instant::now` polling loops) in `src/`/`tests/` hermetic tiers, requiring a deterministic fake clock/RNG instead |
| Destination | `packages/d2b-contract-tests/tests/policy_test_determinism.rs` |
| Owner/dependencies | ADR046-streamline-020 |
| Dependency/owner | ADR046-streamline-020 |
| Current source | D094 deterministic-clock requirement; no existing wall-clock sleep/retry policy lint |
| Reuse action | create |
| Implementation shape | Scans hermetic sources for banned time/sleep APIs and asserts the deterministic fake-clock/RNG fixtures from the toolkit are used; classified crypto/property exceptions are allow-listed by explicit name with a declared per-test budget |
| Detailed design | Scans hermetic sources for banned time/sleep APIs and asserts the deterministic fake-clock/RNG fixtures from the toolkit are used; classified crypto/property exceptions are allow-listed by explicit name with a declared per-test budget Primary reuse disposition: `create`. Preserved source-plan detail: net-new policy lint layered on ADR046-streamline-020. |
| Integration | `make test-policy` row |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Fixture hermetic test using `thread::sleep` is rejected; a classified crypto test on the allow-list passes |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-022 - Test-runtime ledger and timing gate

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-022` |
| Tier | A |
| Observed friction evidence | D094: no machine-readable record of execution-only test time exists, so budget regressions are invisible until the inner loop is already slow |
| Desired behavior | A test-runtime ledger + timing gate (reusing existing `xtask`/`libtest --format=json` output, no new framework) measuring execution-only time after build against the §10.16 budgets, recording reference runner/repetitions/p95, reporting top slow tests, applying a historical regression threshold, and emitting a CI artifact |
| Destination | `packages/xtask/src/test_runtime_ledger.rs` (shared with `ADR046-delivery-007`) |
| Owner/dependencies | ADR046-delivery-007 |
| Dependency/owner | ADR046-delivery-007 |
| Current source | The test-runtime ledger has since landed as `packages/xtask/src/test_runtime_ledger.rs` (shared with `ADR046-delivery-007`), invoked by `make test-runtime-ledger` against the pinned `tests/runtime-ledger-baseline.json`/`tests/runtime-ledger-census.json`, and run as the `test-runtime-ledger` Layer-1 job; remaining effort is hardening the census/regression enforcement, not creation |
| Reuse action | adapt |
| Implementation shape | Parses per-test JSON timings, aggregates per test/crate/shard, compares against pinned budgets and the previous ledger, and fails on regression beyond the threshold |
| Detailed design | Parses per-test JSON timings, aggregates per test/crate/shard, compares against pinned budgets and the previous ledger, and fails on regression beyond the threshold Primary reuse disposition: `adapt`. Preserved source-plan detail: share/adapt ADR046-delivery-007 timing-ledger implementation for this gate. |
| Integration | Consumed by wave entry/exit (`ADR-046-validation-and-delivery` §4/§10.16); `make test-rust` and Layer-1 shards run concurrently |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Synthetic timing regression fails the gate; ledger output is deterministic and machine-readable |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-023 - Legacy-test retirement generator

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-023` |
| Tier | A |
| Observed friction evidence | D094: replaced behavior otherwise leaves old duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest entries running alongside their successors indefinitely |
| Desired behavior | A generator that, from each current-code migration work item's old-selector→disposition table, produces the retirement checklist and removal gate: which old test selectors/files/`tests/layer1-jobs.json` rows/closed gate manifests/flake-matrix-Nix-unit pins/generated ledgers/CI shards are deleted once successor coverage and removal proof pass, and asserts a retired selector is absent afterward |
| Destination | `packages/xtask/src/bin/legacy_test_retirement.rs` (`cargo run -p xtask -- legacy-test-retirement`) |
| Owner/dependencies | ADR046-streamline-008, ADR046-streamline-022 |
| Dependency/owner | ADR046-streamline-008, ADR046-streamline-022 |
| Current source | Current-code migration-map disposition rows and existing gate manifests; no legacy-test retirement generator exists yet |
| Reuse action | adapt |
| Implementation shape | Reads the migration map's disposition rows, cross-references the live `tests/layer1-jobs.json`/gate manifests, and emits the delete set plus an absence assertion; never deletes automatically - it produces the gated checklist and the failing test that proves incomplete retirement |
| Detailed design | Reads the migration map's disposition rows, cross-references the live `tests/layer1-jobs.json`/gate manifests, and emits the delete set plus an absence assertion; never deletes automatically - it produces the gated checklist and the failing test that proves incomplete retirement Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new generator reading existing migration rows and gate manifests without mutating them. |
| Integration | `make test-policy`/`make test-drift` row; wired to every current-code migration work item's removal proof |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | Fixture with a replaced behavior whose old selector still appears in `tests/layer1-jobs.json` fails; once removed, the absence assertion passes |
| Adoption timing | Immediately |
| Removal/supersession | None - net-new; no prior owner to remove |
| Removal proof | None - net-new; no prior owner to remove |

### ADR046-streamline-024 - Implementation-graph generator and duplicate-generator reconciliation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-streamline-024` |
| Tier | A (shared prep) |
| Observed friction evidence | The implementation graph (D095) is hand-regenerated per wave; separate ad hoc generators exist for the spec-set manifest and the test-runtime ledger, so their output can drift from a single canonical path |
| Desired behavior | The `xtask implementation-graph` subcommand (landed) that deterministically emits `docs/specs/ADR-046-implementation-graph.json` and its `.md` view from `ADR-046-spec-set.json` + `ADR-046-work-items.json` + the `ADR-046-validation-and-delivery` §3 wave topology (D095/D096/D097), preserving the current node/work-item counts and never overwriting with stale counts; and the reconciliation of the duplicate generators for the spec-set manifest and the test-runtime ledger to a single canonical `xtask` path |
| Destination | `packages/xtask/src/bin/implementation_graph.rs` (`cargo run -p xtask -- implementation-graph`); folds the spec-set and test-runtime emitters into the one `xtask` dispatch |
| Owner/dependencies | ADR046-streamline-001 |
| Dependency/owner | ADR046-streamline-001 |
| Current source | `xtask implementation-graph` now generates the graph deterministically and `xtask spec-registry` emits `ADR-046-spec-set.json`, so the graph is no longer regenerated by hand; the remaining reconciliation is folding the separate test-runtime ledger (ADR046-delivery-007) emitter into the single canonical `xtask` path - duplicate-generator finding, retained here for that reconciliation |
| Reuse action | create |
| Implementation shape | Reads the two manifests plus §3 wave topology, maps every member spec and every work item exactly once to a wave/parallel-group, emits typed edges and topological rank, renders the Mermaid/table `.md`, and is wired into the existing `gen-*` drift gate; the spec-set and test-runtime emitters become subcommands of the same `xtask` binary so there is one canonical generator, not three |
| Detailed design | Deterministic (sorted keys, no timestamps/host paths); regenerated after any spec or work-item change and after the two manifests; a drift gate runs it and `git diff --exit-code`. Retains the duplicate-generator findings for the spec-set manifest and the test-runtime ledger and reconciles them to the single `xtask` path rather than leaving three independent emitters Primary reuse disposition: `create`. Preserved source-plan detail: net-new generator that consumes the two manifests and the wave topology; folds the duplicate spec-set/test-runtime emitters into one canonical path. |
| Integration | `tests/unit/gates/` drift gate alongside `xtask spec-registry`; consumed by the ready-wave query in `ADR-046-validation-and-delivery` §3.5.1 |
| Data migration | None - docs/tooling only; no runtime state |
| Validation | `implementation_graph::every_spec_and_work_item_mapped_once`; `implementation_graph::acyclic_and_wave_monotonic`; `implementation_graph::parallel_groups_are_file_disjoint`; `implementation_graph_regenerates_clean`; duplicate-generator reconciliation asserts one canonical emitter path for spec-set and test-runtime |
| Adoption timing | Immediately after ADR046-streamline-001 |
| Removal/supersession | Supersedes the hand-regeneration and the separate spec-set/test-runtime emitters once the single `xtask` path is green |
| Removal proof | The ad hoc separate emitters for spec-set and test-runtime are removed after the single `xtask implementation-graph`/`spec-registry` dispatch reaches parity |
