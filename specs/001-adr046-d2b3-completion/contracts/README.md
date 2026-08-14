# Contracts: Complete the d2b Provider Control Plane

**Feature**: `001-adr046-d2b3-completion`

d2b exposes contracts to operators, guest components, Nix consumers, and desktop
companions. This directory records the contract surfaces that the product must deliver,
change, or retire. Architectural ADR references remain useful where they explain existing
interfaces and their focused validation evidence.

## Contract surfaces

| File | Surface | Consumers |
| --- | --- | --- |
| [resource-api.md](./resource-api.md) | `d2b.resource.v3` and ComponentSession admission | Components, controllers, Providers |
| [operator-cli.md](./operator-cli.md) | `d2b` command surface | Operators and companions |
| [nix-configuration.md](./nix-configuration.md) | `d2b.zones.<zone>.resources.*` options | Host configurations |
| [generated-artifacts.md](./generated-artifacts.md) | Schemas, bundles, UI colors, and handoff artifacts | Broker, daemon, companions |
| [companion-contracts.md](./companion-contracts.md) | Public surfaces consumed by desktop companions | Sibling repositories |

## Shared contract rules

1. **Versioned, not silently changed.** Adding, removing, or renaming a field or operation
   bumps the relevant version and updates the paired schema, prose, tests, and emitter.
2. **Generated artifacts move together.** Regenerate schema and API outputs with the ecosystem
   tool and keep focused drift checks clean; do not hand-edit generated output.
3. **Documentation ships with behavior.** A reference page affected by a change lands with
   that behavior.
4. **No compatibility by accident.** A removed capability requires a removal proof, successor
   behavior where promised, a justification, and release-note treatment.
5. **Nothing leaks.** Secrets, credentials, commands, raw host paths, and PII do not cross
   these surfaces into telemetry, audit, logs, or errors.
6. **Production evidence crosses production boundaries.** Resource acceptance enters through
   registrar-admitted, pidfd-bound ComponentSession and the published ZoneBus route. Restart
   uses a fresh pidfd; PID reuse, mismatch, `ESRCH`, or ambiguity denies.
7. **One readiness projection, no partial publication.** Store, matching policy,
   session/router, controller endpoint, watch admission, audit catch-up, and the
   `Provider/system-core` registration publish together with exactly one
   `system-core-host` and one `system-core-user` handler record per Zone, or that Zone
   remains unpublished and degraded.
8. **Policy bootstrap is private and one-shot.** The first exact-revision policy snapshot uses
   one sealed private capability. Later access is authenticated Resource API access; store
   crates remain policy-neutral.
9. **A committed mutation is never unaudited or reported as rolled back.** Its immutable
   journal row commits transactionally. Until export completion, return operation-bound
   `CommittedPendingAudit` through the layered `ResourceStatus`, keep the Zone unpublished,
   and permit only exact replay-bound retry or inspection.

## Product acceptance

Run focused tests for every changed contract and its negative fixtures. Run container, host,
live, hardware, or performance checks only when the changed surface requires them. Record the
exact commands and deliberate omissions; an optional broad check is not a substitute for
focused evidence.
