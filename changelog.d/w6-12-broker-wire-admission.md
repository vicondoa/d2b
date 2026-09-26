### Changed

- `BrokerRequestEnvelope` no longer carries the test-only `test_peer_uid`
  member. The broker's harness - the bootstrap probe CLI and the integration
  tests - now sends that override as a `testPeerUid` member beside the
  envelope, and only a `--test-mode` broker unwraps it in front of its strict
  decode, so the wire contract carries no test seam and every other broker
  refuses a frame that carries one. Production frames stop emitting the
  `"testPeerUid": null` member; the broker socket is private to this tree, so
  no consumer outside it observes the shape.
- `AuditExportEntry` now carries exactly one payload,
  `AuditExportEntryPayload::Record { record }` or `::Error { error }`, instead
  of the independent `record` / `error` optional pair, so an entry that
  carries neither or both is unrepresentable and refused at decode. The
  emitted members are unchanged - `sequence` plus exactly one of `record` /
  `error` - so the broker audit page and the public daemon audit page keep
  their JSON, and the `legacy_export_entry_line` renderer keeps its output.

### Added

- `d2b_broker::runtime::{TEST_PEER_UID_FIELD, test_peer_uid_frame}` name the
  harness-only peer-uid override for the probe CLI and the integration tests.
- An admission test pinning that `OpenUnitPidfdRequest` and `StopUnitRequest`
  refuse an unknown member. Re-verifying the audited flatten defect showed the
  container's `deny_unknown_fields` is the guard that refuses it, so no
  admission repair applies and the observed contract is pinned instead.
