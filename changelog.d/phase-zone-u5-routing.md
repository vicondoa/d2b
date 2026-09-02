### Changed

- ZoneLink routing and Provider forwarding now consume runtime-issued sealed route admissions instead of caller-populated authorization, connectivity, capability, and time claims.
- Production Zone composition now installs child-local Gateway Guest route state, while enrolled Relay connections carry only protected ComponentSession packets.
- Gateway-backed Zones are classified before lifecycle, exec, reconcile, shell, or Resource API dispatch; unsupported and refused paths fail closed instead of falling through to host-local effects.
- Allocator topology artifacts now require explicit compiler topology input, and host acceptance boots an isolated Gateway Guest canary credential, requiring the Guest runtime's non-secret open marker before proving exact secret non-materialization.

### Security

- Route admission verifies exact ZoneLink identity, topology edge, controller and reconnect generations, Zone identities, immutable operation, capability, policy revision, session profile, and bounded expiry at current route use, with single-use invalidation after consumption.
- Host d2bd no longer loads legacy gateway configuration or credential paths; Relay credential custody remains inside the Gateway Guest.
