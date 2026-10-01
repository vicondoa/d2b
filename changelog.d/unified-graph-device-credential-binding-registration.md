---
type: changed
area: providers,daemon
---

### Changed

- **`DeviceBinding` and `CredentialBinding` are served in production.** Each
  serving driver is registered from the family that owns the capability it
  binds - `d2b-provider-device` and `d2b-provider-credential` - so one family's
  registration carries both its own resource type and its binding type, and
  both are built from the same daemon-supplied facet set rather than a second
  convention. `DeviceBindingDriver` resolves the parent `Device` through the
  family's own component resolver and reaches the trusted inventory through the
  facet that `DeviceEffects::new` had been dropping, which is what makes the
  family's capability-vocabulary observation reachable at all.
  `CredentialBindingDriver` revokes through the daemon's real
  `RevokeToken` call and binds the parent row's store uid and generation, the
  provider generation, the live session generation, and the lease rotation
  generation; an unconfirmed revoke withholds cleanup rather than reporting a
  release.

- **`EndpointBinding` and `NetworkBinding` are withdrawn from the converted
  registry until their effects have a daemon-reachable port.** Both drivers
  and both derivations were written, and the endpoint driver's three routed
  verbs reach `exact_endpoint_access`, `grant_exact_endpoint_access`, and
  `revoke_exact_endpoint_access` in the broker. But `BrokerRequest` has no
  variant that carries them and the daemon cannot link the broker, so there is
  no path from a provider effect to those helpers. The types therefore stay
  out of the converted registry - a cataloged type with no registered driver
  refuses plane open - and the drivers are removed rather than retained
  unregistered. Their derivations, which emit the exact committed bytes from a
  source row, are kept.