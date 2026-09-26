### Changed

- d2b-clipd now shares clipboard payload maps by reference instead of
  copying every byte when a host selection is recorded, a history entry is
  materialized, or a bridge copy is published, so large pastes no longer
  pay a full copy per path.
- The niri IPC line reader now reads in 4 KiB chunks instead of one byte
  per read syscall, keeping the same maximum-line bound.
- The descriptor permit pool and helper-thread counters now use relaxed
  atomic ordering; the counts publish no data, so the weaker ordering is
  sufficient and cheaper.

### Fixed

- The clipboard history now has unit coverage for its entry-count and
  byte-quota eviction order, materialization owner and TTL rejections, and
  entry expiry reporting.
- The controller's display-dependency admission gates now have unit tests
  covering the accepted route shape and each rejected shape, including the
  evidence class, locality, subject type, and generation checks.
- The picker-cancel transition and the niri window-closed and
  workspace-activated cache paths are now covered by unit tests.