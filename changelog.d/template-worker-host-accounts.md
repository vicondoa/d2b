### Added

- The host layer now provisions a host account for every template-bound
  Process row class, so the workers d2b launches resolve a real principal
  instead of being refused. `nixos-modules/lib.nix`
  (`templateWorkerAccounts`) derives one account per row class per Zone from
  the same trusted Zone rows the resource compiler binds, and the generated
  `nixos-modules/host-users.nix` materializes them alongside the accounts the
  committed principal allocation names. The accounts are the same system
  accounts the Device TPM family already used - own uid and gid, own primary
  group, `isSystemUser`, no supplementary groups - so a launcher keeps the
  identity, the group and the ACL grants it had.

  The row class is the privilege boundary each account carries, not
  convenience:

  - `d2b-<zone>-<device>-gpu` and `d2b-<zone>-<device>-video`: one account per
    Device and per family. The GPU authority admission refuses a video
    principal equal to the GPU principal, so a Device's two families are two
    identities, and each Device is the exclusivity unit its worker runs as.
  - `d2b-<zone>-controller-<provider>`: one account per owning Provider, with
    the Zone in the name. A Provider's controller rows are hashed per
    component, so the row name is not a boundary to separate on; one Provider
    is one signed artifact published under one key, and that is the trust
    boundary the account follows.
  - `d2b-<zone>-virtiofsd`: the binding-owned serving worker gets its own
    account rather than its Provider's controller account, because two path
    trees are opened to its principal. The credential agent a `Credential`
    controller adopts is deliberately not here: the shared crate may not name
    it under the Provider family-knowledge rule, so an account only the Nix
    side held would resolve nothing, and those rows stay refused.
</input>

  The numeric identity is the name-derived id every other named principal in
  the module tree uses, so an account keeps its ids for as long as it keeps its
  name. The Device TPM family keeps the ids it already holds rather than
  re-deriving them, because those ids predate this scheme and changing them
  would renumber a live principal. The generated module now asserts that no two
  provisioned accounts collide on a name or on a uid, and that no provisioned
  name is longer than the 63 bytes the account database carries, instead of
  assuming it.

  `packages/d2b-core`'s `template_account` (`bundle_resolver.rs`) composes the
  same names from the binding's own owner reference, declared row name, and
  template - the facts the compiler emits from those same Zone rows - so both
  sides are two evaluators of one rule over one input. A row outside that
  vocabulary, and a row whose account the host does not hold, still refuse.

### Fixed

- A host-integration guest that declared a Provider controller template, the
  binding-owned virtiofsd serving worker, or a Device GPU worker logged
  `template-principal-unprovisioned` for each of those rows and minted no
  launch intent for them, so the controllers, the serving worker, and the GPU
  workers of a Zone never ran. Those rows now resolve a provisioned account.

- Two of those accounts could not be provisioned at all, so the guest failed
  to evaluate rather than refusing the rows it names. A Provider controller
  account composes to `d2b-<zone>-controller-<provider>`, which for the
  Providers a host declares (`volume-local`, `volume-virtiofs`,
  `runtime-cloud-hypervisor`) is past the 31 bytes the host account database
  carries - NixOS's own user and group options refuse 32 or more - and the
  guest aborted with `Group name 'd2b-work-controller-volume-local' is longer
  than 31 characters which is not allowed!`. Every derived name is bounded now:
  one that fits is used exactly as composed, and one that does not keeps a
  readable prefix of its row-class token and carries eight hex digits of the
  SHA-256 over the whole name, so two row classes that overflow together stay
  two accounts. A Zone name long enough that no prefix fits has no account on
  either side and is refused.

  `packages/d2b-provider-device-tpm` composed the same two TPM worker
  accounts a third time, in its state Volume's granted principals and in its
  worker's own process principal, with no bound at all - so a Device name
  long enough to overflow the account would have been named for an account
  only that crate held while the host provisioned the bounded one. Both
  compose through `d2b-core`'s `bounded_account_name` now, which is the same
  composition `nixos-modules/lib.nix` and `template_account` use.

  `template_account` also ended the composition on the Device families. A
  controller row owned by a Device Provider - which signs a controller
  artifact as well as its worker artifact - matched that arm, found a row name
  outside the worker vocabulary, and returned nothing, leaving the Provider
  controller rule below it unreachable for exactly the Providers that reach
  it. A row name no Device family claims now falls through to the remaining
  classes instead.

  The row classes and the account each composes to are one committed table
  (`tests/unit/nix/cases/host-worker-accounts.json`) that both sides read: the
  `host-worker-accounts` Nix case evaluates the module over it, and
  `every_provisioned_row_class_composes_its_host_account_name` in
  `packages/d2b-core` runs `template_account` over it. Neither side carries its
  own copy of the names, so the two cannot agree by accident of two careful
  ones, and cannot drift apart without one of them failing.