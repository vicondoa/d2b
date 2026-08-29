### Added

- Azure Relay transport sessions now use concrete Guest-local WebSocket
  connectors and credential leases bound to the exact ZoneLink, session, and
  reconnect generation.

### Security

- Gateway Guest Relay credentials and sealing material now use zeroizing
  storage, transactional generation fencing, exact acquire/revoke tracking,
  bounded lease cardinality, and redacted transport errors without exposing
  secret bytes to diagnostics.
