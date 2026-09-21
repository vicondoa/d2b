# Trusted host contract artifact (`host.json`).
#
# The zone-native (v3) bundle carries the host-owned capability contract the
# legacy host manifest carried, beginning with the NetworkManager unmanaged
# drop-in contract (`networkManager`) that the resolver's empty host model
# deliberately leaves blank. The remaining v3 host facts live in Zone
# resource bundles, so this artifact is emitted from framework defaults and
# stays small. The values are operator-visible in the generated bundle:an
# audit can tell a declared contract from a compiled one. A bundle that omits
# `hostPath` leaves the empty host model in place, whose NetworkManager
# fields are empty strings:the `apply-nm-unmanaged` kernel fails closed on
# the empty file path before any host effect.
{ ... }:

let
  # The default NetworkManager unmanaged drop-in contract: visible in the
  # generated bundle and pinned by the same artifactHashes policy as every
  # other private bundle artifact.

  nmFilePath = "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf";
  nmMatchCriteria = [ "interface-name:d2b-*" ];
  nmReloadBehavior = "atomic-reload";
  nmOwnership = {
    owner = "root";
    group = "d2bd";
    mode = "0640";
    driftPolicy = "preserve";
  };
in
{
  config.d2b._bundle.hostJson = {
    data = {
      schemaVersion = "v3";
      site = {
        allowUnsafeEastWest = false;
      };
      environments = [ ];
      nftables = {
        family = "inet";
        table = "d2b";
        chains = [ ];
        ownershipId = "";
      };
      networkManager = {
        filePath = nmFilePath;
        matchCriteria = nmMatchCriteria;
        reloadBehavior = nmReloadBehavior;
        ownership = nmOwnership;
      };
      hostsFile = {
        startMarker = "# d2b-managed begin";
        endMarker = "# d2b-managed end";
        rule = "";
      };
      kernelModules = [ ];
      fdOwnership = [ ];
      cloudHypervisorCapabilities = [ ];
    };
    installFileName = "host.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}