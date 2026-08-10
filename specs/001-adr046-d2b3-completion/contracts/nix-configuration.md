# Contract: Nix configuration surface

**Owning specs**: `ADR-046-nix-configuration`, `ADR-046-zone-routing` | **Waves**: W2, W5, W6

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
controller-owned effect, and declared-resource removal cleanup. Active local T604 owns that
positive proof after authoritative provider work merges. W5 retains the emitted-bundle,
source-generation compatibility, deployment, and double-opt-in contract prerequisites only.
`zone-resources-json.nix` is retained only as a historical/compatibility input. It cannot
emit, version, hash, or publish the active envelope. The sole canonical active chain is
`bundle-zones.nix` -> `d2b-resource-compiler` -> `bundle-artifacts.nix` -> installed
`resource-bundle.json`.

## Obligations

| # | Obligation | Requirement | Wave |
| --- | --- | --- | --- |
| NIX-1 | Restructure `options-zones.nix` as the generated base; emit `generated/resource-types.nix` and per-type `generated/options-zones-<Type>.nix` via `xtask gen-zone-nix-options` | FR-001 | W2 |
| NIX-2 | Historical W2 compatibility input only: `zone-resources-json.nix` supplied legacy resource JSON but is not an active bundle emitter, version, hash, or publication authority. The canonical active path is `bundle-zones.nix` -> `d2b-resource-compiler` -> `bundle-artifacts.nix` -> installed per-Zone `resource-bundle.json` | FR-001 | W2 |
| NIX-3 | Add Zone assertions to `assertions.nix` (sole W2 writer of that file) | FR-001 | W2 |
| NIX-4 | Removing a declared resource activates the new generation immediately and requests async owner- and finalizer-safe deletion with visible cleanup status | FR-005 | W5 |
| NIX-5 | Extend the `eval-*` flake checks with Zone and resource examples | FR-032 | W5 |
| NIX-6 | Active local T604 authors and development-validates the exact Volume/Network/Device path and state-preserving cleanup after T221 and every exact workItemId in `tasks.md` `required_manifest_dependencies.T604`. It owns `tests/golden/delivery/host-generation-pre-start-case-ids.txt` and `tests/golden/delivery/host-generation-unit-census-case-ids.txt`, authors and development-validates the `operator-nix-activation-cleanup` validator, and authors the daemon-restart case. It emits no candidate-bound record; after F6 freezes, T479 invokes the operator validator, emits its one record, and alone executes candidate-bound FR-075. | FR-001, FR-005, FR-072, FR-075, SC-034, SC-035 | active feature-local T604 |
| NIX-7 | Historical Wave 5 design for the compiler-only audit carrier and bundle-version transition. Exact retired ownership remains only in fenced history. | FR-070, SC-032 | historical W5 |
| NIX-8 | Code canon has no production host-generation handoff. The accepted activation-nixos specification and `ADR046-activation-001` own the broker-only handoff after T607/T609; T606 owns its shared contract/dispatch prep. | FR-070, SC-032 | prospective W6 activation foundation |
| NIX-9 | Code canon has no rebuild-reference option, emitter, or test. `ADR046-activation-006` owns the carrier after `ADR046-activation-001` and T606/T607 foundations. | FR-070, SC-032 | prospective W6 activation foundation |

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
- `d2b host-generation apply-authorized-handoff` has no selector or authority token. Under one coordinator lock,
  the broker permits at most one durable nonterminal intent per source generation and
  atomically claims only the sole `authorized-pending` intent for the accepted apply
  connection. Zero, multiple, concurrent, and terminal selections refuse before mutation.
  A pre-mutation disconnect releases only after a durable zero-mutation proof; after any
  mutation, only coordinator replay of the same intent may bind the same pinned apply object
  after proving the old peer dead. No command retry guesses or reapplies a terminal intent.
- Runtime refusal errors, JSON, wire output, logs, and `Debug` contain no host, Zone,
  generation, path, command, argv, or shell fragment. They carry only the closed
  `rebuild-host-generation` action. Documentation separately gives the explicit target
  installable command required before first publication and the stable-reference deployment
  command permitted afterward; it contains no unresolved replacement token and never points
  the first migration at an absent target-generated file.

## Acceptance

- A declared Zone with resources evaluates, emits its pinned generation, and is rejected at
  eval time when malformed.
- Active local T604 pins declaration and removal generations at the fixture-backed contract layer,
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
  manifest-backed `ADR046-ch-001` plus local T479/T480 and cannot satisfy the authoritative
  acceptance row. Direct
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
- Active local T604 host-activation coverage consumes the accepted activation-nixos contract
  and completed `ADR046-activation-001`/`ADR046-activation-006` implementation after T607
  and T609. Those rows own source-floor membership, fixture identities, poison cases, and
  transition ordering; this contract does not copy them. The Type-10 positive runs the
  parameterized target-closure entrypoint from an independently installed source floor,
  proves unprivileged request-only entrypoint behavior, broker-only mutation, one durable
  coordinator transfer, target-before-daemon activation, crash/rollback continuation, exact
  target/apply/GC-root/live-peer pinning, and raw-identity redaction. Missing, stale,
  wrong-owner, non-ancestor, runtime-derived, skipped, or failing generated coverage refuses
  before mutation. The same host case fails on unit listing error, excludes only canonical
  `d2b.slice`, and requires exactly `d2bd.service`, `d2b-priv-broker.socket`, and
  `d2b-priv-broker.service` after the exclusion.
