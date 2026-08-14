# Contract: Generated artifacts

**Owning specs**: resource object model, Nix configuration, and the product contracts in this
directory.

## Why these are contracts

Generated artifacts are consumed by the broker, daemon, controllers, Nix evaluation, contract
tests, and companions. A generator-owned artifact must be regenerated with its ecosystem tool,
reviewed for a clean diff, and changed in lockstep with its source contract. Product code and
its focused drift checks determine ownership.

## Artifact register

| Artifact | Generator or owner | Consumers | Notes |
| --- | --- | --- | --- |
| `docs/reference/schemas/v3/core.d2bus.org_<Type>.schema.json` | `xtask gen-zone-schemas` | Nix eval, contract tests, companions | Versioned resource schemas |
| `nixos-modules/generated/resource-types.nix` and per-type options | `xtask gen-zone-nix-options` | Nix option surface | Generated from the resource contract |
| per-Zone `resource-bundle.json` | `bundle-zones.nix` -> `d2b-resource-compiler` -> `bundle-artifacts.nix` | Zone runtime and controllers | Active bundles use schema 4 / bundle 2; top-level `audit` is outside `resources`, and `contentHash` covers canonical `{audit,resources}` |
| `docs/reference/schemas/v3/resource-bundle.json` | active crate-root `ZoneBundle` schema generator | compiler, Nix, daemon tests, companions | No competing full-envelope DTO may generate a schema |
| target-closure `share/d2b/host-generation-rebuild-ref` | Nix configuration owner | broker handoff | Must be an explicit target installable output |
| `/etc/d2b/host-generation-rebuild-ref` | broker handoff owner | recovery and stable-reference checks | Must be validated and pinned before privileged apply |
| `/etc/d2b/ui-colors.{json,css}` | `nixos-modules/ui-colors.nix` | desktop presentation | Never an authorization input |

## Invariants

- Resource-bundle emitters, Rust consumers, JSON schema, digest reference, generated pins, tests,
  and changelog move atomically with the 4/2 version pair. Consumers reject 3/1 and future or
  mixed pairs and never synthesize a missing v4 `audit` object.
- `bundle-zones.nix`, `d2b-resource-compiler`, and `bundle-artifacts.nix` are the only active
  emission/publication chain, in that order. The compatibility emitter cannot publish or hash
  an active bundle independently.
- Installed-host migration starts through the target closure's
  `d2bHostGenerationDeploy` entrypoint with an explicit flake reference. The entrypoint builds,
  verifies, stages one immutable transition identity, and submits only an opaque request. It
  cannot publish a profile, control a service, mutate bootstrap state, or initiate rollback.
- Before privileged invocation, the operator passes public-socket `SO_PEERCRED` plus current
  `d2b`-group Admin classification. The broker consumes that evidence into one sealed,
  nonfabricable capability. The source broker pins one immutable apply object from trusted
  installed metadata; privileged apply receives no flake URI, installable, stable-reference
  path, target executable, or caller executable to reevaluate.
- The accepted apply connection's peer pidfd and executable store/NAR/digest identity are
  bound to the pinned object and revalidated before every mutation. Exit, exec, PID reuse,
  mismatch, and ambiguity refuse. Pidfds and executable descriptors are never serialized or
  persisted, and raw peer identity never reaches human, JSON, wire, error, log, span, metric,
  audit, panic, or `Debug` output.
- The existing `d2b-priv-broker.service` remains the lifecycle owner before and after transfer.
  Ownership moves exactly once before target-daemon activation; the broker-owned coordinator
  resumes or rolls back after entrypoint, broker, or daemon failure. No new unit or supervisor
  is introduced.
- Runtime refusal is identifier-free and carries only the closed action
  `rebuild-host-generation`. Documentation uses parameterized authorization and apply commands,
  validates all values before socket authorization or `sudo`, and emits no sensitive reference
  value in diagnostics.
- The apply command carries no intent selector or authority token. A coordinator lock protects
  zero-or-one nonterminal intent and atomically claims only the sole authorized pending intent.
  Zero, multiple, concurrent, or terminal selection refuses before mutation; post-mutation
  replay is allowed only for the same intent after the old peer is proven dead.

## Acceptance

Run the focused generator, schema, Nix, and daemon contract checks for the changed artifact.
Verify that no generated output is hand-edited, no sensitive value enters diagnostics, the 4/2
bundle succeeds while 3/1 and mixed versions fail, and handoff crash/identity/rollback cases
are covered. Run host or live acceptance only when the changed artifact needs it. `make check`
is an optional broad check, not a mandatory pre-PR gate.
