# Contract: Nix configuration surface

**Owning specs**: `ADR-046-nix-configuration`, `ADR-046-zone-routing` | **Waves**: W2, W5

## What this surface is

How an operator declares intent. Nix mirrors the canonical ResourceSpec `spec` shape directly;
only name, Zone, and `apiVersion` are derived or defaulted, and `status` is controller-owned
and never authored. Per-Zone compiler policy is the narrow exception: it is typed outside
`ResourceSpec` and carried in the versioned bundle header.

## Current state

`nixos-modules/options-zones.nix` and the focused resource modules provide the eval-time
option schema for the 19 ResourceTypes and qualified types. `bundle-zones.nix` now emits the
pinned per-Zone `zones/<zone>/resource-bundle.json` artifact. What remains unproved is the
complete operator activation from that emitted artifact through the production daemon,
controller-owned effect, and declared-resource removal cleanup.

## Obligations

| # | Obligation | Requirement | Wave |
| --- | --- | --- | --- |
| NIX-1 | Restructure `options-zones.nix` as the generated base; emit `generated/resource-types.nix` and per-type `generated/options-zones-<Type>.nix` via `xtask gen-zone-nix-options` | FR-001 | W2 |
| NIX-2 | Emit per-Zone resource generations with integrity pinning through `zone-resources-json.nix` and a new `bundle-artifacts.nix` row | FR-001 | W2 |
| NIX-3 | Add Zone assertions to `assertions.nix` (sole W2 writer of that file) | FR-001 | W2 |
| NIX-4 | Removing a declared resource activates the new generation immediately and requests async owner- and finalizer-safe deletion with visible cleanup status | FR-005 | W5 |
| NIX-5 | Extend the `eval-*` flake checks with Zone and resource examples | FR-032 | W5 |
| NIX-6 | Prove the exact-candidate positive operator path from a Nix declaration whose closed acceptance-resource identity set is exactly `Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm`, using each exact spec-pinned Provider/config fixture, through the emitted bundle and automatic startup/declaration/removal ingestion to durable reconciliation and that same identity's real owned effect plus production `Ready` projection. Then prove state-preserving Device cleanup with the ready, identity-stable, unrecreated acceptance Volume/Network and unrelated resources intact; refusal cases are separate. The acceptance set does not move Network implementation from Wave 4. Guest support objects are prerequisites only: Guest runtime-effect acceptance is deferred specifically to Wave 6 `Provider/runtime-cloud-hypervisor` T384/T479/T480, and Guest emission, ingestion, status, or refusal cannot satisfy this partial US1 production-plane checkpoint | FR-001, FR-005, FR-072, FR-075, SC-034, SC-035 | W5 |
| NIX-7 | The one carrier is compiler-only `d2b.zones.<zone>.audit`, emitted as the required top-level `audit` object in that Zone's `resource-bundle.json`, outside every ResourceSpec and the runtime-created empty `Zone.spec`. It carries exactly `retentionDays` (default 30, range 1-3650), `maxRecordsPerSegment` (default 65536, range 1-1000000), and `maxSegmentBytes` (default 67108864, range 1048576-1073741824). This breaking bundle-header change moves the accepted pair from `schemaVersion: 3` / `bundleVersion: 1` to `schemaVersion: 4` / `bundleVersion: 2`; v4 `contentHash` covers the canonical `{audit,resources}` object so an audit-only change creates a new generation identity. T592 owns the typed option, active crate-root `ZoneBundle`, retirement of the duplicate full envelope, compiler entry point and CLI tests, schema generator/output, digest reference, and focused tests; T595 wires the emitter and daemon; T220 coordinates generated artifacts, references, contract tests, and changelog treatment | FR-070, SC-032 | W5 |
| NIX-8 | Upgrade an installed 3/1 host through the target closure's `system.build.d2bHostGenerationDeploy` entrypoint only after an accepted external compatibility floor is installed in the source generation. The parameterized procedure validates the exact flake/configuration grammar and 2048-byte composition, resolves exactly one target output, discards raw Nix stderr, and stops before public-socket authorization or `sudo` on failure; first migration never reads the absent stable reference. The caller-flake entrypoint runs only unprivileged and may validate, build, stage, authorize, and submit only. The accepted external floor atomically owns the exact nonempty 13-member `SourceGenerationCompatibilityFloorV1` census under the existing socket/service lifecycle; every role occurs once under one disposition and source generation, and missing, stale, or cross-disposition members refuse. Only after both peers match the exact `source-handoff-v1` fingerprint may the source daemon transfer exactly one accepted public-socket evidence fd; the source broker's ordinary `serve` process consumes it into the nonfabricable intent-bound capability, pins the target object and separately pins the immutable apply object from trusted installed-generation metadata, durably owns and reopens the coordinator across existing-unit restart, performs audited pre-transfer phases, and transfers ownership exactly once. Privileged apply invokes only that installed pinned object, receives no flake URI, reference, or target executable to reevaluate, and refuses target-output, apply-object, symlink, or GC-root substitution. Its accepted connection is bound by a direct connection-scoped peer pidfd and live executable store/NAR/digest identity to the pin; every identity transition refuses before the first mutation and every later mutation edge, no selected or successor mutation occurs, and no pidfd persists. Raw peer PID/start and executable store/NAR identity remains absent from every output surface; only typed fixed correlation digests are permitted and metrics carry no identity label. T592 consumes the external source set read-only and owns only protocol-5 target adoption, post-transfer behavior, and target-v5 schema/catalogue/fingerprint/snapshot/fixture outputs. The target broker requires exact-generation protocol-5 Hello while the daemon is unready, publishes d2b pointer/reference state durably before ingestion/readiness, and restores prior bytes or absence before rollback. Bare committed protocol 4 with the field absent, or a source-peer catalogue mismatch, refuses. No target-only bootstrap mode, synthetic starting image, new unit or override, child, entrypoint mutation, daemon recovery owner, serialized credential, daemon identity, euid 0, or provenance claim substitutes. T589 and downstream Wave 5 remain blocked until the external floor is accepted and installed | FR-070, SC-032 | W5 |
| NIX-9 | Declare required `d2b.site.hostGenerationRebuildRef` with no default. Its option type is `lib.types.strMatching "^[A-Za-z0-9+._~:/?@%=&,-]+#[A-Za-z0-9][A-Za-z0-9_-]{0,63}$"` plus an assertion that the UTF-8 encoding is at most 2048 bytes. The grammar is exactly `<flake-ref>#<configuration-name>`: one ASCII `#`; a nonempty ASCII flake ref using only the listed characters; and a 1-64 byte configuration name beginning with an alphanumeric and continuing with alphanumerics, `_`, or `-`. The option description states that it is an opaque rebuild locator, provides no fixed target example, points to the parameterized validated quickstart, and makes missing, empty, 2049-byte, multiline, control-bearing, whitespace-bearing, selector-free, extra-`#`, empty-selector, slash/dot-selector, or overlong-selector values fail evaluation. Nix places the exact validated bytes only in the immutable target closure. The broker publishes `/etc/d2b/host-generation-rebuild-ref` atomically as a regular `root:d2bd` `0640` file, audits only the fixed digest, repairs only through the same typed operation, and restores the prior bytes or absence on rollback. Runtime output never includes the value or stable path | FR-070, SC-032 | W5 |

## Invariants

- New assertions need a matching nix-unit case; loosening an assertion silently converts a
  rejected misconfiguration into runtime breakage.
- Generated Nix is generated. Hand-editing a `generated/` file is a drift-gate failure.
- No new `nixpkgs.overlays` entry and no `nixpkgs.url` change.
- Missing, invalid, or unenforceable audit bounds fail closed. A journal row cannot be pruned
  before durable export completion plus `retentionDays`; prune or sync failure degrades and
  blocks publication of only the affected Zone.
- A Zone self-resource remains controller-created with byte-identical empty `Zone.spec`.
  Bundle emission must neither synthesize a Zone resource nor copy compiler-only `audit` into
  any resource. A missing top-level v4 `audit` object, an unknown field, an old/mixed version
  pair, future pair 5/2, 4/3, or 5/3, or a digest that omits `audit` is rejected before
  publication.
- Installed-host migration is whole-generation only. No activation may expose a 4/2 daemon
  to a partial or mixed bundle set, and no rollback may feed a 3/1 artifact to 4/2 code. The
  deployment entrypoint is unprivileged
  validation/build/stage/public-socket-authorization/opaque-request-only. It is never the
  privileged apply executable. Capability-authorized typed source broker code before transfer
  and target broker code after transfer exclusively own stock profile publication, broker/daemon service
  transition, 3/1 bootstrap, d2b pointer/reference publication/repair, stock rollback, and
  source-service restoration, with immutable pre-mutation and outcome audit. Target broker
  activation precedes target daemon activation. The daemon completes mandatory Hello while
  unready, then presents a phase attenuation in the authenticated publication request; the
  broker publishes durably before ingestion and readiness. Daemon identity and euid 0 alone
  refuse. The broker consumes initial public-socket Admin classification into the durable
  handoff capability and owns the coordinator before first mutation. Coordinator ownership
  transfers exactly once to the target broker before daemon activation. Before transfer only
  the matching installed source compatibility actor may resume; after transfer the existing broker service
  reopens across broker restart or daemon startup failure. `d2bd` and Nix activation have no
  recovery ownership, direct mutation, or independent
  unit-control path, and no fourth unit is introduced.
- Runtime refusal errors, JSON, wire output, logs, and `Debug` contain no host, Zone,
  generation, path, command, argv, or shell fragment. They carry only the closed
  `rebuild-host-generation` action. Documentation separately gives the explicit target
  installable command required before first publication and the stable-reference deployment
  command permitted afterward; it contains no unresolved replacement token and never points
  the first migration at an absent target-generated file.

## Acceptance

- A declared Zone with resources evaluates, emits its pinned generation, and is rejected at
  eval time when malformed.
- T604 pins the declaration and removal generations at the fixture-backed contract layer,
  consumes those exact generations through the Type-3 production daemon startup/change
  ingestion test, and exercises declaration and removal deployments for exactly
  the closed identity set `Volume/acceptance-state`, `Network/acceptance-net`, and
  `Device/acceptance-tpm` through
  `d2bHostGenerationDeploy`, without a manual daemon restart or private reload, through the
  real activation/effect/cleanup boundary in
  `tests/host-integration/resource-operator-activation.nix` through the public
  `make test-host-integration` target. Each exact resource's observed effect and production
  `Ready` projection must both carry that same exact resource identity; a missing, duplicate,
  unrelated, or mixed-identity member is rejected. Guest support objects remain
  prerequisites only: Guest runtime-effect acceptance is deferred specifically to Wave 6
  `Provider/runtime-cloud-hypervisor` T384/T479/T480 and cannot satisfy Wave 5. Direct
  ResourceService calls, status-only
  effects, actionable refusals, a skipped lane, or empty check discovery are ineligible for
  that positive proof. Evidence must enumerate and successfully build the exact
  `vmChecks.x86_64-linux.resource-operator-activation` attr.
- `make test-drift` is clean after regeneration.
- The Type-1 case `tests/unit/nix/cases/host-generation-rebuild-ref.nix` covers a normal
  reference, the exact 2048-byte boundary, missing required option, 2049 bytes, every
  malformed character/line case, missing/empty/extra selector, invalid selector characters,
  and selector lengths 64 and 65. Adding it requires `make nix-unit-pin`; a Type-10 result is
  not a substitute for these pure evaluation cases.
- Nix-unit and bundle tests pin every audit default, lower/upper bound, unknown field, and
  out-of-range refusal; pin exact bundle versions 4/2 and an audit-only generation change;
  reject versions 3/1, every mixed pair, 5/2, 4/3, 5/3, missing `audit`,
  ResourceSpec/ZoneSpec placement, and consumer-side silent defaulting. Production tests pin
  post-export-only journal retention and degraded health on prune or file/directory-sync
  failure.
- Host activation positive coverage starts from an installed 3/1 generation whose accepted
  external compatibility floor atomically provides the exact nonempty 13-member
  `SourceGenerationCompatibilityFloorV1` census in `data-model.md`. Every role occurs exactly
  once under one accepted disposition and source generation. Separate per-role missing,
  empty/stale, and cross-disposition poison cases refuse before fd transfer, authorization,
  or mutation. Separate negatives also start from bare
  committed protocol 4 and from mismatched source-peer fingerprints and MUST refuse before
  fd transfer, authorization, or mutation. The positive executes the documented parameterized
  target-closure entrypoint with validated flake/configuration inputs and rejects empty,
  malformed, over-bound, mismatched, or nonexistent inputs plus zero-output or multi-output
  target resolution before public-socket authorization or `sudo`. It verifies the
  caller-flake executable runs only for unprivileged `--authorize-handoff`; privileged
  `--apply-authorized-handoff` invokes only the independently pinned installed apply object
  with no URI, reference, or target executable to reevaluate. The broker binds that accepted
  connection's direct peer pidfd and live executable identity to the pin and rejects exit,
  exec, PID reuse, start-identity mismatch, executable mismatch, or ambiguity before mutation
  without persisting a pidfd. After the first privileged mutation, each transition is
  injected independently before every later mutation edge and must refuse before that edge
  or any successor executes. Raw peer PID/start and executable store/NAR identity are absent
  from human, JSON, wire, error, log, span, metric, audit, and `Debug` output; only typed
  fixed correlation digests are permitted, and metrics carry no peer-identity label. The
  test also proves the sealed capability, no emitted authority token, and that
  daemon identity/euid0 alone refuse. It verifies the entrypoint is
  unprivileged validation/build/stage/authorization/request-only, the external source actor
  selection,
  capability-authorized broker-only audited
  profile/service mutation, target broker activation before target daemon activation, daemon
  Hello while unready, phase-attenuated authenticated publication request, and atomic broker publication
  before ingestion/readiness. It injects crashes before
  and after transition-intent durability, stock profile publication, broker service
  transition, reference temporary-file sync/rename/directory sync, pointer publication,
  daemon Hello/readiness, rollback intent, reference/pointer restoration, and stock rollback.
  Every recovery path, including one where the entrypoint is killed, is resumed by the
  broker-owned coordinator and restores matching complete generations and the prior
  reference bytes or verified absence, with one logical ingestion/effect and no unaudited
  profile/service/bootstrap/rollback or d2b artifact transition. It injects target broker
  startup failure, target daemon startup/reconciliation failure, every installed source
  compatibility-actor crash
  boundary, and both sides of durable ownership transfer without a new unit. The same Type-10 test then
  executes the documented
  stable-reference-based entrypoint, rejects raw `nixos-rebuild` as the documented path,
  executes the parameterized prior-target rollback procedure, rejects direct
  entrypoint/daemon/Nix mutation and caller-claimed handoff authority, and proves malformed or
  absent values fail without rendering their contents. Independent target-executable,
  apply-object, installed-symlink, and GC-root substitutions refuse before mutation. T592's
  evidence owns only target-v5 adoption and target artifacts; the source peer and source
  artifact atomicity is evidence of the independently accepted external floor. The host case
  also checks the complete loaded `d2b*`/`microvm*` unit namespace: a nonzero
  `systemctl list-units --all` result fails before filtering, exactly canonical `d2b.slice`
  is excluded, and the sorted remainder must contain exactly the three lifecycle units
  `d2bd.service`, `d2b-priv-broker.socket`, and `d2b-priv-broker.service`. Separate
  unexpected-slice and unexpected-service poison cases
  survive that sole exclusion and fail exact equality.
