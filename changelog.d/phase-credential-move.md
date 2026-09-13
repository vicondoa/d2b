### Changed

- The Credential resource driver now lives in its own `d2b-provider-credential`
  crate together with its spec decoder, its effect port, the session and
  revocation vocabulary its teardown binds, and the driver declaration the
  resource plane registers the type by. The three Credential Providers stay
  the separate realizer crates they already are; the daemon keeps the
  production effect implementation - the Provider reads, the live session
  adapter, and the handoff registry - behind the port. Operator-visible
  behavior is unchanged: the same three Providers are admitted, the same
  per-Provider scope checks apply, the managed-identity agent Process child is
  still minted through the manager before it is spawned, and a delete still
  revokes the lease before anything owned is marked deleting.
- The managed-identity agent's Process child is now a declared `ChildCreation`
  on the Credential declaration, pinned to the minijail Process Provider's own
  exported reference rather than a daemon-side literal, and the Credential
  family's `credential_driver` module is gone from the daemon.
