# Zone resource-plane runtime

`d2bd` owns the lifetime of each configured Zone resource runtime. Startup
derives the Zone set from the trusted host bundle and requests the matching
opaque `zone-store-<zone>` row from `d2b-priv-broker`.

The broker response carries exactly one close-on-exec database descriptor.
`d2bd` consumes that descriptor with `RedbResourceStore::open_owned`, rehydrates
the current store metadata, and constructs the native Resource API. The
ComponentSession router and registrar-owned subject resolver are not yet
registered in `d2bd`, so the public socket remains a lifecycle-authenticated
surface only; it never turns `SO_PEERCRED` into a Resource API subject.
Resource-plane readiness therefore remains fail-closed until the real
ComponentSession endpoint, policy owner, and watch consumer are registered.

Restart reopens the same opaque storage row, validates its immutable identity,
and rehydrates the persisted policy, catalog, configuration, and controller
revision metadata before the readiness barrier. Revision metadata does not
install policy content or establish session authority. Shutdown drains the
daemon-owned runtime and asks the production store to persist its
clean-shutdown marker.
CLI Zone requests use the daemon's authoritative Zone index for routing; the
request's `zoneRef` is checked as a route assertion and is never used to mint
authority. Direct resource mutation verbs remain unsupported. Removal is
owned by the existing authenticated compiled desired-state omission and
finalizer controller, which is not yet wired to this daemon endpoint.
