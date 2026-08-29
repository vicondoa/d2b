### Fixed

- Allocate client-initiated ttrpc correlation IDs from the valid odd-ID
  range, and keep package and manifest digests outside signed Provider
  manifests so artifact verification remains non-self-referential.
