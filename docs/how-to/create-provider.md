# Create a Provider

This guide is the short path for adding or maintaining one d2b Provider. A
Provider is one identity, one independently buildable crate, and one signed
package. Package presence does not install a `Provider/<name>` resource.

## Start with the dossier and identity

Choose one lower-case Provider identity, such as `device-gpu`, and read its
normative dossier in
[`docs/specs/providers/`](../specs/providers/README.md). The crate name is
`d2b-provider-<base>-<implementation>`, and the crate README must declare the
same Provider name. Do not add a crate for a Provider that is only listed in
the catalog until its owning implementation work is ready.

The authored resource selects an artifact and its root configuration:

```nix
d2b.zones.dev.resources.device-gpu = {
  type = "Provider";
  spec = {
    artifactId = "device-gpu";
    config = { };
  };
};
```

`artifactId` is resolved through the signed catalog. The configuration is
validated against the Provider package's signed settings schema; it is not a
place for host paths, executable paths, device selectors, credentials, or
runtime state.

## Use the uniform crate layout

Every in-scope Provider crate under `packages/` contains:

```text
packages/d2b-provider-<base>-<implementation>/
├── Cargo.toml
├── src/
├── tests/
├── integration/
│   └── README.md
└── README.md
```

`src/` contains the Provider implementation and colocated unit tests.
`tests/` contains hermetic Cargo tests. `integration/` is reserved for
declared container, Host, Guest, or cross-process fixtures and contains an
integration README even before executable fixtures are wired. The repository
layout policy checks these paths, the dependency direction, and the Provider
identity.

## Keep the README contract

The crate README has these headings, using this spelling:

1. Provider identity
2. Config schema
3. Exported resource types
4. Controllers / services / workers / binaries
5. Placement and dependencies
6. RBAC requirements
7. Security posture
8. State and telemetry
9. Build and test

Document the one Provider identity, its configuration fields and bounds, the
resource and status authority it owns, process placement, effect boundaries,
RBAC, redaction, state custody, bounded telemetry labels, and exact commands.
The README should link back to this guide rather than copying the entire
packaging contract.

## Find the schema and Nix source

Use the Provider dossier and the shared catalog together:

- The standard v3 ResourceType schemas are in
  [`docs/reference/schemas/v3/`](../reference/schemas/v3/). Core artifacts use
  the `core.d2bus.org_<ResourceType>.schema.json` filename namespace.
- Primitive Nix schema modules are in
  [`nixos-modules/resource-schemas/`](../../nixos-modules/resource-schemas/).
  This is the primitive schema-module boundary.
- Resource authoring and bundle validation live in the
  [`nixos-modules/resources-*.nix`](../../nixos-modules/) modules and the
  generated resource catalog.
- Semantic audio and security-key schemas are qualified shared Service and
  Binding contracts. They remain in `docs/reference/schemas/v3/`; they do not
  belong in `nixos-modules/resource-schemas/`.

Provider-specific configuration and status extensions are qualified schemas
described by the Provider dossier and signed manifest. Keep common fields in
the shared base schema and implementation-only fields in the Provider
extension.

## Use the toolkit

The toolkit supplies neutral controller, audit, dispatch, fake-port, and
conformance helpers. It does not own a Provider identity. Its manifest CLI
canonicalizes and verifies the bytes used by a Provider package:

```bash
cargo run --manifest-path packages/Cargo.toml \
  -p d2b-provider-toolkit --bin d2b-provider-toolkit -- \
  manifest emit --out build/provider-manifest.json < provider-manifest-input.json
cargo run --manifest-path packages/Cargo.toml \
  -p d2b-provider-toolkit --bin d2b-provider-toolkit -- \
  manifest verify build/provider-manifest.json
```

The first command reads a `ProviderManifest` JSON document from standard
input and writes canonical JSON. The second prints `canonical` or reports the
first divergent byte and the remediation command.

## Run hermetic tests

Run the crate tests without a container, VM, live host, or physical device:

```bash
cargo nextest run --manifest-path packages/Cargo.toml -p d2b-provider-<base>-<implementation>
cargo test --manifest-path packages/Cargo.toml -p d2b-provider-<base>-<implementation>
make test-policy
```

The layout policy is the enforcing check for the crate shape. Keep schema
round trips, conformance, fault injection, redaction, and effect-port tests
in `tests/` or the crate's `src/` unit-test modules.

## Declare and run heavy tests

An executable `integration/*.rs` fixture declares exactly one target in its
first twenty lines:

```rust
//! integration-target: container
```

Use `container` for a foreign-userland or process fixture and
`host-integration` for a NixOS/Host/Guest fixture. Run the public lanes, which
acquire the shared heavy-test slot:

```bash
make test-integration
make test-host-integration
```

Use `make test-hardware` only for a fixture that genuinely requires a GPU,
security key, TPM, or other physical device. Do not invoke an integration
script or an internal heavy-lane target directly.

Several current Provider directories are scaffolding and intentionally have
an integration README but no executable runtime fixture:
`credential-entra`, `credential-managed-identity`, `credential-secret-service`,
`system-core`, `system-minijail`, `system-systemd`, and `volume-virtiofs`.
Their README files record the intended scenarios and the hermetic tests remain
the available evidence. Do not add a fake "basic integration" test or claim
runtime wiring until the owning implementation work lands.
