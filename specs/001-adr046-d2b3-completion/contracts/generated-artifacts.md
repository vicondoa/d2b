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
| per-Zone `resource-bundle.json` | `zone-resources-json.nix` + `bundle-artifacts.nix` + `d2b-resource-compiler` | Zone runtime, core controllers | W5 emits only `schemaVersion: 4` / `bundleVersion: 2`; the required top-level compiler-only `audit` object is outside `resources`, and `contentHash` covers canonical `{audit,resources}` |
| `docs/reference/schemas/v3/resource-bundle.json` | `xtask gen-zone-schemas` from the active crate-root `ZoneBundle` | compiler, Nix and daemon contract tests, companions | Generated with the 4/2 change; no duplicate full-envelope DTO may generate a competing schema |
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
- Resource-bundle emitters, Rust consumers, JSON schema, digest reference, generated pins,
  tests, and changelog move atomically with the 4/2 version pair. No consumer may accept 3/1
  or future pairs 5/2, 4/3, or 5/3, and no consumer may synthesize a missing v4 `audit`
  object from defaults.
- An installed-host migration builds the complete 4/2 set before the NixOS generation switch.
  Failed build/install leaves the old generation active. Rollback restores the matching 3/1
  module, compiler, daemon, and artifacts together; it never presents 3/1 to 4/2 code.
- Version refusal is actionable: regenerate with
  `sudo nixos-rebuild switch --flake <host-flake>#<host>` and never hand-edit a generated or
  installed bundle.

## Acceptance

`make test-drift` is clean; no artifact is hand-edited; no delivery record appears in
`git status`; 4/2 passes while 3/1, mixed, 5/2, 4/3, and 5/3 fail at Rust, Nix, and daemon
boundaries; installed-host upgrade, failed activation, and whole-generation rollback tests
pass.
