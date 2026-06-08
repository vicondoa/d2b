# 0006. Manifest bundle versioning

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "Manifest bundle: keep `vms.json` public-compatible, split sensitive control-plane artifacts into a root/service-readable bundle, and version the bundle without changing v0.4.0 manifest semantics."
- Companion ADRs: ADR 0002, [ADR 0007](0007-bash-coexistence-and-migration.md)

## Context

The v0.4.0 baseline ships `vms.json` at
`/run/current-system/sw/share/nixling/vms.json` with
`manifestVersion = 2`. Operators, scripts, and the mature bash CLI use
that public manifest as the compatibility surface for VM list and
capability metadata.

The portability plan adds a daemon-owned manifest bundle alongside that
public manifest. The bundle includes `bundle.json`, `host.json`,
`processes.json`, `privileges.json`, per-VM `closures/<vm>.json`,
`minijail/*.json`, and `seccomp/*.policy`. These artifacts carry host
requirements, process DAGs, broker authorization policy, closure paths,
minijail profile metadata, and seccomp policy sources.

Unlike `vms.json`, several new artifacts are security-sensitive. They
must not be world-readable from the system profile. W1 therefore needs a
single versioning policy that lets additive per-artifact changes land
without needless global churn, while still failing closed when a daemon,
broker, Nix emitter, schema, or prose contract drifts.

## Decision

1. `bundleVersion` is a single integer covering the whole artifact
   bundle. It is bumped on any breaking schema change across any bundle
   artifact.
2. Each artifact additionally carries its own `schemaVersion`. Additive
   per-artifact changes, such as a new optional field, use the artifact
   `schemaVersion` and do not require a `bundleVersion` bump.
3. The existing `vms.json` `manifestVersion`, currently `2`, is
   preserved unchanged through W1. W1 lands bundle metadata as
   additional fields and sibling artifacts, not by mutating `vms.json`
   semantics.
4. Rust wire types in `nixling-core` plus `schemars` are canonical for
   the JSON Schemas. Nix emitters validate against the generated
   schemas, and `tests/static.sh` static gates fail closed on any
   schema, documentation, emitter, Rust, or Nix divergence.
5. `deny_unknown_fields` is enforced for security-sensitive artifacts:
   `privileges.json`, `processes.json`, and `minijail/*.json`.
6. Public and private visibility are distinct. `vms.json` stays
   world-readable per v0.4.0. `bundle.json`, `host.json`,
   `processes.json`, `privileges.json`, `closures/<vm>.json`,
   `minijail/*.json`, and `seccomp/*.policy` install as
   root:`nixlingd` with mode `0640`.
7. The broker independently re-loads the trusted bundle per ADR 0002.
   Daemon-supplied paths, UIDs, capabilities, and authorization claims
   are never trusted.

## Consequences

1. Positive: W1 can add the bundle and schemas without breaking
   consumers that already read `vms.json` version 2.
2. Positive: Additive artifact-local changes avoid unnecessary global
   `bundleVersion` bumps while keeping breaking changes obvious.
3. Positive: The daemon and broker in W2 consume one trusted bundle
   contract with closed-world validation for sensitive inputs.
4. Positive: W10 release notes can call out any final `bundleVersion`
   bump as an intentional compatibility event.
5. Negative: Schema generation, Nix validation, prose docs, and static
   drift gates must be updated together whenever bundle artifacts
   change.

## Alternatives considered

- Bump `vms.json` `manifestVersion` for every bundle change: rejected
  because W1 must preserve the v0.4.0 public manifest semantics while
  private daemon artifacts are introduced.
- Version each artifact independently with no bundle-level version:
  rejected because daemon and broker compatibility must be decided for
  the artifact set as a whole.
- Rely on prose schemas only: rejected because Rust, Nix emitters, JSON
  Schemas, and tests need a single canonical contract.
- Make all bundle files world-readable like `vms.json`: rejected
  because process, privilege, closure, minijail, and seccomp artifacts
  expose control-plane details intended only for root and `nixlingd`.

## References

- plan.md, "Baseline: nixling v0.4.0"
- plan.md, "Manifest bundle"
- plan.md, "W1 Bundle and schema contract"
- [ADR 0007](0007-bash-coexistence-and-migration.md)
