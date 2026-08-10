# Zone resource-plane runtime

`d2bd` owns the lifetime of each configured Zone resource runtime. Startup
derives the Zone set from the trusted host bundle and requests the matching
opaque `zone-store-<zone>` row from `d2b-priv-broker`.

The broker response carries exactly one close-on-exec database descriptor.
`d2bd` consumes that descriptor with `RedbResourceStore::open_owned`, creates
the native Resource API and fixed core-controller process coordinator, and
publishes the runtime only after store, API, local-session, Provider, and core
readiness succeeds. A missing bundle, broker refusal, descriptor mismatch, or
store identity failure leaves the Zone unavailable; the daemon does not fall
back to the legacy resource path.

Restart reopens the same opaque storage row and validates the persisted store
identity before readiness. Shutdown drains the daemon-owned runtime and asks
the production store to persist its clean-shutdown marker. CLI Zone requests
use the daemon's authoritative Zone index for routing; the request's `zoneRef`
is checked as a route assertion and is never used to mint authority.
