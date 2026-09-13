### Changed

- Convert the Credential resource family to the v3 `ResourceDriver` contract
  (U12): `packages/d2bd/src/credential_driver.rs` carries the secret-service,
  entra, and managed-identity controller behavior as one type-level driver -
  validate folds `validate_spec` plus the per-kind scope checks, recover
  adopts a serving managed-identity agent Process, reconcile mints and
  ensures `Process/mi-agent-<credential>` through the manager (the agent is a
  Process resource; the driver never spawns it, KTD13), and delete preserves
  the revocation-first ordering: the provider RevokeToken call must confirm
  (or already have confirmed) before any owned Process child is marked
  deleting, and a missing or no-longer-current session generation fails the
  pass closed (R28).
- Delete the runner-backed `CredentialResourceReconciler`,
  `credential_controller_descriptor`, its finalizer/mutation helpers, and the
  old reconciler test set from `packages/d2bd/src/credential_resource_runtime.rs`.
  What remains is the provider-side session surface the conversion still
  needs: the typed `CredentialSession` registry, the exact non-secret
  revocation request that binds one session generation, and the same-Zone
  scoped credential client. Status is now the driver's in-memory typed
  projection (R11), carrying the old phase/outcome classification
  (`credential-provider-unavailable`, `credential-agent-pending`,
  `credential-agent-unavailable`, `credential-agent-draining`,
  `credential-lease-revoked`, `credential-revocation-uncertain`).

### Notes

- The old revocation gate read the lease facts from the Credential's durable
  status (`/status/resource/credential/...`), which the rewrite deletes.
  `CredentialDriverEffects::lease_facts` supplies them instead; `None`
  reproduces the old "no lease state" case (skip revocation) exactly. Until
  a production lease-fact source is wired, that branch stays dormant, as it
  is today in-tree (nothing writes `leaseState`).
