{ config, lib, ... }:

let
  cfg = config.d2b;
  allocatorRows = cfg._realmAllocatorRows;
  childRealms = lib.sortOn (row: row.realmPath) (cfg._realmAccess.children or [ ]);
  childRealmIds = map (row: row.realmId) childRealms;
  allocatorStateDir = "${toString cfg.site.stateDir}/allocator";
  allocatorRunDir = "/run/d2b/allocator";
  allocatorRootSocket = "${allocatorRunDir}/local-root.sock";

  declaredRealm = row: cfg.realms.${cfg._index.realms.byId.${row.realmId}.realmName};
  indexRealm = row: cfg._index.realms.byId.${row.realmId};

  realmMetadata =
    row:
    let
      realm = indexRealm row;
      declared = declaredRealm row;
    in
    {
      inherit (realm)
        realmName
        realmId
        realmPath
        placement
        ;
      enabled = true;
      hostMutation = declared.broker.hostMutation;
      placementProvider = declared.placementProvider;
      providerSpecificPlacement = declared.providerSpecificPlacement;
      providerKeys = lib.sort lib.lessThan (lib.attrNames declared.providers);
      envNames = [ ];
    };

  pathPartition = row: {
    realmPath = row.realmPath;
    stateDir = row.resources.state.path;
    runDir = row.resources.runtime.path;
    auditDir = row.resources.audit.path;
    publicSocket = row.resources.publicSocket.path;
    brokerSocket = row.resources.brokerSocket.path;
  };

  resourceRequest = row: {
    inherit (row)
      realmPath
      resourceId
      kind
      share
      acquisitionOrder
      source
      ;
  };

  providerPlacements =
    map (
      provider:
      {
        realmPath = cfg._index.realms.byId.${provider.realmId}.realmPath;
        inherit (provider)
          providerName
          providerId
          enabled
          placement
          capabilityRefs
          configRef
          ;
        kind = provider.providerType;
      }
    ) (
      lib.sortOn (provider: "${provider.realmId}/${provider.providerId}") (
        lib.filter (
          provider: builtins.elem provider.realmId childRealmIds
        ) cfg._index.providers.enabledList
      )
    );

  data = {
    schemaVersion = "v2";
    allocator = {
      enabled = childRealms != [ ];
      runtimeState = "metadata-only";
      rootSocket = allocatorRootSocket;
      stateDir = allocatorStateDir;
      leaseLedger = "${allocatorStateDir}/leases.jsonl";
      auditDir = "${allocatorStateDir}/audit";
      runtime = {
        spawnsService = false;
        socketActivated = false;
        serviceName = null;
      };
    };
    realms = map realmMetadata childRealms;
    resourceRequests = map resourceRequest allocatorRows.resources;
    pathPartitions = map pathPartition childRealms;
    inherit providerPlacements;
    envBridge = [ ];
    invariants = {
      noRuntimeAllocatorService = true;
      preservesEnvRuntimeSourceOfTruth = true;
      privateMetadataOnly = true;
    };
  };
in
{
  imports = [ ./realm-allocator-rows.nix ];

  config = {
    assertions = [
      {
        assertion = builtins.substring 0 9 allocatorRootSocket == "/run/d2b/";
        message = "d2b allocator.json rootSocket must remain under /run/d2b";
      }
      {
        assertion = builtins.stringLength allocatorRootSocket <= 107;
        message = "d2b allocator.json rootSocket must fit Linux AF_UNIX sockaddr_un.sun_path";
      }
      {
        assertion =
          builtins.length data.resourceRequests
          == builtins.length (lib.unique (map (row: row.resourceId) data.resourceRequests));
        message = "d2b allocator.json resource request IDs must be unique";
      }
    ];

    d2b._bundle.allocatorJson = {
      inherit data;
      installFileName = "allocator.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
