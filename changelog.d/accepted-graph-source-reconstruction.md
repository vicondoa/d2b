---
type: fixed
area: contracts,daemon,broker
---

### Fixed

- **A committed binding row now rebuilds its accepted source.** `AcceptedGraph`
  previously populated its binding-source map only through a builder with no
  production caller, so every leg-bearing binding admission saw an absence and
  refused with `SourcePolicyRefused` - correctly, for a fact that was in fact
  committed. Each typed binding row now carries the source provider's accepted
  decision (the rights it admitted, the arbitration it chose, and the
  realization facets it declared), and a boundary rebuilds the accepted source
  from the row through the family's own contract. A row whose identity the
  projection did not resolve still contributes nothing, because absence is a
  refusal and not an admission.
- **`device_inventory` decodes a committed `Device` row again.** The committed
  row carries the envelope's `providerRef` alongside the spec facets and
  `DeviceSpec` denies unknown fields, so the decode failed for every Device row
  and propagated `InvalidResource` into GPU reconcile. The envelope fields are
  now stripped before decoding, matching the network path.