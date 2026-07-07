{ config, lib, ... }:

let
  cfg = config.d2b;

  sortNames = names: lib.sort lib.lessThan names;
  sortedAttrNames = attrs: sortNames (lib.attrNames attrs);
  sortedMapAttrsToList = f: attrs:
    map (name: f name attrs.${name}) (sortedAttrNames attrs);

  realmRows = cfg._index.realms.enabledList;
  envMeta = cfg._index.envMeta;

  allocatorStateDir = "${toString cfg.site.stateDir}/allocator";
  allocatorRunDir = "/run/d2b/allocator";
  allocatorRootSocket = "${allocatorRunDir}/local-root.sock";

  providerPlacement = realm: providerName: provider: {
    realmPath = realm.path;
    inherit providerName;
    providerId = provider.id;
    enabled = provider.enabled;
    kind = provider.kind;
    placement = provider.placement;
    capabilityRefs = sortNames (lib.unique provider.capabilityRefs);
    configRef = provider.configRef;
  };

  providerPlacements = lib.flatten (map
    (realm: sortedMapAttrsToList (providerPlacement realm) realm.providers)
    realmRows);

  pathPartition = realm: {
    realmPath = realm.path;
    inherit (realm.paths) stateDir runDir auditDir publicSocket brokerSocket;
  };

  envBridge = realm: envName:
    let
      declared = builtins.hasAttr envName cfg.envs;
      enabled = builtins.hasAttr envName cfg._index.enabledEnvs;
      meta = if builtins.hasAttr envName envMeta then envMeta.${envName} else null;
    in
    {
      realmPath = realm.path;
      inherit envName declared enabled;
      mode = realm.network.mode;
      netVm = if declared then cfg.envs.${envName}.netName else null;
      lanBridge = if meta != null then meta.lanBridge else null;
      uplinkBridge = if meta != null then meta.uplinkBridge else null;
    };

  envBridgeRows = lib.flatten (map
    (realm: map (envBridge realm) realm.network.envNames)
    realmRows);

  acquisition = phase: ordinal: { inherit phase ordinal; };
  source = kind: refName: { inherit kind; refName = refName; };
  request = realm: resourceId: kind: share: phase: ordinal: sourceKind: refName: {
    realmPath = realm.path;
    inherit resourceId kind share;
    acquisitionOrder = acquisition phase ordinal;
    source = source sourceKind refName;
  };

  baseRealmRequests = realm:
    let
      id = realm.id;
    in
    [
      (request realm "realm-${id}-state" "host-file-partition" "exclusive" 10 0 "realm-state-dir" realm.paths.stateDir)
      (request realm "realm-${id}-run" "host-file-partition" "exclusive" 10 1 "realm-run-dir" realm.paths.runDir)
      (request realm "realm-${id}-audit" "host-file-partition" "exclusive" 10 2 "realm-audit-dir" realm.paths.auditDir)
      (request realm "realm-${id}-public-socket" "host-file-partition" "exclusive" 10 3 "realm-socket" realm.paths.publicSocket)
      (request realm "realm-${id}-broker-socket" "host-file-partition" "exclusive" 10 4 "realm-socket" realm.paths.brokerSocket)
    ];

  hostMutationRequests = realm:
    lib.optionals realm.broker.hostMutation [
      (request realm "realm-${realm.id}-cgroup" "cgroup-subtree" "exclusive" 20 0 "realm-broker" realm.id)
      (request realm "realm-${realm.id}-nft" "nftables-partition" "shared-partition" 20 1 "realm-broker" realm.id)
    ];

  networkRequests = realm:
    (lib.imap0
      (i: envName:
        (request realm "env-${envName}-bridge" "bridge" "shared-partition" 30 i "env-bridge" envName))
      realm.network.enabledEnvNames)
    ++ lib.optional (realm.network.mode != "none")
      (request realm "realm-${realm.id}-netns" "namespace-boundary" "exclusive" 31 0 "realm-network" realm.id);

  resourceRequests = lib.flatten (map
    (realm: baseRealmRequests realm ++ hostMutationRequests realm ++ networkRequests realm)
    realmRows);

  realmMetadata = realm: {
    inherit (realm)
      realmName
      enabled
      placement
      placementProvider
      providerSpecificPlacement
      ;
    realmId = realm.id;
    realmPath = realm.path;
    hostMutation = realm.broker.hostMutation;
    envNames = realm.network.envNames;
    providerKeys = realm.providerKeys;
  };

  data = {
    schemaVersion = "v2";
    allocator = {
      enabled = realmRows != [ ];
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
    realms = map realmMetadata realmRows;
    resourceRequests = resourceRequests;
    pathPartitions = map pathPartition realmRows;
    providerPlacements = providerPlacements;
    envBridge = envBridgeRows;
    invariants = {
      noRuntimeAllocatorService = true;
      preservesEnvRuntimeSourceOfTruth = true;
      privateMetadataOnly = true;
    };
  };

in
{
  config = {
    assertions = [
      {
        assertion = builtins.substring 0 9 allocatorRootSocket == "/run/d2b/";
        message = "d2b allocator.json rootSocket must remain under /run/d2b while runtime is metadata-only.";
      }
      {
        assertion = builtins.stringLength allocatorRootSocket <= 107;
        message = "d2b allocator.json rootSocket must fit Linux AF_UNIX sockaddr_un.sun_path.";
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
