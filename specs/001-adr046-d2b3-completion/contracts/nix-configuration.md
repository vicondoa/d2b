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
| NIX-6 | Prove the exact-candidate positive operator path from a Nix declaration of every supported representative Guest, Volume, Network, and Device through the emitted bundle and automatic startup/declaration/removal ingestion to durable reconciliation and each resource's real owned effect/readiness, then dependency-safe removal cleanup with unrelated resources still ready; refusal cases are separate | FR-001, FR-005, FR-072, SC-034 | W5 |
| NIX-7 | The one carrier is compiler-only `d2b.zones.<zone>.audit`, emitted as the required top-level `audit` object in that Zone's `resource-bundle.json`, outside every ResourceSpec and the runtime-created empty `Zone.spec`. It carries exactly `retentionDays` (default 30, range 1-3650), `maxRecordsPerSegment` (default 65536, range 1-1000000), and `maxSegmentBytes` (default 67108864, range 1048576-1073741824). This breaking bundle-header change moves the accepted pair from `schemaVersion: 3` / `bundleVersion: 1` to `schemaVersion: 4` / `bundleVersion: 2`; v4 `contentHash` covers the canonical `{audit,resources}` object so an audit-only change creates a new generation identity. T592 owns the typed option, active crate-root `ZoneBundle`, retirement of the duplicate full envelope, compiler entry point and CLI tests, schema generator/output, digest reference, and focused tests; T595 wires the emitter and daemon; T220 coordinates generated artifacts, references, contract tests, and changelog treatment | FR-070, SC-032 | W5 |
| NIX-8 | Upgrade an installed 3/1 host by building every Zone's complete 4/2 bundle set into the immutable new NixOS closure, staging that set behind one generation pointer, atomically publishing the pointer with the system-profile switch, then using only the existing `d2bd.service` continuation. Compile, stage, or publish failure leaves the complete old generation/pointer active. Rollback restores the matching 3/1 module, compiler, daemon, pointer, and bundle set as one NixOS generation; a 4/2 daemon never consumes a retained 3/1 or mixed bundle. Version refusal names `sudo nixos-rebuild switch --flake <host-flake>#<host>` and says not to edit installed JSON | FR-070, SC-032 | W5 |

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
  to a partial or mixed bundle set, and no rollback may feed a 3/1 artifact to 4/2 code.

## Acceptance

- A declared Zone with resources evaluates, emits its pinned generation, and is rejected at
  eval time when malformed.
- T604 pins the declaration and removal generations at the fixture-backed contract layer,
  consumes those exact generations through the Type-3 production daemon startup/change
  ingestion test, and exercises public NixOS declaration and removal switches, without a
  manual daemon restart or private reload, through the real activation/effect/cleanup boundary in
  `tests/host-integration/resource-operator-activation.nix` through the public
  `make test-host-integration` target. Every supported representative resource must reach its
  owned effect and readiness in the positive leg. Direct ResourceService calls, status-only
  effects, and actionable refusals are ineligible for that positive proof.
- `make test-drift` is clean after regeneration.
- Nix-unit and bundle tests pin every audit default, lower/upper bound, unknown field, and
  out-of-range refusal; pin exact bundle versions 4/2 and an audit-only generation change;
  reject versions 3/1, every mixed pair, 5/2, 4/3, 5/3, missing `audit`,
  ResourceSpec/ZoneSpec placement, and consumer-side silent defaulting. Production tests pin
  post-export-only journal retention and degraded health on prune or file/directory-sync
  failure.
- Host activation coverage starts from an installed 3/1 generation, proves an atomic rebuild
  to 4/2, proves failed pre-activation compilation/install leaves 3/1 active, and rolls back
  the complete matching generation. The 4/2 refusal text includes the exact regeneration
  command.
