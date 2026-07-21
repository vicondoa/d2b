{ config, lib, ... }:

let
  plan = import ./realm-network-rows.nix {
    inherit config lib;
  };
in
{
  options.d2b._realmNetwork = lib.mkOption {
    type = lib.types.attrs;
    default = { };
    internal = true;
    visible = false;
    description = "Realm-scoped network resource and provider rows.";
  };

  config = {
    d2b._realmNetwork = {
      schemaVersion = plan.schemaVersion;
      inherit (plan) realms allocatorRequests;
      invariants = {
        allocatorOwnsGlobalClaims = true;
        childBrokerUsesLeases = true;
        defaultEastWestIsolation = true;
        foreignOwnershipFailsClosed = true;
        noLegacyEnvOwnership = true;
        noGatewayVmSynthesis = true;
      };
    };

    d2b._bundle.allocatorJson.data.resourceRequests =
      lib.mkAfter plan.allocatorRequests;

    assertions = plan.assertions;
  };
}
