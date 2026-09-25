### Changed

- The resource API now compares a desired update envelope against the stored row before copying it, so a byte-identical no-op update no longer pays for a full canonical-resource clone; list cursors and authorization-facts compilation also avoid per-byte and per-row allocations.
- Error responses from the resource RPC methods are now rendered through a single generic helper instead of thirteen generated functions; the wire output is unchanged.
- The list snapshot-revision test tolerates a second boundary crossing between the served snapshot and the clock read, so it no longer flakes at epoch-second boundaries.