# The host accounts a template-bound Process row class runs as.
#
# `packages/d2b-core`'s `template_account` (`bundle_resolver.rs`) composes
# each of these names from a binding's own owner reference, declared row name,
# and template, and refuses a row whose host holds no such account. These cases
# pin the Nix side of that same rule - `nixos-modules/lib.nix`
# (`templateWorkerAccounts`), materialized by the generated
# `nixos-modules/host-users.nix` - against the Zone rows the resource compiler
# binds, so a host that declares a Zone's Providers and Devices resolves those
# rows and one that drifts from the vocabulary does not.
#
# The Zone and the row classes are read from `host-worker-accounts.json`, the
# one committed table both sides are read through: the crate's
# `every_provisioned_row_class_composes_its_host_account_name` test runs
# `template_account` over the same rows and asserts the same names. Neither
# list is written out twice, so the two sides cannot agree by accident of two
# careful copies - a name composed differently on one side fails there.
#
# The module is called as the plain function it is rather than through
# `mkEval`: it depends on `config.d2b` and nothing else in the NixOS module
# system, and a case that reached through a full module evaluation would drag
# the host-tool package set in for a value assertion about account names.
#
# The account shape is asserted as well as the name: a launcher keeps the
# identity, the group and the empty supplementary set the Device TPM accounts
# already had, and no account is provisioned for a Provider or Device the
# compiler would project no row for.
{ lib, d2bLib, ... }:

let
  table = builtins.fromJSON (builtins.readFile ./host-worker-accounts.json);
  zoneName = table.zone;

  cfg = {
    site = {
      adminUsers = [ ];
      launcherUsers = [ ];
    };
    zones.${zoneName}.resources = table.resources;
  };

  module = import ../../../../nixos-modules/host-users.nix {
    inherit lib;
    config = { d2b = cfg; };
  };

  # `users.users` is a `lib.mkMerge`, so the merged set is folded out here
  # rather than read as one attrset. The key the merge carries is read
  # through both spellings so the case does not pin the nixpkgs revision it
  # runs against.
  users = lib.foldl' (acc: element: acc // element) { }
    (module.users.users.elements or module.users.users.contents);
  groups = module.users.groups;
  account = name: users.${name} or null;

  # Every account the table's rows run as, in the order the table lists them.
  # A row class either side refuses carries no name here at all, so an account
  # provisioned for one would be an account no row resolves through.
  derivedNames = map (row: row.account) (builtins.filter (row: row.account != null) table.rows);

  # The Zone resource-store owner comes from the committed principal
  # allocation rather than from these Zone rows, and is provisioned
  # alongside them.
  expectedProvisioned = lib.sort lib.lessThan
    (derivedNames ++ [ "d2b-zonert" ]);

  provisioned = lib.filter
    (name: lib.hasPrefix "d2b-" name && (account name) != null)
    (lib.sort lib.lessThan (builtins.attrNames users));

  nonTpmNames = lib.filter (name: !lib.hasInfix "-swtpm" name) derivedNames;
in
{
  "host-worker-accounts/derived-for-every-declared-row-class" = {
    expr = provisioned;
    expected = expectedProvisioned;
  };

  "host-worker-accounts/no-account-for-a-provider-without-a-controller-target" = {
    expr = account "d2b-work-controller-unattached-provider" == null;
    expected = true;
  };

  "host-worker-accounts/no-video-account-without-a-configured-sidecar" = {
    expr = account "d2b-work-gpu1-video" == null;
    expected = true;
  };

  # The host account database carries a name of at most 31 bytes - NixOS's own
  # user and group options refuse 32 or more - and a row class whose composed
  # name would be longer is bounded to one that fits rather than left to fail
  # the guest's evaluation. The readable prefix of the row-class token
  # survives, and the digest of the whole name keeps two row classes that
  # overflow together two accounts.
  "host-worker-accounts/every-derived-account-fits-the-account-database-bound" = {
    expr = builtins.all (name: builtins.stringLength name <= d2bLib.accountNameLimit) expectedProvisioned;
    expected = true;
  };

  "host-worker-accounts/a-row-class-past-the-bound-is-shortened-not-refused" = {
    expr = builtins.elem "d2b-work-controller-vo-632196ce" provisioned
      && builtins.elem "d2b-work-controller-vo-4d448bd4" provisioned
      && builtins.elem "d2b-work-controller-ru-f0691444" provisioned;
    expected = true;
  };

  # The account shape is the Device TPM family's shape: a system account with
  # its own uid, its own primary group, and no supplementary groups.
  "host-worker-accounts/every-derived-account-is-a-system-account" = {
    expr = builtins.all
      (name:
        let declared = account name; in
        declared.isSystemUser
        && declared.group == name
        && (declared.extraGroups or [ ]) == [ ]
        && !(declared ? home))
      derivedNames;
    expected = true;
  };

  # Every derived account owns its group too, so the primary group a launch
  # runs with is the account's own rather than a shared one.
  "host-worker-accounts/every-derived-account-owns-its-group" = {
    expr = builtins.all
      (name: groups.${name}.gid == (account name).uid)
      derivedNames;
    expected = true;
  };

  # The Device TPM family keeps the binding-triple ids its accounts already
  # hold; every other derived account takes the name-derived id the module
  # tree uses for a named principal. Neither is assumed: both are read back
  # from the identities themselves.
  "host-worker-accounts/tpm-family-keeps-its-binding-triple-ids" = {
    expr = (account "d2b-work-tpm0-swtpm").uid
      == d2bLib.deviceWorkerPrincipalId
        "Provider/device-tpm" "Process/swtpm-tpm0" "Host/host-system"
      && (account "d2b-work-tpm0-swtpm-flush").uid
        == d2bLib.deviceWorkerPrincipalId
          "Provider/device-tpm" "EphemeralProcess/swtpm-flush-tpm0"
          "Host/host-system";
    expected = true;
  };

  "host-worker-accounts/other-classes-take-the-name-derived-id" = {
    expr = builtins.all
      (name: (account name).uid == d2bLib.stablePrincipalId name)
      nonTpmNames;
    expected = true;
  };

  # Two row classes of one Device never share an identity: the GPU worker's
  # account and its video sidecar's are distinct.
  "host-worker-accounts/gpu-and-video-are-separate-accounts" = {
    expr = (account "d2b-work-gpu0-gpu").uid != (account "d2b-work-gpu0-video").uid;
    expected = true;
  };

  # The allocated identities and the derived per-Zone identities are two
  # independent derivations; the module asserts they do not collide, and the
  # identities are distinct over every provisioned account here as a value.
  "host-worker-accounts/identities-are-distinct" = {
    expr = builtins.length
      (lib.unique (map (name: toString (account name).uid) expectedProvisioned))
      == builtins.length expectedProvisioned;
    expected = true;
  };
}