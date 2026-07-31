### Added

- Added local Network provider primitives for deterministic interface naming, isolated bridge ports, ownership-scoped firewall updates, route readiness, and defense-in-depth IPv6 suppression.

### Security

- Local Network firewall updates preserve sibling and foreign rules, fail closed on ownership-marker conflicts, and reject cross-Zone shared L2 attachments before host mutation.
