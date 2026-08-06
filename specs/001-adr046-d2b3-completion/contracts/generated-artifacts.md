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
| per-Zone `resource-bundle.json` | active chain: `bundle-zones.nix` + `d2b-resource-compiler` + `bundle-artifacts.nix`; compatibility-only legacy input: `zone-resources-json.nix` | Zone runtime, core controllers | W5 emits only `schemaVersion: 4` / `bundleVersion: 2`; the required top-level compiler-only `audit` object is outside `resources`, and `contentHash` covers canonical `{audit,resources}`. `zone-resources-json.nix` cannot emit, version, hash, or publish the active bundle |
| `docs/reference/schemas/v3/resource-bundle.json` | `xtask gen-zone-schemas` from the active crate-root `ZoneBundle` | compiler, Nix and daemon contract tests, companions | Generated with the 4/2 change; no duplicate full-envelope DTO may generate a competing schema |
| `/etc/d2b/host-generation-rebuild-ref` | `host-daemon.nix` from required `d2b.site.hostGenerationRebuildRef` | Handoff digest, operator recovery command | Complete bounded single-line flake output reference; `root:d2bd` mode `0640`; runtime binds only its digest and never renders the value or path |
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
- `bundle-zones.nix`, `d2b-resource-compiler`, and `bundle-artifacts.nix` are the only active
  emission/publication chain. `zone-resources-json.nix` is compatibility-only and cannot be
  an independent envelope, version, hash, or publication authority.
- An installed-host migration builds the complete 4/2 set before the NixOS generation switch.
  Failed build/install leaves the old generation active. Only the typed privileged broker
  adapter publishes the system profile and generation pointer, restarts the existing
  `d2bd.service`, or rolls back. Each phase has an immutable audit record, and direct
  daemon/Nix mutation is rejected. Rollback restores the matching 3/1 module, compiler,
  daemon, and artifacts together; it never presents 3/1 to 4/2 code.
- Runtime version refusal is identifier-free and carries only closed action
  `rebuild-host-generation`; it contains no command or argv. Reference documentation alone
  says to run `sudo nixos-rebuild switch --flake "$(sudo cat /etc/d2b/host-generation-rebuild-ref)"`
  and never hand-edit a generated or installed bundle. The stable reference file removes
  unresolved placeholders; its value and path stay out of runtime diagnostics.

## Acceptance

`make test-drift` is clean; no artifact is hand-edited; no delivery record appears in
`git status`; 4/2 passes while 3/1, mixed, 5/2, 4/3, and 5/3 fail at Rust, Nix, and daemon
boundaries; installed-host upgrade, failed activation, and whole-generation rollback tests
pass through the typed broker adapter with one audit row per privileged phase. Host recovery
coverage executes the documented stable-reference discovery command, rejects direct
daemon/Nix mutation plus missing or malformed reference values, and proves no sensitive
reference value enters diagnostics. The nonempty structural/API guard and poison fixture
reject a second bundle envelope or alias, version authority, hash implementation/entry point,
or re-export through the existing policy and fixture-contract gates.
