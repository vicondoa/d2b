### Changed

- `EndpointSpec.purpose` values are closed tokens everywhere: the SecurityKey
  relay Endpoint now declares `security-key-ctaphid-relay` and the
  credential-entra login Endpoint `credential-entra-login` (constant, Nix
  assertion, unit fixture, README and dossiers renamed in one pass), so the
  two endpoints that still carried pre-contract dotted spellings decode.

### Fixed

- The SecurityKey relay Endpoint child spec now decodes as the closed
  `EndpointSpec`: its `purpose` and its `allowedProviderComponents` entry were
  dotted tokens and its `attachmentPolicy` was a bare string where the
  contract requires `{supported, maxAttachments}`. The driver test decodes the
  whole child spec, which is the assertion that would have caught the three.
- `make changelog-fold` (the Bazel/Make entry point) deleted every fragment
  without folding the changelog when the tree was reached through symlinks;
  the fold now resolves the changelog and fragment directory once, writes
  through the resolved paths and refuses - before consuming anything - when
  they do not resolve into one repository root (#519).
- `tests/runtime_boundary.rs`'s runtime file list now covers all 60 sources
  (the two omissions predated the rewrite).

### Removed

- Retired the `OpenZoneStore` broker operation end to end: the wire
  request/response and their structs, the broker dispatch arm, handler and
  `ops/zone_store.rs`, the audit-field variant, the privilege row, the op's
  integration test and profile entries, with the generated wire/privilege
  schemas and `docs/reference/daemon-api.md` regenerated. Nothing in the
  daemon sent the op since the resource-store cutover, and the store it opened
  no longer exists.
- Deleted the controller toolkit's dead surface: `owner_hints.rs`,
  `state_migration.rs` and the zero-consumer `ControllerDescriptor`/`TriggerSet`
  cluster in `contract.rs` (`ResourceKey`, `ResourceSnapshot` and
  `DependencySnapshot` stay - they are what `d2b-core-controller` consumes).
