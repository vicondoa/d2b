### Added

- Added local Network Provider primitives and hermetic tests for deterministic interface naming, isolated bridge ports, ownership-scoped firewall plans, route readiness, and defense-in-depth IPv6 suppression. The matching broker handlers are live, but the neutral Network effect adapter and production caller are not yet present.

### Security

- The live broker handlers preserve sibling and foreign firewall rules and fail closed on ownership-marker conflicts, while Provider-level cross-Zone and lifecycle behavior remains proven only through hermetic admission tests.
