# Contract: Nix configuration surface

**Owning specs**: `ADR-046-nix-configuration`, `ADR-046-zone-routing` | **Waves**: W2, W5

## What this surface is

How an operator declares intent. Nix mirrors the canonical ResourceSpec `spec` shape directly;
only name, Zone, and `apiVersion` are derived or defaulted, and `status` is controller-owned
and never authored.

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

## Invariants

- New assertions need a matching nix-unit case; loosening an assertion silently converts a
  rejected misconfiguration into runtime breakage.
- Generated Nix is generated. Hand-editing a `generated/` file is a drift-gate failure.
- No new `nixpkgs.overlays` entry and no `nixpkgs.url` change.

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
