### Changed

- Zone bundle lookups no longer re-parse resource-bundle JSON or allocate composite keys per call: zone UID presence/identity, network-spec resolution, and guest setup descriptor / VMM intent lookups now read the bundles parsed once at load and borrow (zone, guest) keys from per-zone nested maps, so intent resolution allocates less on bundles with many zones.
- The nftables and hosts renderers write directly into the output buffer instead of building a temporary String per line, and SHA-256 digests are hex-encoded into a fixed buffer, cutting per-render allocations.
- The root-only chown tamper test is now a documented ignored test instead of silently passing when not root, so the skip is visible in test output.