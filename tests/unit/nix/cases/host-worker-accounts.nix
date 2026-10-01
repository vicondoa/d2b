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
  provider = controllerExecutionRef: {
    type = "Provider";
    spec = {
      artifactId = "fixture-provider";
      config = { inherit controllerExecutionRef; };
    };
  };

  gpuDevice = label: settings: {
    type = "Device";
    metadata.ownerRef = "Guest/acceptance-guest";
    spec = {
      providerRef = "Provider/device-gpu";
      deviceClass = "physical";
      inventory.selector = { busClass = "drm"; inherit label; };
      provider.settings = settings;
    };
  };

  cfg = {
    site = {
      adminUsers = [ ];
      launcherUsers = [ ];
    };
    zones.work.resources = {
      host-system = { type = "Host"; };
      acceptance-guest = { type = "Guest"; };
      volume-local = provider "Host/host-system";
      volume-virtiofs = provider "Host/host-system";
      runtime-cloud-hypervisor = provider "Host/host-system";
      device-tpm = provider "Host/host-system";
      device-gpu = provider "Host/host-system";
      # A Provider that declares no controller execution reference: the
      # compiler projects no controller row for it, so no account is derived.
      unattached-provider = {
        type = "Provider";
        spec.artifactId = "fixture-provider";
      };
      tpm0 = {
        type = "Device";
        metadata.ownerRef = "Guest/acceptance-guest";
        spec = {
          providerRef = "Provider/device-tpm";
          deviceClass = "emulated";
          inventory.selector = { };
          provider.settings = { };
        };
      };
      gpu0 = gpuDevice "fixture-gpu0" { videoSidecar = true; };
      gpu1 = gpuDevice "fixture-gpu1" { renderNodeOnly = true; };
    };
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

  # Every account this Zone's rows run as. `d2b-work-<provider>-controller` is
  # per Provider because one Provider is one signed artifact published under
  # one key; `d2b-work-<device>-{gpu,video}` is per Device and per family
  # because the GPU authority admission refuses a video principal equal to the
  # GPU principal; the serving worker and the managed-identity agent carry
  # their own account rather than their Provider's controller account.
  derivedNames = [
    "d2b-work-tpm0-swtpm"
    "d2b-work-tpm0-swtpm-flush"
    "d2b-work-gpu0-gpu"
    "d2b-work-gpu0-video"
    "d2b-work-gpu1-gpu"
    "d2b-work-controller-device-gpu"
    "d2b-work-controller-device-tpm"
    "d2b-work-controller-runtime-cloud-hypervisor"
    "d2b-work-controller-volume-local"
    "d2b-work-controller-volume-virtiofs"
    "d2b-work-virtiofsd"
  ];

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