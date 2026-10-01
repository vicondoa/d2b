---
type: changed
area: contracts,providers,daemon
---

### Changed

- **Corrected binding ownership.** The plan does not give each typed binding
  its own serving provider crate. U14, U17, U18, and U37 each put the binding
  driver inside the owning **source** family crate - adding `src/binding.rs`
  there - with the privileged half of the realization routed through broker
  operations. U18 names the integration point directly: the existing
  `live_handlers.rs` ACL and file-descriptor helpers, proved by
  `packages/d2b-broker/tests/endpoint_delivery.rs`. Four separate
  `d2b-provider-*-binding` crates had been written against a daemon-side effect
  seam that does not exist; they are removed rather than left as a serving
  claim nothing can honour.

  What remains is the part that was correct: the five typed binding row
  contracts, the source provider's accepted decision carried on each row, the
  per-family derivations that emit the exact committed bytes from a source row,
  and the accepted-graph reconstruction that lets a boundary rebuild the
  accepted source from a committed row. The four binding row types leave the
  converted-type registry until each source family serves its own driver, and
  `d2b-provider-volume-binding` - the one serving provider the plan does name -
  is untouched.