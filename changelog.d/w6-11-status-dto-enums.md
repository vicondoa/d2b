### Changed

- d2b-contracts-control: the status DTOs that carried daemon-side vocabularies
  as free-form strings now use closed kebab-case enums, and the serde names
  match the emitted wire strings so no emitted byte moves. `RealmMode` and
  `RealmGatewayState` type the realm rows (`RealmPolicyOutputV1`,
  `OpInspectRealmOutputV1`; the historical `not reported by d2bd` sentinel
  stays representable), `QemuMediaRunnerState` and `QemuMediaRegistryState`
  type the guest-media status, `PublicReadModelKind` types
  `PublicReadModelMetadata.kind`, and `VmAutostartMode` types
  `VmAutostartPosture.mode`. The generated CLI schemas, the v2 wire-protocol
  schema, and the `daemon-api.md` enum table move with the change; the frozen
  v1 schema and the CLI output goldens are untouched.
- d2bd and d2bd-runtime: the daemon-side producers emit those vocabularies
  through the shared contract types instead of string literals. The read-model
  metadata publishes `PublicReadModelKind` directly (the crate-local duplicate
  enum and the parallel `kind_name` string are gone), the qemu-media runner
  state comes from the pidfd-table liveness helper, and the media registry
  state and autostart posture are typed where they are emitted. The daemon's
  public list/status JSON is byte-identical.
