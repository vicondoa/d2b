### Added

- The broker can now resolve the numeric principal of a committed consumer from
  the consumer reference alone. `BundleResolver::consumer_principal` reads the
  committed `Process` / `EphemeralProcess` row out of the verified Zone resource
  bundle and derives the uid/gid the row runs as from the `<ownerRef>:<rowRef>:<executionRef>`
  triple the row commits, and `d2b-broker`'s `ops::consumer_principal` maps a
  Zone self-resource uid plus a consumer reference onto it. A privileged effect
  can therefore name the consumer's uid without a caller-supplied numerical
  credential ever crossing the wire: no wire field, no committed column, and no
  second identity source. The derivation is the one the launch path already
  applies, so the principal an effect resolves and the principal a launch grants
  address one host account; the host layer already mirrors it in
  `nixos-modules/lib.nix` (`deviceWorkerPrincipalId`) and provisions the matching
  `d2b-<zone>-<device>-<row>` account, so a drift between the two sides fails
  closed rather than silently.

- A claim about a consumer principal is checked, never trusted, matching the
  `security_key_authority_binding` and Device-worker launch-scope fences.
  `ops::consumer_principal::repin_consumer_principal` resolves the principal
  independently and refuses a claim that does not reproduce it, naming both
  principals; a claim that agrees returns the derived principal, so no downstream
  effect acts on the claim's copy either way. An undeclared consumer row, a
  reference that is not a consumer row, and a Zone uid with no verified bundle
  each refuse by their own closed reason rather than borrowing another row's or
  another Zone's number.
