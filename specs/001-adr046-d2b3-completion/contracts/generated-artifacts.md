# Contract: Generated artifacts

**Owning specs**: `ADR-046-resource-object-model`, `ADR-046-nix-configuration`,
`ADR-046-validation-and-delivery` | **Waves**: W2-W7

## Why these are contracts

Generated artifacts are consumed by the broker, the daemon, the drift gates, and sibling
tools. The committed bytes are authoritative - `make test-drift` regenerates every ADR-046
artifact and requires a clean `git diff`, fail-closed.

## Artifact register

| Artifact | Generator | Consumers | Notes |
| --- | --- | --- | --- |
| `docs/reference/schemas/v3/core.d2bus.org_<Type>.schema.json` | `xtask gen-zone-schemas` | Nix eval, contract tests, companions | NEW; `v2/` remains until its paths retire |
| `nixos-modules/generated/resource-types.nix` | `xtask gen-zone-nix-options` | Nix option surface | NEW in W2 |
| `nixos-modules/generated/options-zones-<Type>.nix` | `xtask gen-zone-nix-options` | Nix option surface | NEW in W2, one per ResourceType |
| per-Zone `resource-bundle.json` | `zone-resources-json.nix` + `bundle-artifacts.nix` | Zone runtime, core controllers | Integrity-pinned |
| `docs/specs/ADR-046-spec-set.json` | `xtask spec-registry` | Gate 0, drift gate | Integrator-only; last commit of each wave |
| `docs/specs/ADR-046-work-items.json` | `xtask spec-registry` | Wave entry/seal checks | Same |
| `docs/specs/ADR-046-implementation-graph.{json,md}` | `xtask implementation-graph` | Wave planning, seal | Same |
| `/etc/d2b/ui-colors.{json,css}` | `nixos-modules/ui-colors.nix` | wlcontrol, Waybar, niri, wlterm | Public presentation metadata, never authz input |
| delivery snapshot, panel, seal records | `xtask delivery wave *` | Wave gate | **Never committed**; stored outside any git tree |

## Retirement

| Artifact | Disposition |
| --- | --- |
| `/etc/d2b/allocator.json` and its `allocator-json.nix` emitter | DELETE, no successor - explicit retirement list |
| `/run/d2b/allocator.sock` | DELETE, no successor - same cluster |

## Invariants

- Work-item and spec-set manifests are written by the integrator only, as the last commit of
  a wave, because every slice would otherwise contend on them.
- Delivery state must never enter git. The tooling refuses a state root inside a working tree,
  so this is structural rather than a convention.
- Downstream tools must fail visibly but remain usable when a public artifact is missing or
  malformed, without reading root-owned d2b state directly.

## Acceptance

`make test-drift` is clean; no artifact is hand-edited; no delivery record appears in
`git status`.
