# ADR 0035: Efficiency and simplification roadmap

- Status: Accepted
- Date: 2026-06-19
- Related: ADR 0006 (manifest bundle versioning), ADR 0009 (Rust
  toolchain, MSRV, and supply-chain policy), ADR 0015 (daemon-only clean
  break), ADR 0022 (stabilization-mode releases), ADR 0032 (nixling v2
  constellation control plane), ADR 0034 (storage lifecycle, restart
  adoption, and synchronization)

## Context

Nixling has intentionally paid complexity to remove legacy surfaces and
to make host mutation explicit: daemon-only lifecycle, typed broker ops,
generated bundle artifacts, pidfd handoff, minijail profiles, guest-control
RPCs, and v2 constellation/provider seams. Those decisions are still correct.

The current codebase now has a different risk: every security hardening,
feature wave, generated contract, and test migration has left scaffolding
behind. The result is high cognitive load and slower iteration:

- large Rust hub files concentrate unrelated concerns (`packages/nixlingd`,
  `packages/nixling`, and `packages/nixling-priv-broker`);
- some CLI/daemon JSON view models still live beside presentation or
  dispatch logic instead of a shared contract boundary;
- Nix modules repeatedly construct the same bundle-artifact shapes and
  recompute similar VM/env indexes;
- activation, tmpfiles, broker repair, and generated storage contracts
  overlap in ways that are hard to reason about;
- test drivers preserve fail-closed coverage but still carry transitional
  shell wrappers, repeated linting, repeated generated-artifact checks, and
  multiple entrypoint vocabularies;
- v2 constellation work is already adding provider crates and transport
  abstractions, so any cleanup that ignores ADR 0032 would become a forked
  architecture instead of simplification.

Efficiency work must therefore be architectural, not cosmetic. The goal is
to make the fastest path the normal path: fewer code paths, fewer generated
artifact patterns, fewer subprocess gates, fewer roots of authority, and
fewer places where an operator or contributor must remember historical
context.

## Decision

Nixling will run an efficiency and simplification program as a set of
reviewed waves. Each wave removes one class of duplication or transitional
surface while preserving the load-bearing contracts from earlier ADRs:

- `nixlingd` remains the sole lifecycle supervisor.
- `nixling-priv-broker` remains the sole privileged host-mutation authority.
- The Rust CLI remains the only operator CLI surface.
- Generated bundle artifacts remain versioned contracts, not ad hoc JSON.
- Storage, lock, ACL, cleanup, and restart behavior follow ADR 0034.
- v2 provider/transport work follows ADR 0032; this ADR narrows and cleans
  that path rather than introducing another abstraction hierarchy.
- Tests remain fail-closed and follow the Layer-1-first test model; this ADR
  does not authorize new ad hoc shell gates.

The waves below are ordered to reduce future work first. Wave 0 creates
measurement and deletion rules; Waves 1-4 remove duplicated infrastructure;
Waves 5-8 reshape Rust/Nix boundaries; Waves 9-11 tighten runtime and
operator-facing efficiency; Wave 12 is the recurring ratchet that keeps the
codebase from growing back.

## Efficiency principles

### One contract, many consumers

If the CLI, daemon, broker, docs, and tests all need the same shape, that
shape belongs in a contract crate or generated artifact. Presentation code
may adapt it, but it must not redefine a parallel schema.

### One normalized model per evaluation

Nix module evaluation should normalize `cfg.vms`, `cfg.envs`, bundle
artifacts, runner roles, and provider capability records once, then pass
indexes to consumers. Repeated `filterAttrs` / `mapAttrsToList` scans are
acceptable only in leaf modules whose input is already narrowed.

### One side-effect owner

Every mutable host surface has exactly one owner. NixOS tmpfiles creates
static base roots, activation performs migrations and static repairs, the
broker mutates privileged runtime state, and `nixlingd` owns daemon ledgers.
No wave may reintroduce broad recursive `chmod`, `chown`, `setfacl`, or
raw-path repair logic to make a test pass.

### Delete compatibility scaffolding on schedule

A placeholder, no-op option, legacy comment, bootstrap feature, or
transition wrapper must have an owner and removal wave. If a compatibility
surface cannot name a current consumer, it is treated as deletion work, not
as harmless documentation.

### Prefer typed builders over string assembly

Repeated argv, readiness, process, path, lock, and audit shapes should move
toward typed builders that validate once and render once. Builders must
encode security invariants; they are not string-concatenation helpers.

### Make generated output families explicit

Generated schemas, docs, completions, proto bindings, and fixtures are
separate artifact families. Gates should fail closed per family so a stale
completion does not require rerunning protobuf generation, and a protobuf
change does not obscure CLI docs drift.

## Multi-wave plan

### Wave 0 — Baseline, budgets, and deletion ledger

Goal: make efficiency measurable and make deletion safe.

Tasks:

1. Establish repository budgets for the surfaces that predict iteration
   cost:
   - Rust hub size by file and module;
   - public DTO count by crate;
   - crate dependency fan-in/fan-out;
   - Nix module count and repeated full-`cfg.vms` / full-`cfg.envs` scans;
   - generated-artifact family runtime;
   - Layer-1 local wall time and peak Nix store / Cargo target growth.
2. Add a deletion ledger for transitional surfaces. Each row records the
   surface, owner, current consumer, target removal wave, and replacement.
   The ledger starts with bootstrap broker feature paths, no-op systemd
   scaffolding, retired option placeholders, old comments that describe
   deleted CLI/systemd modes, and shell wrappers superseded by Make targets.
3. Categorize all code as one of:
   - contract;
   - pure model/policy;
   - adapter;
   - side-effect execution;
   - presentation;
   - test-only support.
4. Define an "efficiency proof" requirement for later waves: each wave must
   delete or consolidate a named surface, not merely move code.

Validation:

- policy lint or repository inventory proves every deletion-ledger row has
  a valid status;
- baseline report is generated from tracked files only and does not become
  release documentation.

Exit criteria:

- every later wave has an owned list of surfaces to remove or consolidate;
- no implementation wave can add a new compatibility stub without a ledger
  row and removal criterion.

### Wave 1 — Generated artifact family consolidation

Goal: reduce Nix and Rust drift by making bundle-artifact emission a single
pattern.

Tasks:

1. Add a Nix helper for generated bundle artifacts that owns:
   - `schemaVersion`;
   - `data`;
   - compact `jsonText`;
   - `pkgs.writeText` / `pkgs.runCommand` output;
   - `options.nixling._bundle.<artifact>`;
   - private `/etc/nixling/<artifact>.json` installation.
2. Apply it to the repeated host/process/privilege/closure/storage/sync
   emitters while preserving each artifact's schema, path, owner, and mode.
3. Keep `bundle.json` hashing special, but factor the common install and
   `_bundle` option wiring.
4. Create named generated-artifact families:
   - bundle schemas and DTO schemas;
   - CLI shell artifacts and CLI JSON schemas;
   - daemon API and error-code docs;
   - guest-control protobuf / ttRPC bindings;
   - rendered fixture artifacts.
5. Split drift reporting by family while keeping one `make test-drift`
   entrypoint.

Primary targets:

- `nixos-modules/bundle.nix`;
- `nixos-modules/host-json.nix`;
- `nixos-modules/processes-json.nix`;
- `nixos-modules/privileges-json.nix`;
- `nixos-modules/closures-json.nix`;
- `nixos-modules/storage-json.nix`;
- `nixos-modules/sync-json.nix`;
- `packages/xtask/src/main.rs`;
- `tests/unit/gates/drift-check.sh`.

Validation:

- existing bundle/manifest/schema drift gates remain green;
- generated file bytes are unchanged unless a schema bump is explicitly
  part of the wave;
- no new shell gate is introduced.

Exit criteria:

- adding a new bundle artifact requires one helper invocation plus Rust DTO
  and schema registration, not a copy of 40-80 lines of emitter boilerplate.

### Wave 2 — Normalized Nix VM/env indexes

Goal: evaluate host, network, observability, USBIP, and process modules from
one normalized model instead of repeated full-tree scans.

Tasks:

1. Introduce an internal normalized index under the Nix module layer, with
   at least:
   - enabled VMs;
   - enabled normal workload VMs;
   - framework-declared system VMs;
   - enabled envs;
   - workloads by env;
   - graphics/audio/video/TPM/USBIP/observability subsets;
   - declared net VM per env;
   - stable per-env port/name/IP metadata;
   - provider/runtime capability summary.
2. Replace local recomputation in host and network modules with reads from
   the index.
3. Move legacy/fallback interface-name derivation into a single helper with
   typed inputs.
4. Keep index generation pure and read-only; it may not perform activation,
   tmpfiles, broker, or host mutation work.

Primary targets:

- `nixos-modules/lib.nix`;
- `nixos-modules/host.nix`;
- `nixos-modules/network.nix`;
- `nixos-modules/net.nix`;
- `nixos-modules/options-envs.nix`;
- `nixos-modules/options-vms.nix`;
- `nixos-modules/processes-json.nix`.

Validation:

- representative examples render byte-identical manifest/process/network
  artifacts unless intentional changes are documented;
- eval cases cover multi-env, net VM, USBIP, observability, and graphics
  selection from the index;
- `nix flake check --no-build` does not regress in eval time.

Exit criteria:

- modules no longer open-code "enabled VMs in env X" or "USBIP envs" scans;
- adding a per-VM component adds one index classification, not repeated
  scans across modules.

### Wave 3 — Contract crate and DTO boundary cleanup

Goal: move shared JSON/wire/output models out of CLI and daemon hub files.

Tasks:

1. Classify all serde/json schema structs in `nixling`, `nixlingd`,
   `nixling-ipc`, and `nixling-core` as one of:
   - wire contract;
   - bundle contract;
   - CLI output contract;
   - daemon internal state;
   - presentation-only view.
2. Move stable CLI output DTOs and daemon API DTOs to a contract crate
   boundary. The exact home may be an expanded `nixling-ipc` or a narrower
   `nixling-contracts` crate, but dependency direction must stay acyclic:
   presentation crates depend on contracts; contracts do not depend on CLI
   or daemon execution crates.
3. Leave formatting, terminal behavior, and command dispatch in `nixling`.
4. Leave listener loops, process supervision, and mutation orchestration in
   `nixlingd`.
5. Regenerate CLI schemas and daemon API docs from the new DTO homes.

Primary targets:

- `packages/nixling/src/lib.rs`;
- `packages/nixlingd/src/lib.rs`;
- `packages/nixling-ipc/src/public_wire.rs`;
- `packages/nixling-core/src/*`;
- `packages/xtask/src/main.rs`;
- `docs/reference/cli-output/`;
- `docs/reference/daemon-api.md`.

Validation:

- generated docs/schemas are byte-for-byte equivalent except source-location
  churn and intentional module-path changes;
- CLI JSON contract tests still deserialize with `deny_unknown_fields`;
- dependency-direction policy continues to forbid CLI/daemon backedges.

Exit criteria:

- `nixling/src/lib.rs` is presentation and dispatch, not a schema warehouse;
- `nixlingd/src/lib.rs` is orchestration and module wiring, not the canonical
  source for public output schemas.

### Wave 4 — Rust hub-file decomposition

Goal: make the largest Rust crates understandable without changing behavior.

Tasks:

1. Split crate-root hub files by architectural role:
   - accept loop and auth admission;
   - lifecycle command dispatch;
   - VM status/read models;
   - guest-control client/server bridge;
   - gateway-mode routing;
   - QEMU media lifecycle;
   - host doctor/read-only checks;
   - CLI command groups.
2. Replace broad imports in hub files with narrow module APIs.
3. Move test-only helpers behind `cfg(test)` modules or dedicated
   test-support features/crates.
4. Keep public crate APIs stable unless an earlier contract wave explicitly
   moved the public type.
5. Remove comments that only narrate deleted migration phases; keep comments
   that explain current invariants.

Primary targets:

- `packages/nixlingd/src/lib.rs`;
- `packages/nixling/src/lib.rs`;
- `packages/nixling-priv-broker/src/runtime.rs`;
- `packages/nixling-priv-broker/src/live_handlers.rs`;
- `packages/nixling-core/src/bundle_resolver.rs`;
- `packages/nixling-guestd/src/service.rs`.

Validation:

- no public JSON/wire behavior changes;
- `cargo test` and contract tests pass with warnings denied;
- generated daemon API docs remain accurate after source-line churn.

Exit criteria:

- no crate root remains the place where unrelated subsystems accumulate by
  default;
- new command or daemon feature work has an obvious module home.

### Wave 5 — Runner/process builder DSL

Goal: encode process launch invariants once and remove repeated argv,
profile, readiness, and audit assembly.

Tasks:

1. Define a typed runner/process builder that composes:
   - role id and principal;
   - minijail profile id;
   - argv renderer;
   - environment;
   - fd requirements;
   - cgroup leaf;
   - readiness predicate;
   - writable path references;
   - restart/adoption class;
   - audit operation id.
2. Convert existing pure argv generators to builder-backed renderers without
   weakening their current tests.
3. Make `processes.json`, minijail profiles, storage references, and broker
   `SpawnRunner` requests consume the same builder model.
4. Keep provider-specific launch details behind provider adapters, not in
   the generic builder.

Primary targets:

- `packages/nixling-host/src/*_argv.rs`;
- `packages/nixling-host/src/runner_argv_regenerator.rs`;
- `packages/nixling-core/src/processes.rs`;
- `nixos-modules/processes-json.nix`;
- `nixos-modules/minijail-profiles.nix`;
- `packages/nixling-priv-broker/src/ops/*`;
- `packages/nixlingd/src/supervisor/*`.

Validation:

- existing argv-shape and minijail-validator tests remain green;
- rendered `processes.json` and minijail profile fixtures are unchanged
  unless an intentional schema bump is made;
- adding a new runner role requires a lifecycle matrix plus builder
  implementation, not scattered JSON/string logic.

Exit criteria:

- the runner role lifecycle matrix becomes executable structure rather than
  prose that must be manually mirrored in Nix and Rust.

### Wave 6 — Side-effect ownership cleanup

Goal: make host filesystem, ACL, lock, cleanup, and migration work follow a
single ADR 0034 contract.

Tasks:

1. Audit every tmpfiles, activation, broker storage op, daemon ledger write,
   and runner-created path against `storage.json` / `sync.json`.
2. Move broad static path creation to tmpfiles only when the path is a base
   root and has no dynamic ACL/ownership state.
3. Move privileged dynamic repair to broker ops addressed by opaque storage
   ids.
4. Limit activation to migrations and static repairs that cannot be moved to
   tmpfiles or broker execution.
5. Delete no-op systemd and activation scaffolding once no consumers remain.
6. Replace inherited or incidental file locks with sync-contract-owned OFD
   locks.

Primary targets:

- `nixos-modules/host-activation.nix`;
- `nixos-modules/host-daemon.nix`;
- `nixos-modules/bundle.nix`;
- `nixos-modules/storage-json.nix`;
- `nixos-modules/sync-json.nix`;
- `packages/nixling-priv-broker/src/ops/*`;
- `packages/nixlingd/src/storage_lifecycle.rs`.

Validation:

- storage lifecycle contract tests cover every mutable host path touched by
  activation, broker, or daemon code;
- no new raw-path privileged mutation enters daemon code;
- restart/adoption behavior remains continuation-safe.

Exit criteria:

- when a path is wrong, there is one generated storage row and one repair
  owner; contributors do not patch around failures with one-off ACL code.

### Wave 7 — Test driver thinning and native policy migration

Goal: keep coverage fail-closed while reducing shell orchestration and
duplicate work.

Tasks:

1. Make `make test-*` the single stable test vocabulary; keep legacy aliases
   only when they are documented compatibility shims with removal criteria.
2. Collapse duplicate shell linting: `test-lint` owns syntax, shellcheck,
   and Nix parse; `static-fast-tier0.sh` becomes either a pure alias or is
   retired through the migration ledger.
3. Move source-tree policy checks that need parsing or cross-file reasoning
   into Rust policy tests under `packages/nixling-contract-tests/tests`.
4. Keep shell only for orchestration that genuinely needs ecosystem tools,
   Nix evaluation, or platform setup.
5. Split generated-artifact drift failures by artifact family as defined in
   Wave 1.
6. Avoid running the same fixture build from both `test-rust` and
   `test-flake` unless the second run proves a different boundary.

Primary targets:

- `Makefile`;
- `tests/test-lint.sh`;
- `tests/static-fast-tier0.sh`;
- `tests/test-rust.sh`;
- `tests/test-drift.sh`;
- `tests/unit/gates/drift-check.sh`;
- `tests/unit/meta/*`;
- `packages/nixling-contract-tests/tests/*`;
- `.github/workflows/*`.

Validation:

- migration ledger records any retired shell gate and successor test;
- pinned test inventory confirms no coverage silently disappears;
- CI still runs every Layer-1 family, but with fewer repeated setup paths.

Exit criteria:

- contributors can answer "what should I run?" from the Makefile alone;
- shell scripts orchestrate tools, while policy and contract logic lives in
  typed tests.

### Wave 8 — Workspace and dependency graph simplification

Goal: reduce compile cost and architectural ambiguity in the Rust workspace.

Tasks:

1. Audit crates with one source file or no independent release boundary:
   keep them only if they enforce dependency direction, feature isolation,
   static-link boundaries, or provider plug-in boundaries.
2. Decide whether `nixling-priv-broker` remains a separate workspace. If it
   must remain separate for security or build reasons, document the reason
   and remove duplicated dependency/lint declarations through shared
   metadata where possible.
3. Move provider traits and capability descriptors to the narrowest crates
   that ADR 0032 needs; avoid generic provider crates that only forward
   types.
4. Gate heavyweight provider dependencies behind feature flags or separate
   binaries so the local-only path does not compile cloud providers.
5. Ensure test-support features do not enter production builds.

Primary targets:

- `packages/Cargo.toml`;
- `packages/nixling-priv-broker/Cargo.toml`;
- `packages/nixling-constellation-*`;
- `packages/nixling-provider-*`;
- `packages/nixling-gateway*`;
- `packages/nixling-host-providers`;
- `flake.nix` Rust package source construction.

Validation:

- dependency-direction policy remains green;
- local-only CLI/daemon build does not pull provider-only dependencies unless
  explicitly enabled;
- supply-chain gates still cover every lockfile that can ship.

Exit criteria:

- each crate has a sentence-long reason to exist;
- compile graphs match operator paths: local-only users do not pay for
  provider experiments.

### Wave 9 — v2 provider integration simplification

Goal: make ADR 0032 extensibility reduce code, not multiply adapters.

Tasks:

1. Fold Wave 0 provider abstraction work from ADR 0032 into the efficiency
   taxonomy from this ADR: provider code is an adapter, not a new place to
   define core lifecycle semantics.
2. Define a single provider capability descriptor that covers local
   hypervisors, host substrates, display transports, cloud sandboxes, and
   remote full hosts.
3. Keep local fast path as one provider instance with no relay/gateway
   overhead when no remote realm is configured.
4. Make unsupported operations fail through typed capability denial, not
   by probing for optional code paths.
5. Require each provider to declare which generic builders/contracts it
   consumes: runner/process, storage/sync, display, transport, audit, and
   observability.

Primary targets:

- `packages/nixling-constellation-core`;
- `packages/nixling-constellation-provider`;
- `packages/nixling-constellation-router`;
- `packages/nixling-provider-aca`;
- `packages/nixling-provider-relay`;
- `packages/nixling-gateway-runtime`;
- `nixos-modules/gateway-vm.nix`;
- ADR 0032 implementation wave notes.

Validation:

- local single-host lifecycle path is unchanged and does not instantiate
  gateway/relay/provider credentials;
- provider-managed sandbox paths use provider exec/log/display subsets and
  do not pretend to be local guestd VMs;
- capability denial is covered by unit/contract tests.

Exit criteria:

- adding a provider is mostly adapter code plus capability records, not a new
  copy of lifecycle, display, transport, audit, and storage logic.

### Wave 10 — Runtime hot-path efficiency

Goal: improve runtime behavior where simplicity and performance align.

Tasks:

1. Keep daemon state in immutable snapshots with narrow mutation points so
   list/status/doctor reads do not lock large global structures.
2. Replace repeated JSON parse/read cycles for static bundle artifacts with
   an explicitly versioned in-memory cache invalidated by bundle hash.
3. Batch broker requests where the ordering is contractually one operation,
   such as host prepare phases, storage repair sets, or network reconcile,
   while keeping per-op audit records.
4. Use pidfd and cgroup discovery helpers consistently so restart/adoption
   code does not duplicate kernel traversal.
5. Keep expensive observability scraping off command-response paths; surface
   cached health with timestamps and explicit freshness.

Primary targets:

- `packages/nixlingd/src/concurrency.rs`;
- `packages/nixlingd/src/supervisor/*`;
- `packages/nixlingd/src/storage_lifecycle.rs`;
- `packages/nixlingd/src/ch_stats.rs`;
- `packages/nixlingd/src/metrics.rs`;
- `packages/nixling-priv-broker/src/runtime.rs`;
- `packages/nixling-core/src/bundle_resolver.rs`.

Validation:

- no stale-bundle dispatch after host switch;
- broker audit remains per typed operation;
- list/status/doctor output includes freshness where cached health is used.

Exit criteria:

- runtime speedups come from fewer repeated reads/parses/locks, not from
  skipping validation or swallowing errors.

### Wave 11 — Example, template, and documentation diet

Goal: make shipped docs/examples teach the current model without preserving
historical scaffolding.

Tasks:

1. Remove comments in examples/templates that explain retired modes instead
   of current usage.
2. Keep one minimal example, one multi-env example, one graphics/workstation
   example, one observability example, and one identity-composition example
   only when each teaches a distinct public surface.
3. Move deep architecture history out of how-to docs and into ADRs.
4. Ensure README and reference docs describe the same CLI, daemon, and bundle
   surfaces as the code.
5. Keep process markers out of shipped consumer docs and released changelog
   sections.

Primary targets:

- `README.md`;
- `docs/how-to/*`;
- `docs/reference/*`;
- `examples/*`;
- `templates/default/*`;
- `CHANGELOG.md`.

Validation:

- docs links and ADR index checks pass;
- examples still evaluate;
- no user-facing docs describe retired bash/systemd surfaces as live.

Exit criteria:

- new users see the current architecture first; historical context remains
  available in ADRs but does not dominate day-to-day docs.

### Wave 12 — Recurring efficiency ratchet

Goal: prevent re-growth.

Tasks:

1. Add review checklist items for:
   - new crate justification;
   - new shell gate justification;
   - new generated artifact family registration;
   - new compatibility surface removal criterion;
   - new full-tree Nix scan justification;
   - new public DTO location.
2. Add lightweight budgets to policy tests where they are stable enough to
   avoid churn. Budgets should warn or fail only on trends that predict real
   maintenance cost, not on arbitrary line counts.
3. Require every future ADR with implementation waves to include a
   simplification/deletion row.
4. Periodically retire stale ADR process markers from unreleased changelog
   prose before release.

Validation:

- policy tests enforce the checklist only where mechanically reliable;
- panel review treats budget increases as design questions, not automatic
  blockers.

Exit criteria:

- every growth wave also pays down or deletes something;
- efficiency remains part of architecture review, not a one-time cleanup.

## Highest-leverage deletion and consolidation targets

These targets are good first entries for the Wave 0 deletion ledger:

| Target | Desired outcome |
| --- | --- |
| Duplicate JSON emitter boilerplate | One helper-backed artifact emitter pattern. |
| Repeated full `cfg.vms` / `cfg.envs` scans | One normalized Nix index consumed by host/network/process modules. |
| CLI/daemon public DTOs in hub files | Shared contract boundary plus presentation adapters. |
| `nixlingd`, `nixling`, and broker hub files | Focused modules with narrow APIs. |
| One-off argv/readiness/audit assembly | Typed runner/process builder shared by Nix and Rust contracts. |
| Activation/tmpfiles/broker ownership overlap | ADR 0034 storage/sync ownership rows with one repair owner. |
| Duplicate shell lint entrypoints | One lint owner, compatibility aliases only with removal rows. |
| Monolithic drift gate reporting | Named generated-artifact families. |
| Shell policy checks that parse source | Rust contract/policy tests. |
| Provider dependencies on local-only path | Feature-gated/provider-isolated compile graph. |
| Example/template historical comments | Current-model examples with history moved to ADRs. |

## Anti-goals

- Do not weaken security boundaries to reduce code. The broker/daemon split,
  typed ops, pidfd handoff, minijail profiles, and no-bash invariant are
  not optimization targets.
- Do not collapse generated contracts into untyped runtime maps.
- Do not replace many small explicit broker ops with one unstructured
  "run shell" or "apply path" escape hatch.
- Do not introduce a second v2 architecture beside ADR 0032.
- Do not add new shell gates for efficiency measurement; use existing
  Layer-1 test types or repository-local generated reports.
- Do not use line count alone as a success metric. Deleting useful
  invariants is failure even when the diff is smaller.
- Do not optimize CI by skipping families of validation. Efficiency comes
  from better factoring and narrower invalidation, not fail-open coverage.

## Consequences

Positive:

- New contributors can find the owner of a behavior by type: contract,
  model, adapter, side-effect executor, or presentation.
- The v2 provider architecture gets simpler because providers plug into
  existing lifecycle/storage/display/transport contracts instead of
  inventing parallel paths.
- Local-only users pay less compile and runtime cost for cloud/provider
  experiments.
- Generated-artifact failures become narrower and cheaper to fix.
- The daemon and CLI crates become easier to review because public DTOs and
  presentation/dispatch code no longer live in the same hub files.
- Deletion becomes a normal implementation task with explicit owners.

Negative:

- Several waves initially move code without changing behavior, which can
  create source-line churn in generated docs.
- Contract extraction may require temporary compatibility re-exports while
  downstream code moves.
- A normalized Nix index is itself a new abstraction; if it becomes a dumping
  ground, it will centralize complexity instead of reducing it.
- Splitting drift families can make the gate graph more explicit but also
  requires careful CI wiring to avoid accidental coverage gaps.
- Compile-graph feature gating can make local development more sensitive to
  missing feature flags if not documented well.

## Review and validation policy

Each implementation wave must include:

1. a deletion/consolidation list;
2. validation evidence for the surfaces it touched;
3. generated artifact regeneration when source locations or DTO homes move;
4. a panel review before the next wave begins when the wave changes
   architecture or behavior;
5. no new compatibility surface without a removal row.

Panel reviewers should treat this ADR as a ratchet: a wave that only adds a
new abstraction without deleting duplication has not satisfied the roadmap,
even if tests pass.
