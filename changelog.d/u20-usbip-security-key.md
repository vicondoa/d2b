### Added

- USBIP and security-key mediation now runs on the admitted resource graph
  instead of on provider-local authority tables. A USB Service *requests* its
  physical backing as one canonical `DeviceBinding` (and its relay's
  `NetworkBinding` and `EndpointBinding`), and a security-key Service requests
  one exclusive hidraw `DeviceBinding` while each Binding requests the
  `EndpointBinding` its in-guest frontend consumes. The `Device` source
  arbitrates those requests; the family realizes only what was admitted, as a
  bounded helper leg over the parent's reservation.

### Changed

- A USB Service no longer arbitrates its own backing. The exclusive/shared
  ceiling, the claim conflict, and the release move out of a provider-local
  table and behind the `Device` source's admission, and the per-Network relay
  becomes a bounded leg of the admitted relationship instead of a second
  claimant. A cross-Zone or stale claim, a leg that reaches another physical
  authority, a leg bound to another helper, and a device that reappears under a
  new authority are all refused before any relay, listener, host bind, or
  firewall rule exists.
- The security-key lease no longer takes the Host physical-device authority
  itself on the converted path: the hidraw open is driven by the admitted
  `Device` relationship plus the leg bound to the Host relay, and a ceremony is
  gated on the Guest's own admitted `EndpointBinding` rather than on a
  configured VM-id list. Relay shutdown and Guest Endpoint closure now precede
  the source release, and a relay that would not stop keeps the reservation
  held.

### Removed

- Nothing is removed in this change. The pre-graph USBIP claim table, the
  security-key Core-admission lease path and its physical-claim effect, and the
  relay's configured VM-id access list stay in place for the not-yet-cutover
  production entry point and are queued for deletion with the rest of the old
  wiring.
