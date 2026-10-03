---
type: added
area: providers
---

### Added

- **Serving drivers for the four typed binding rows, inside their owning
  source families.** `EndpointBinding`, `DeviceBinding`, `NetworkBinding`, and
  `CredentialBinding` each gained a serving driver in the crate that owns the
  capability it binds - `d2b-provider-endpoint`, `-device`, `-network-local`,
  and `-credential` - with the row contract's strict decoder, the committed
  source decision enforced (a row whose admitted rights, arbitration, or
  realized facets do not cover its relationship is refused terminally), parent
  and consumer resolution through the manager behind a same-Zone owner fence,
  and idempotent release on pre-drain and delete. The privileged half routes to
  the machinery that already performs it: endpoint delivery reaches
  `exact_endpoint_access`, `grant_exact_endpoint_access`, and
  `revoke_exact_endpoint_access`; device and credential reach their families'
  real runtime; network release reaches the broker's persistent-tap removal.

  Two verbs are deliberately unimplemented and refuse with a named reason
  rather than approximating: `NetworkBinding` membership realization
  (`membership-tap-unavailable`, because no per-consumer interface creation
  exists broker-side) and `CredentialBinding` delivery
  (`MintPathUnroutable`, because the admitted evidence it needs exists only
  inside the source's admission).

### Known gaps

- No binding row is yet committed through a driver verb, so no binding is
  created or served in production. The drivers are the serving half of a
  relationship whose producing half is still unwired.
- The endpoint binding's grant, revoke, and observation facets have no
  daemon-side implementation: the helpers exist in the broker, but
  `BrokerRequest` has no variant that reaches them, and the daemon cannot link
  the broker. U18's "modify relevant `live_handlers.rs` ACL/FD helpers" needs a
  wire request to carry them across the boundary.