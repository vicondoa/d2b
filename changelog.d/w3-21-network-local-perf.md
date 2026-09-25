### Fixed

- d2b-provider-network-local: host firewall digest rendering now writes
  into a single preallocated buffer instead of allocating one String per
  hex byte.
- d2b-provider-network-local: host network observation now runs the link,
  address, and route `ip` probes concurrently instead of sequentially, and
  parses observed addresses into a reused buffer instead of formatting a
  fresh String per CIDR.