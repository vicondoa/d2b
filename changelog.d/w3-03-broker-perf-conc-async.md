### Changed

- The broker's state-cell store now keeps records in the same nested
  cell -> invocation layout its durable file uses, so partial-key lookups
  (contains, payload, remove, keys, clear) are O(log n) instead of scanning
  every record.
- The initial obs-vsock ACL grant on runner spawn now runs on the broker's
  bounded dispatch pool like the retry path, so a spawn no longer stalls an
  executor worker on the setfacl shellout.
- The broker re-parses the embedded store-view posture contract once per
  process instead of once per posture pass and row, and drops per-call
  allocations on the nftables projection digest, cgroup-open audit records,
  and the invocation-id counter.