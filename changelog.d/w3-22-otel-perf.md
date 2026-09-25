### Changed

- The emitter socket drain reuses one scratch buffer across datagrams instead of allocating a 64 KiB buffer per datagram.
- Metric frames are no longer re-serialized to JSON for size admission on the wire-boundary paths; the size measured at decode is threaded through, and frame admission reuses it.
- OTEL resource attribute values are scanned case-insensitively in place instead of allocating a lowercase copy per attribute.