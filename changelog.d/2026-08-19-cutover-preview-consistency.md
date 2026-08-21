### Fixed

- Keep host cutover preview digests recovery-free while retaining recovery
  binding in apply requests and consent.
- Secure the one-shot cutover bootstrap descriptor across broker exec without
  allowing it to leak into later runner children.
- Grant lifecycle traversal only on the runtime socket directories while
  keeping cutover state root-only.
- Add configured Admin users to the `d2b` lifecycle group without widening
  realm-only access.
