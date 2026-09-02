### Changed

- Controller-owned Guest lifecycle now resolves Gateway Guests and guest-control Endpoints from committed Zone resources and establishes sessions by immutable Guest identity.
- Guest lifecycle and activation paths preserve exact Zone, Guest, Endpoint, Provider, generation, and revision fences across reconnect and restart.

### Security

- Lifecycle identity mismatches are isolated instead of superseding another Guest generation, and Gateway composition retains relay credentials and Provider effects inside the Gateway Guest.
- Host shutdown remains a distinct stop-only capability.
