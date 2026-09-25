### Fixed

- `d2bd` no longer computes a discarded SHA-256 operation digest plus a
  formatted operation id on every Cloud Hypervisor spec/status update and
  child deletion: the dead canonical-digest chain is gone from the
  reconcile hot path (RS-0800).
- Resource identity projections are pre-sized to their field count
  (twelve required+optional identity columns) and audio status queries to
  their VM set, bounding a refused call's allocations on frequently-
  repeated provider relists (RS-0801, RS-0803).
- The forward rendezvous sizes frame read buffers from the length-prefixed
  datagram instead of the one-megabyte ceiling, so a drain of refused
  frames no longer allocates the ceiling per read (RS-0802).