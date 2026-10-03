### Added

- **The `Device` source now derives the `DeviceBinding` rows its own row
  implies.** A binding relationship is a committed row of its own type, and the
  Device source is what produces it: `d2b_provider_device::binding` now turns
  each admitted device claim into exactly one `DeviceBindingSpec` row, named
  from the KTD3 key and carrying the source's accepted decision, so a reader
  and the graph cannot disagree about what was admitted. The row names the
  consumer, the stable slot, the named capability, and the requested claim
  exactly as the consumer authored them, and the decision is read off the
  admission rather than recomputed, so a row never describes a different grant
  than the one that was evaluated. A Device row that admits no claim implies no
  row at all, and a claim the family's declared capability vocabulary does not
  back - or one the trusted inventory no longer backs - is refused or retires
  rather than committed as a declaration no family realizes.
