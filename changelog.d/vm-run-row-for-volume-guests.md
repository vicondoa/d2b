### Fixed

- Fixed a zone-native Guest that carries a virtiofs Volume attachment but owns
  no Device declaring no `path:vm-run:<guest>` storage row. The virtiofsd
  serving worker binds its private socket under that Guest's shared per-Guest
  runtime tree, and the daemon reads the tree's posture out of this row before
  it creates anything, so the absent row made every such launch refuse with
  `provider-ticket:serving-socket-dir-mode-unresolved` and the Volume's
  `VolumeBinding` never became ready. The row is now emitted for every zone
  native Guest a virtiofsd serving worker can serve, with the same posture
  (mode `1770`, scope `vm:<guest>`, `directory`) the Device-owner path already
  declared, so both sides of the create race land on one posture. Guests that
  own a Device and carry a virtiofs attachment are deduplicated to a single
  row, a `virtio-blk` attachment declares no runtime tree because no host-side
  worker binds a socket for it, and `path:swtpm-state:<guest>` stays keyed on
  the Device-owner relation alone.
