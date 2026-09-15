# Drift gate: the Nix privilege-plane copy of broker operations must stay
# aligned with the committed broker catalog across retirements (KTD10, plan
# Verification Contract).
#
# Invariant: every `brokerOperations` name in nixos-modules/privileges-json.nix
# is either a committed broker catalog operation (docs/reference/policy/
# broker-operations.json) OR one of the pre-existing host-activation verbs that
# predate the catalog and that no committed row owns (closed allowlist below).
#
# The check is one-directional and shrinking-safe: retiring a row from BOTH
# documents keeps it green; leaving a retired row behind in the Nix copy (or
# inventing a new one) fails; the allowlist never grows.
{ flakeRoot, pkgs, ... }:

let
  privilegesJson = import (flakeRoot + "/nixos-modules/privileges-json.nix") {
    config = { };
    inherit (pkgs) lib;
    inherit pkgs;
  };
  privilegeOps = map (row: row.operation)
    privilegesJson.config.d2b._bundle.privilegesJson.data.brokerOperations;
  catalog = (builtins.fromJSON (builtins.readFile (flakeRoot + "/docs/reference/policy/broker-operations.json"))).rows;
  catalogOps = map (row: row.operation) catalog;
  # Closed allowlist of pre-plan daemon-internal verbs that live only in the
  # privilege plane (no committed broker catalog row owns them).
  hostActivationVerbs = [
    "BindMountFromHardlinkFarm"
    "MigrateLegacySwtpmState"
    "PrepareStoreView"
    "RunActivation"
    "RunGc"
    "RunHostInstall"
    "RunHostKeyTrust"
    "RunKeysRotate"
    "RunMigrate"
    "RunRotateKnownHost"
    "SetupMountNamespace"
    "StoreVerify"
  ];
  orphans = builtins.filter (op: !(builtins.elem op catalogOps)) privilegeOps;
  unaccounted = builtins.filter (op: !(builtins.elem op hostActivationVerbs)) orphans;
in
{
  cases = {
    "privileges-json-drift/brokerOperations-rows-stay-aligned-with-the-catalog" = {
      expr = builtins.deepSeq unaccounted unaccounted;
      expected = [ ];
      propagateError = true;
    };
  };
}