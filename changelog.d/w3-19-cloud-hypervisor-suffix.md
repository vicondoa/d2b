### Changed

- Cloud Hypervisor guest planning no longer allocates a scratch string per candidate role when inferring a child role from its ResourceRef, removing four small heap allocations from every per-child upgrade and status projection.