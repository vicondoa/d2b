### Added

- Added ZoneLink Nix coverage for local and gateway-backed transport
  projections, same-Zone reference validation, and Guest execution placement.

### Security

- Rejected credential material, raw locators, and host fallback placement in
  gateway Provider and Guest settings while keeping relay credentials as
  Guest-scoped ResourceRefs.
