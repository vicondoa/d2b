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
| target-closure `share/d2b/host-generation-rebuild-ref` | `host-daemon.nix` from required `d2b.site.hostGenerationRebuildRef` | Target-closure deployment entrypoint, broker handoff | Immutable trusted input in the target Nix closure; never the stable runtime reference |
| `/etc/d2b/host-generation-rebuild-ref` | `d2b-priv-broker` from the verified target-closure input | Handoff digest, post-bootstrap operator recovery | Broker-published bounded `<flake-ref>#<configuration-name>` reference; `root:d2bd` mode `0640`; file and directory durable; runtime binds only its digest and never renders the value or path |
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
- An installed-host migration starts through the target closure's
  `d2bHostGenerationDeploy` entrypoint with an explicit
  `<flake-ref>#<configuration-name>`; the first 3/1-to-4/2 migration never reads a file that
  4/2 has not published. The entrypoint builds and verifies the complete target closure,
  stages one immutable transition identity, and submits only an opaque request. It cannot
  publish a profile, control a service, mutate 3/1 bootstrap state, or initiate rollback.
  The target-closure entrypoint is executed only while unprivileged. Before any privileged
  invocation, the operator must pass the existing
  public-socket `SO_PEERCRED` plus `d2b`-group Admin classification. The broker consumes that
  one-shot classification into one durably sealed nonfabricable handoff capability bound to
  the complete intent and emits no authority token. The source broker also pins one exact
  immutable broker-managed apply object from the installed source generation. Only that
  object runs under `sudo`; it receives no flake URI, installable, stable-reference path, or
  caller-flake executable to reevaluate.
  Capability-authorized broker code exclusively owns stock profile publication,
  broker/daemon service transition, 3/1 bootstrap, d2b pointer/reference publication and
  repair, stock rollback, and source-service restoration. Before transfer that actor must be
  a source-generation-installed protocol-4 broker running as the ordinary `serve` process of
  the existing `d2b-priv-broker.service`; after durable transfer it is the target broker.
  Committed protocol 4 has no handoff operation and the installed service is pinned to its
  installed `brokerPackage`, so this feature remains blocked until an accepted external
  disposition installs that compatibility floor before migration. A target-closure-only
  mode, synthetic starting image, new unit or override, child, mutating entrypoint, or daemon
  recovery owner is not a substitute.
- The stock activation orders the target `d2b-priv-broker.service` before target `d2bd.service`.
  The broker verifies and audits the staged source/target identity. The target daemon starts
  and completes Hello for the exact target broker generation and protocol while unready, then
  presents a broker-issued phase attenuation in the authenticated publication request.
  Daemon identity, Hello, and bootstrap euid 0 never authorize independently. The broker
  publishes the pointer and reference with file and directory durability before daemon
  ingestion/readiness.
- A failed build leaves 3/1 active. Before first mutation the broker durably owns the
  coordinator; ownership transfers exactly once to the target broker before target
  daemon activation. A later failure is reopened by that broker-owned coordinator. Before
  transfer only the matching installed source compatibility actor may resume; after transfer the existing
  `d2b-priv-broker.service` reopens after restart and restores the prior pointer and stable
  reference bytes or verified absence before broker-owned stock rollback.
  Rollback therefore cannot leave a 4/2 reference on a restored 3/1 host.
- Nix activation stages immutable input only. Direct activation or daemon creation, repair,
  replacement, or removal of the stable reference fails policy tests. The broker uses
  create-exclusive temporary state, regular-file/owner/mode/link-count checks, atomic rename,
  file and parent-directory sync, fixed-digest audit fields, and the same operation for repair.
- Runtime version refusal is identifier-free and carries only closed action
  `rebuild-host-generation`; it contains no command or argv. Reference documentation gives
  parameterized paths: an unprivileged validated target-closure `--authorize-handoff`
  invocation followed by `--apply-authorized-handoff` on the separately pinned installed
  object for a 3/1 host where the stable reference is absent; an installed
  `d2b-host-generation-deploy --from-reference ... --authorize-handoff` invocation followed
  by the same pinned installed object's reference-free `--apply-authorized-handoff` only after
  broker publication; and the equivalent unprivileged prior-target authorization followed by
  installed-object apply for rollback. Every preflight validates
  grammar and bounds and stops before public-socket
  authorization or `sudo`; a failed authorization prevents the privileged invocation. No path contains a
  fixed illustrative target, invokes raw `nixos-rebuild` directly, or asks an operator to edit
  generated state. The value and stable path stay out of runtime diagnostics.

## Acceptance

`make test-drift` is clean; no artifact is hand-edited; no delivery record appears in
`git status`; 4/2 passes while 3/1, mixed, 5/2, 4/3, and 5/3 fail at Rust, Nix, and daemon
boundaries. Type-1 Nix evaluation pins the rebuild-reference grammar and bounds. Type-10
coverage starts with a 3/1 source generation that has independently installed the accepted
external versioned protocol-4 source-daemon/source-broker bridge and its broker-managed
immutable apply object while still lacking the target v5 operation. Bare committed protocol
4 without that source-side bridge is a refusal case. The positive case executes the
parameterized target-closure entrypoint, proves the caller-flake executable runs only
unprivileged and only validates/builds/stages/authorizes/submits, rejects zero-output and
multi-output resolution, proves privileged apply uses only the separately pinned installed
object with no URI or reference to reevaluate,
proves initial public-socket Admin classification, sealed durable capability,
broker-before-daemon activation, Hello while unready, phase-attenuated authenticated
publication request, and durable publication before ingestion/readiness, then injects failure and crash points through
profile/service/bootstrap/publication/reference repair/readiness/rollback. It kills the
entrypoint and proves the broker-owned coordinator resumes across target broker and daemon
startup failures, installed source compatibility-actor crashes, and durable ownership
transfer. Target-executable, apply-object, installed-symlink, and GC-root substitutions
refuse before mutation. Prior reference
bytes or absence, 3/1 artifacts, and source service generations are restored together with
immutable broker audit. Host recovery also executes the post-publication stable-reference and
parameterized prior-target rollback commands, rejects direct entrypoint/daemon/Nix mutation
plus missing or malformed values, and proves no sensitive reference value enters diagnostics.
The nonempty structural/API guard and poison fixture reject a second bundle
envelope or alias, version authority, hash implementation/entry point, or re-export through
the existing policy and fixture-contract gates.
