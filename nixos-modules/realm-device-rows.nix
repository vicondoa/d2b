{ config, lib, ... }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;

  attrPathOr = path: fallback: attrs:
    lib.attrByPath path fallback attrs;
  enabledAt = paths: attrs:
    lib.any (path: attrPathOr path false attrs) paths;
  hasCapability = capability: workload:
    builtins.elem capability (workload.capabilityRefs or [ ]);

  realmById = cfg._index.realms.byId;
  deviceProviders = lib.filter
    (provider:
      provider.enabled
      && provider.providerType == "device"
      && provider.implementationId == "host-mediated"
      && provider.placement == "host-local")
    cfg._index.providers.list;

  providersFor = workload:
    let
      explicit = workload.providerBindings.device or null;
      candidates = lib.filter
        (provider: provider.realmId == workload.realmId)
        deviceProviders;
    in
    if explicit != null
    then lib.filter (provider: provider.providerId == explicit.providerId) candidates
    else candidates;

  requestedKinds = workload:
    let
      spec = workload.spec;
      tpm = enabledAt [
        [ "tpm" "enable" ]
        [ "localVm" "tpm" "enable" ]
      ] spec || hasCapability "tpm" workload;
      usbip = enabledAt [
        [ "usbip" "enable" ]
        [ "usbip" "yubikey" ]
      ] spec || hasCapability "usbip" workload;
      fido = enabledAt [
        [ "securityKey" "enable" ]
        [ "usb" "securityKey" "enable" ]
      ] spec || hasCapability "security-key" workload;
      graphics = enabledAt [
        [ "graphics" "enable" ]
        [ "localVm" "graphics" "enable" ]
      ] spec || hasCapability "gpu" workload;
      video = enabledAt [
        [ "graphics" "videoSidecar" ]
        [ "localVm" "graphics" "videoSidecar" ]
      ] spec || hasCapability "video" workload;
    in
    lib.optionals tpm [ "tpm" ]
    ++ lib.optionals usbip [ "usbip" ]
    ++ lib.optionals fido [ "fido" ]
    ++ lib.optionals graphics [ "gpu" "render-node" ]
    ++ lib.optionals video [ "video" ];

  roleKindFor = kind: {
    tpm = "swtpm";
    usbip = "usbip";
    fido = "security-key-frontend";
    gpu = "gpu";
    "render-node" = "gpu-render-node";
    video = "video";
  }.${kind};

  capabilityFor = kind: {
    tpm = "tpm2-stateful";
    usbip = "usbip-exclusive";
    fido = "fido-ceremony";
    gpu = "gpu-cross-domain";
    "render-node" = "mediated-device";
    video = "video-decode";
  }.${kind};

  leaseFor = workload: kind:
    if kind == "tpm" then {
      resourceId = "device-tpm-${workload.workloadId}";
      share = "exclusive";
    } else if builtins.elem kind [ "usbip" "fido" ] then {
      resourceId = "device-security-key-global";
      share = "exclusive";
    } else {
      resourceId = "device-render-node-global";
      share = "shared-partition";
    };

  endpointFor = workload: roleId: kind:
    let
      roleRoot =
        "/run/d2b/r/${workload.realmId}/w/${workload.workloadId}/roles/${roleId}";
    in
    if kind == "tpm" then "${roleRoot}/tpm.sock"
    else if kind == "video" then "${roleRoot}/video.sock"
    else if kind == "fido" then "${roleRoot}/security-key.sock"
    else null;

  mkRow = workload: provider: kind:
    let
      roleKind = roleKindFor kind;
      roleId = identity.deriveRoleId
        workload.realmId workload.workloadId roleKind;
      lease = leaseFor workload kind;
    in
    {
      schemaVersion = 1;
      resourceId = "device-${workload.workloadId}-${kind}";
      selectorId = "selector-${workload.workloadId}-${kind}";
      resourceKind = kind;
      capability = capabilityFor kind;
      inherit (workload) realmId workloadId;
      inherit (provider) providerId;
      inherit roleId roleKind;
      mediation = {
        authority = "host-mediated";
        attachment = "fd-only";
        broker = "realm-local";
      };
      endpointPath = endpointFor workload roleId kind;
      stateResourceId =
        if kind == "tpm"
        then "workload/${workload.workloadId}/tpm"
        else null;
      allocatorLease = lease;
    };

  workloadRequests = map
    (workload:
      let
        kinds = requestedKinds workload;
        providers = providersFor workload;
      in
      {
        inherit workload kinds providers;
      })
    cfg._index.workloads.enabledList;

  rows = lib.concatMap
    (request:
      if builtins.length request.providers == 1
      then map
        (kind: mkRow request.workload (builtins.head request.providers) kind)
        request.kinds
      else [ ])
    workloadRequests;

  sortedRows = lib.sort
    (left: right: lib.lessThan left.resourceId right.resourceId)
    rows;

  byId = lib.listToAttrs (map
    (row: {
      name = row.resourceId;
      value = row;
    })
    sortedRows);

  allocatorRequestRows = lib.attrValues (lib.listToAttrs (map
    (row: {
      name =
        "${row.realmId}:${row.providerId}:${row.allocatorLease.resourceId}";
      value = {
        realmPath = realmById.${row.realmId}.realmPath;
        resourceId = row.allocatorLease.resourceId;
        kind = "host-file-partition";
        share = row.allocatorLease.share;
        source = {
          kind = "realm-broker";
          refName = row.providerId;
        };
      };
    })
    sortedRows));
  sortedAllocatorRequestRows = lib.sort
    (left: right:
      lib.lessThan
        "${left.realmPath}:${left.resourceId}:${left.source.refName}"
        "${right.realmPath}:${right.resourceId}:${right.source.refName}")
    allocatorRequestRows;
  allocatorRequests = lib.imap0
    (ordinal: request: request // {
      acquisitionOrder = {
        phase = 50;
        inherit ordinal;
      };
    })
    sortedAllocatorRequestRows;

  requestedWithoutProvider = lib.filter
    (request: request.kinds != [ ] && builtins.length request.providers == 0)
    workloadRequests;
  ambiguousProviders = lib.filter
    (request: request.kinds != [ ] && builtins.length request.providers > 1)
    workloadRequests;
  conflictingSecurityKeys = lib.filter
    (request:
      builtins.elem "usbip" request.kinds
      && builtins.elem "fido" request.kinds)
    workloadRequests;
  videoWithoutGraphics = lib.filter
    (request:
      builtins.elem "video" request.kinds
      && !(builtins.elem "gpu" request.kinds))
    workloadRequests;
in
{
  config = {
    assertions = [
      {
        assertion = requestedWithoutProvider == [ ];
        message = "d2b realm device resources require exactly one host-mediated device provider in the workload realm.";
      }
      {
        assertion = ambiguousProviders == [ ];
        message = "d2b realm device resources with multiple host-mediated providers require an explicit device provider binding.";
      }
      {
        assertion = conflictingSecurityKeys == [ ];
        message = "d2b realm workloads cannot request USBIP and FIDO security-key mediation simultaneously.";
      }
      {
        assertion = videoWithoutGraphics == [ ];
        message = "d2b realm workloads cannot request video mediation without GPU mediation.";
      }
    ];

    d2b._index.devices = {
      list = sortedRows;
      inherit byId;
      byRealmId = lib.groupBy (row: row.realmId) sortedRows;
      byWorkloadId = lib.groupBy (row: row.workloadId) sortedRows;
      byProviderId = lib.groupBy (row: row.providerId) sortedRows;
      byRoleId = lib.groupBy (row: row.roleId) sortedRows;
      allocatorLeaseRequests = allocatorRequests;
    };

  };
}
