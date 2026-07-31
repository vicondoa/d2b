# Configuration-publication cleanup and authorization contract.
{ config, lib, ... }:

let
  cfg = config.d2b;
  lifecycleSubresources = [ "create" "update-spec" "delete" ];

  activationRole = zoneName: {
    type = "Role";
    metadata = {
      name = "activation-nixos";
      zone = zoneName;
    };
    spec.rules = [
      {
        resourceTypes = [ "Credential" ];
        verbs = lifecycleSubresources;
        subresources = [ ];
        resourceNames = [ ];
        zones = [ zoneName ];
        executionRefs = [ ];
        sessionVerbs = [ ];
      }
      {
        resourceTypes = [ "Credential" ];
        verbs = [ "admin-credential" ];
        subresources = lifecycleSubresources;
        resourceNames = [ ];
        zones = [ zoneName ];
        executionRefs = [ ];
        sessionVerbs = [ ];
      }
    ];
  };

  zoneCleanup = lib.mapAttrs
    (zoneName: zone: {
      retainedGenerations = zone.retainedGenerations;
      role = activationRole zoneName;
      ownership = {
        field = "managedBy";
        eligibleValue = "configuration";
        preservedValues = [ "controller" "api" ];
      };
      transition = {
        commitBeforeIntents = true;
        cleanupBlocksActivation = false;
        absentResourceAction = "delete";
        finalizersAreForceCleared = false;
        pendingCondition = "PendingCleanup";
        nameConflictCondition = "Degraded/name-conflict";
      };
    })
    cfg.zones;
in
{
  config.d2b._resourceCompiler = {
    credentialLifecycle = {
      ordinaryVerbs = lifecycleSubresources;
      supplementalVerb = "admin-credential";
      subresources = lifecycleSubresources;
    };
    zones = zoneCleanup;
  };
}
