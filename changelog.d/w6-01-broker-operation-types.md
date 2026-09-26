### Changed

- The generated broker operation artifacts are now typed instead of carrying
  bare strings and booleans. The profile catalogs (`HOST_OPERATION_CATALOG` /
  `GUEST_OPERATION_CATALOG`) and every committed row's `operation` field are
  typed against a closed `BrokerOperationName` enum emitted by the generator
  (one variant per committed row, with `as_str` keeping the wire spelling);
  profile admission still takes the operation name as a string, so the wire
  boundary is unchanged. Each row's `disposition` is a closed `Disposition`
  enum (callable-read-only, promoted-live, stubbed-unimplemented,
  compile-time-only) instead of a string, and the broker authorization rows
  carry `Destructive::No`/`Destructive::Yes` in place of a bare boolean.

- The serialized `privileges.json` authorization shape spells the
  `destructive` facet as `"no"`/`"yes"` instead of `false`/`true`. The Nix
  emitter that produces the artifact and the v2 JSON schema move with the
  change; the v1 schema stays frozen.