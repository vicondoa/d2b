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
  - `d2b-<zone>-virtiofsd` and `d2b-<zone>-mi-agent`: the binding-owned
    serving worker and the managed-identity agent each get their own account
    rather than their Provider's controller account, because each holds
    authority a controller does not - two path trees opened to the serving
    worker's principal, credential material in the agent's hands.

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