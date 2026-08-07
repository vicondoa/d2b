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
| NIX-6 | Prove the exact-candidate positive operator path from a Nix declaration of the Wave 5 acceptance set - representative Volume, Network, and Device - through the emitted bundle and automatic startup/declaration/removal ingestion to durable reconciliation and each resource's real owned effect/readiness, then dependency-safe Device cleanup with Volume, Network, and unrelated resources still ready; refusal cases are separate. The acceptance set does not move Network implementation from Wave 4. Guest runtime-effect acceptance is deferred to the Wave 6 Guest Provider, and Guest emission, status, or refusal cannot satisfy this partial US1 production-plane checkpoint | FR-001, FR-005, FR-072, SC-034 | W5 |
| NIX-7 | The one carrier is compiler-only `d2b.zones.<zone>.audit`, emitted as the required top-level `audit` object in that Zone's `resource-bundle.json`, outside every ResourceSpec and the runtime-created empty `Zone.spec`. It carries exactly `retentionDays` (default 30, range 1-3650), `maxRecordsPerSegment` (default 65536, range 1-1000000), and `maxSegmentBytes` (default 67108864, range 1048576-1073741824). This breaking bundle-header change moves the accepted pair from `schemaVersion: 3` / `bundleVersion: 1` to `schemaVersion: 4` / `bundleVersion: 2`; v4 `contentHash` covers the canonical `{audit,resources}` object so an audit-only change creates a new generation identity. T592 owns the typed option, active crate-root `ZoneBundle`, retirement of the duplicate full envelope, compiler entry point and CLI tests, schema generator/output, digest reference, and focused tests; T595 wires the emitter and daemon; T220 coordinates generated artifacts, references, contract tests, and changelog treatment | FR-070, SC-032 | W5 |
| NIX-8 | Upgrade an installed 3/1 host through the target closure's `system.build.d2bHostGenerationDeploy` entrypoint. The first invocation is built from an explicit target installable and does not read `/etc/d2b/host-generation-rebuild-ref`. It builds and verifies the complete 4/2 closure, stages one durable transition identity, probes the source broker, and selects the closed v4-bootstrap path when the installed broker lacks protocol 5 and `ApplyHostGenerationHandoff`. System-profile publication plus NixOS activation/rollback remain owned by this one deployment entrypoint using the stock NixOS machinery; the plan does not claim that the old broker can perform a new operation. Target activation starts the target broker before the target daemon. The broker alone publishes the d2b bundle pointer and stable reference from immutable target inputs, and the daemon becomes ready only after Hello binds the exact target broker/daemon generations, protocol 5, capability digest, pointer, bundle set, and reference digest. On failure, the target broker restores the prior pointer and prior reference bytes or absence before the entrypoint performs stock rollback. Every profile/service transition has a durable pre-mutation transition record and immutable broker adoption/outcome audit; Nix activation and `d2bd` have no direct d2b pointer/reference mutation path. Identical deployment and crash replay produce no duplicate ingestion or effect | FR-070, SC-032 | W5 |
| NIX-9 | Declare required `d2b.site.hostGenerationRebuildRef` with no default. Its option type is `lib.types.strMatching "^[A-Za-z0-9+._~:/?@%=&,-]+#[A-Za-z0-9][A-Za-z0-9_-]{0,63}$"` plus an assertion that the UTF-8 encoding is at most 2048 bytes. The grammar is exactly `<flake-ref>#<configuration-name>`: one ASCII `#`; a nonempty ASCII flake ref using only the listed characters; and a 1-64 byte configuration name beginning with an alphanumeric and continuing with alphanumerics, `_`, or `-`. The option description states that it is an opaque rebuild locator, the example is `github:example/host-config?ref=v3#workstation`, and missing, empty, 2049-byte, multiline, control-bearing, whitespace-bearing, selector-free, extra-`#`, empty-selector, slash/dot-selector, or overlong-selector values fail evaluation. Nix places the exact validated bytes only in the immutable target closure. The broker publishes `/etc/d2b/host-generation-rebuild-ref` atomically as a regular `root:d2bd` `0640` file, audits only the fixed digest, repairs only through the same typed operation, and restores the prior bytes or absence on rollback. Runtime output never includes the value or stable path | FR-070, SC-032 | W5 |

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
  deployment entrypoint is the sole stock NixOS profile/activation/rollback initiator; broker
  ownership is limited to the d2b pointer, stable reference, their repair, and handoff audit.
  Target broker activation precedes target daemon activation, and the daemon refuses
  readiness until mandatory Hello negotiation matches the handoff's source/target broker and
  daemon generation digests, selected protocol, operation catalogue digest, bundle-pointer
  generation, and complete bundle set. `d2bd` and Nix activation have no direct d2b
  pointer/reference mutation or independent unit-control path.
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
  ingestion test, and exercises declaration and removal deployments through
  `d2bHostGenerationDeploy`, without a manual daemon restart or private reload, through the
  real activation/effect/cleanup boundary in
  `tests/host-integration/resource-operator-activation.nix` through the public
  `make test-host-integration` target. Every supported representative resource must reach its
  owned effect and readiness in the positive leg. Direct ResourceService calls, status-only
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
- Host activation coverage starts from an installed 3/1 generation whose broker advertises
  protocol 4 and lacks `ApplyHostGenerationHandoff`. It executes the documented explicit
  target-closure entrypoint, verifies the closed bootstrap selection before profile mutation,
  target broker activation before target daemon activation, mandatory protocol-5 Hello, and
  atomic broker publication of the pointer and stable reference. It injects crashes before
  and after transition-intent durability, stock profile publication, broker service
  transition, reference temporary-file sync/rename/directory sync, pointer publication,
  daemon Hello/readiness, rollback intent, reference/pointer restoration, and stock rollback.
  Every recovery path restores matching complete generations and the prior reference bytes or
  verified absence, with one logical ingestion/effect and no unaudited profile/service or d2b
  artifact transition. The same Type-10 test then executes the documented
  stable-reference-based entrypoint, rejects raw `nixos-rebuild` as the documented path,
  rejects direct daemon/Nix reference mutation and caller-claimed handoff authority, and
  proves malformed or absent values fail without rendering their contents.
