---
type: fixed
area: contracts,daemon,providers
---

### Fixed

- **One binding row, one encoding.** A committed `VolumeBinding` row is now the
  row contract (`volumeRef`, `executionRef`, `view`, `access`, `presentation`,
  `slot`, `source`) rather than the consumer's request bytes. The manager's
  relation index and the registered serving driver now read the same bytes;
  previously each decoded a different shape of the same relationship, so a row
  the producer committed was refused by the driver that was supposed to serve
  it. The four sibling binding rows already committed this shape, so Volume was
  the outlier rather than the convention.

- **The row carries the presentation, not a bare path.** A block-device
  attachment has no mount destination, and the old shape had nowhere to put one.
  It is now committed with its `deviceSlot` and refused at realization with the
  stable code `presentation-unsupported` rather than being served at a
  destination invented for it.

- **A committed Volume row is produced and served.** The source family derives
  the binding row from its own admitted relationship, commits it, and retires it
  when the relationship no longer exists; the registered serving driver decodes
  the same bytes, reconciles them, and derives the worker and endpoint children.
  `d2b-provider-volume-binding/tests/volume_binding_chain.rs` drives that whole
  chain through the real descriptors and both readers.