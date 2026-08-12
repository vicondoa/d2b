### Changed

- Derive daemon admission from accepted Unix peer credentials and the configured lifecycle group.
- Reopen Zone stores with their persisted revision metadata while keeping the
  Resource API fail-closed until registrar-owned ComponentSession routing is
  registered.
- Add typed system-core Host and User handler contracts without publishing
  fabricated readiness.
