### Added

- Driver-visible live resource read on the v3 resource-runtime contract
  (KTD3): `ManagerEndpoint::view` / `ResourceContext::get_view` return the
  manager's in-memory runtime view for one key - the committed row plus the
  status its actor last published (R11: memory only, zero store reads) and
  the row generation that status was published for.
- `ResourceView::observed_status`: the observed status of the exact row
  generation, so a `Ready` carried over from an older generation cannot
  pass as current readiness.

### Changed

- `ManagerCall` gains `GetView`; `ChannelManagerEndpoint`,
  `ManagerActorEndpoint`, and the in-tree driver test doubles implement the
  new trait method. Production reads are served by
  `ResourceManagerMsg::Get`, which already answered `ResourceView` - no
  second status store and no store access on the path (KTD12).

### Fixed

- Drivers can prove a child or dependency Ready: readiness is
  `get_view(key).observed_status() == Some(Ready)`, `Ok(None)` means absent
  (no row), and a row with nothing published means unknown - never "not
  ready" and never a fabricated `Pending`. This is the surface the volume
  leg recorded for KTD6 and the U8 child-set readiness lanes were blocked
  on; `WatchCondition::Ready` remains the readiness wake-up.
