### Changed

- A template-bound Process row's principal is now resolved from the host
  account the row runs as, through the account database, instead of being minted
  from a hash of the row's `<ownerRef>:<rowRef>:<executionRef>` triple folded
  into the `50_000..16_777_215` band. The number a launch runs as and the number
  a privileged effect grants an ACL entry to are the ids one real account
  holds, so the launch identity, the ACL entry and the account are one host
  identity rather than three agreeing derivations.

  A principal that is not a real host account is refused with a named reason
  (`template-principal-unprovisioned`, `template-principal-account-absent`, or
  `template-principal-account-unreachable`) instead of being answered with a
  number. That is the same rule the state-Volume layout resolver already applies
  to a layout principal: an earlier revision there fell back to a name-derived
  id, which would have turned a loud refusal into a silent permission grant for
  a uid no process holds, and it was reverted. This brings the launch and
  broker-grant paths onto the rule that was already shipped rather than leaving
  two answers to the same question depending on which layer asks.

  The host layer provisions an account for exactly one template row class today:
  the Device TPM worker rows, as `d2b-<zone>-<device>-swtpm` and its `-flush`
  sibling, derived per Zone in `nixos-modules/lib.nix` (`deviceTpmPrincipals`)
  and materialized by `host-users.nix`. **Rows outside that set are refused
  until the host provisions them** - every Provider controller template, the
  binding-owned virtiofsd serving worker, and the GPU and video Device workers.
  A refused row mints no launch intent and produces no ACL named-user entry, so
  the launch is refused rather than run as a uid nothing holds, and the broker
  answers its existing closed `RowUnresolved` rather than inventing a uid. The
  bundle load reports each refusal with its Zone and row, so a host that has not
  provisioned an account learns which one it needs from the daemon's own log
  rather than from a launch that quietly never appears.

### Removed

- Unit coverage that the change makes unobservable, listed here so the gap is
  visible rather than silent. The unit-test host provisions no `d2b-*` account,
  so no row class can mint an intent there:
  - the Device-worker storage-grant matrix - a declared read-write binding
    resolving to its host path, the read-only and undeclared cases beside it, a
    controller-created child Volume resolving against its declaring Device, a
    foreign Device's Volume refusing the worker, an unresolvable grant refusing
    the worker, an absent storage contract refusing the worker, and the granted
    path matching the broker's own state-directory fence;
  - the private-metadata pinning of a Provider controller intent - its role,
    cgroup subtree, namespaces and binary path, and the lookup's discrimination
    of a wrong owner, a wrong execution target, a wrong template and a wrong row
    kind, which now collapse onto one refusal point;
  - the Device-worker lookup fence - the declared template, the row type, an
    undeclared row, and a wrong execution target - and the supervisor's
    declared-row resolution identity;
  - the broker's consumer-principal agreement with the launch path, the
    `ClaimMismatch` branch of the caller-claim fence, and the per-Zone
    two-principal discrimination;
  - the daemon's static Provider controller ticket: the one inherited fd a
    controller bootstraps with, the process, template, Zone identity, owner and
    runtime scope the ticket pins, and the lookup's discrimination of a wrong
    owner. A ticket is assembled only over a resolved launch policy, and a
    controller row mints no policy without an account, so there is no ticket
    here to inspect and the wrong-owner case no longer separates from the
    right-owner one;
  - the daemon's Device-worker ticket: the declared template, process, owner and
    execution target, the row name as the launch identity, and the refusal of a
    row the Device declares no binding for - the same collapse onto one
    refusal point;
  - the ephemeral one-shot Device worker's ticket carrying the composed TPM
    flush argv, and an untyped row keeping the bare template argv. The argv is
    still proven where it is composed, and what is lost is that the one-shot
    launch path attaches it to the ticket;
  - and, in the exact-endpoint delivery lane, the serving half of a committed
    `Endpoint` relationship: the grant landing on the exact pinned inode, the
    kernel's effective rights read back for the consumer's own ids, the sibling
    and alternate sockets carrying nothing for that principal, a standing grant
    observed rather than re-applied, and a producer's rebound socket reported
    as replaced. That lane still proves the derivation, the canonical bytes the
    driver commits, the idempotent repeat, the withdrawal that retires the row,
    and the named refusal the serving pass now publishes on every pass.

  What replaces each is a typed refusal that names the row and the account.
  The positive paths - a row running as its provisioned account's ids, and the
  exact-endpoint ACL grant landing on the admitted inode and no other - belong
  to the host lane, and are only reachable once the host provisions an account
  for the row classes above. That provisioning is the missing half of this
  change and is the prerequisite for restoring this coverage.
