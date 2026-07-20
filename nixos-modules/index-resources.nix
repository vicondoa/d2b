{ identity, lib }:

let
  sortNames = lib.sort lib.lessThan;
  sortedNames = attrs: sortNames (lib.attrNames attrs);
  sortUnique = values: sortNames (lib.unique values);
  attrPathOr = path: fallback: attrs: lib.attrByPath path fallback attrs;

  duplicateValues = values:
    lib.unique (lib.filter
      (value: lib.length (lib.filter (candidate: candidate == value) values) > 1)
      values);

  requireUnique = label: values:
    let duplicates = duplicateValues values;
    in
    if duplicates == [ ]
    then true
    else throw "normalized index: duplicate ${label}: ${lib.concatStringsSep ", " duplicates}";

  providerTypeOf = provider:
    if provider ? primaryAuthority then provider.primaryAuthority
    else if provider ? primaryType then provider.primaryType
    else if provider ? type then provider.type
    else provider.kind or null;

  implementationOf = provider:
    if provider ? implementation then provider.implementation
    else if provider ? implementationId then provider.implementationId
    else null;

  runtimeImplementation = spec:
    attrPathOr [ "runtime" "implementation" ]
      (attrPathOr [ "runtime" "implementationId" ]
        (if (spec.kind or null) == "local-vm" then "cloud-hypervisor"
         else if (spec.kind or null) == "qemu-media" then "qemu-media"
         else if (spec.kind or null) == "unsafe-local" then "systemd-user"
         else null)
        spec)
      spec;

  rolesFor = runtime: workload:
    let
      spec = workload.spec;
      explicit = attrPathOr [ "roleKinds" ] [ ] spec;
      localVmRoles = lib.optionals (runtime == "cloud-hypervisor") [
        "store-virtiofs-preflight"
        "virtiofsd"
        "cloud-hypervisor"
        "guest-control-health"
      ];
      qemuRoles = lib.optional (runtime == "qemu-media") "qemu-media";
      tpmRoles = lib.optionals (attrPathOr [ "tpm" "enable" ]
        (attrPathOr [ "localVm" "tpm" "enable" ] false spec) spec) [
        "swtpm-pre-start-flush"
        "swtpm"
      ];
      graphicsEnabled = attrPathOr [ "graphics" "enable" ]
        (attrPathOr [ "localVm" "graphics" "enable" ] false spec) spec;
      graphicsRoles = lib.optionals graphicsEnabled [ "gpu" "gpu-render-node" ];
      videoRoles = lib.optional
        (attrPathOr [ "graphics" "videoSidecar" ] false spec) "video";
      audioRoles = lib.optional
        (attrPathOr [ "audio" "enable" ] false spec) "audio";
      usbRoles = lib.optional
        (attrPathOr [ "usbip" "enable" ]
          (attrPathOr [ "usbip" "yubikey" ] false spec) spec) "usbip";
      securityKeyRoles = lib.optional
        (attrPathOr [ "securityKey" "enable" ] false spec)
        "security-key-frontend";
      graphicalLauncher = lib.any
        (item: item.graphical or false)
        (lib.attrValues workload.launcher.items);
      waylandRoles = lib.optional
        (attrPathOr [ "display" "wayland" ]
          (graphicsEnabled || graphicalLauncher) spec)
        "wayland-proxy";
      relayRoles = lib.optional
        (attrPathOr [ "guestControl" "vsockRelay" ] (runtime == "cloud-hypervisor") spec)
        "vsock-relay";
    in
    sortUnique (explicit ++ localVmRoles ++ qemuRoles ++ tpmRoles
      ++ graphicsRoles ++ videoRoles ++ audioRoles ++ usbRoles
      ++ securityKeyRoles ++ waylandRoles ++ relayRoles);

  mkResource = { resourceId, kind, realmId, workloadId ? null,
      providerId ? null, roleId ? null, path ? null }:
    {
      inherit kind path providerId realmId resourceId roleId workloadId;
    };

  groupBy = field: rows:
    lib.groupBy (row: if row.${field} == null then "none" else row.${field}) rows;
in
{ realms, realmIndex, workloadIndex }:
let
  providerRows = lib.concatMap
    (realmRow:
      let providers = (realms.${realmRow.realmName}).providers or { };
      in map
        (providerName:
          let
            provider = providers.${providerName};
            providerType = providerTypeOf provider;
            configuredProviderId = provider.id or providerName;
            providerId =
              if providerType == null
              then throw "normalized index: provider ${providerName} has no primary authority"
              else identity.deriveProviderId
                realmRow.realmId providerType configuredProviderId;
          in
          {
            inherit configuredProviderId providerId providerName providerType;
            realmId = realmRow.realmId;
            realmName = realmRow.realmName;
            enabled = realmRow.enabled && (provider.enable or true);
            implementationId = implementationOf provider;
            placement =
              if (provider.placement or null) != null
              then provider.placement
              else realmRow.placement;
            capabilityRefs = sortUnique
              (provider.capabilities or provider.capabilityRefs or [ ]);
            configRef = provider.configRef or null;
            metadata = {
              label = provider.label or providerName;
            };
          })
        (sortedNames providers))
    realmIndex.list;

  providerForWorkload = workload:
    let
      providerRef = workload.providerRefs.runtime or null;
      matches = lib.filter
        (provider:
          provider.realmId == workload.realmId
          && provider.providerType == "runtime"
          && (provider.providerName == providerRef
            || provider.configuredProviderId == providerRef))
        providerRows;
    in
    if providerRef == null || matches == [ ] then null else builtins.head matches;

  providerBindingsByWorkloadId = lib.listToAttrs (map
    (workload: {
      name = workload.workloadId;
      value = lib.mapAttrs
        (providerType: providerRef:
          let
            matches = lib.filter
              (provider:
                provider.realmId == workload.realmId
                && provider.providerType == providerType
                && (provider.providerName == providerRef
                  || provider.configuredProviderId == providerRef))
              providerRows;
            provider = if matches == [ ] then null else builtins.head matches;
          in
          if provider == null then null else {
            inherit (provider)
              implementationId providerId providerType;
          })
        workload.providerRefs;
    })
    workloadIndex.list);

  roleRows = lib.concatMap
    (workload:
      let
        provider = providerForWorkload workload;
        runtime =
          let declared = runtimeImplementation workload.spec;
          in if declared != null
             then declared
             else if provider == null then null else provider.implementationId;
      in map
        (roleKind: {
          inherit roleKind;
          roleId = identity.deriveRoleId
            workload.realmId workload.workloadId roleKind;
          realmId = workload.realmId;
          workloadId = workload.workloadId;
          enabled = workload.enabled;
        })
        (rolesFor runtime workload))
    workloadIndex.list;

  controllerRoleFor = realmId:
    if (realmIndex.byId.${realmId}).realmPath == "local-root"
    then "local-root-controller"
    else "realm-controller";

  localTransportImplementations = [
    "cloud-hypervisor-vsock"
    "native-vsock"
    "unix-seqpacket"
    "unix-stream"
  ];
  transportMappings = map
    (provider: {
      inherit (provider) implementationId providerId realmId;
      controllerRole = controllerRoleFor provider.realmId;
      transportBindingIds = [ "transport-${provider.providerId}" ];
    })
    (lib.filter
      (provider:
        provider.enabled
        && provider.providerType == "transport"
        && builtins.elem provider.implementationId localTransportImplementations)
      providerRows);

  substrateMappings = map
    (provider: {
      inherit (provider) implementationId providerId realmId;
      controllerRole = controllerRoleFor provider.realmId;
    })
    (lib.filter
      (provider:
        provider.enabled
        && provider.providerType == "substrate"
        && builtins.elem provider.implementationId [ "linux" "nixos" ]
        && (realmIndex.byId.${provider.realmId}).realmPath == "local-root")
      providerRows);

  enabledWaylandRoles = lib.filter
    (role: role.enabled && role.roleKind == "wayland-proxy")
    roleRows;
  displayMappings = map
    (role:
      let
        workload = workloadIndex.byId.${role.workloadId};
        binding =
          providerBindingsByWorkloadId.${role.workloadId}.display or null;
        provider =
          if binding == null
          then throw
            "normalized index: workload ${role.workloadId} requires an explicit wayland display provider binding"
          else providerById.${binding.providerId}
            or (throw
              "normalized index: workload ${role.workloadId} references an unknown display provider");
        normalizedAuthorityError =
          if binding == null then
            "normalized index: workload ${role.workloadId} requires an explicit wayland display provider binding"
          else if !(
            workload.realmId == role.realmId
            && binding.providerType == "display"
            && binding.implementationId == "wayland"
            && provider.enabled
            && provider.realmId == role.realmId
            && provider.providerType == "display"
            && provider.implementationId == "wayland"
            && provider.placement == "host-local"
          ) then
            "normalized index: workload ${role.workloadId} display binding disagrees with normalized authority"
          else
            null;
      in
      if normalizedAuthorityError != null
      then throw normalizedAuthorityError
      else
      {
        implementationId = binding.implementationId;
        providerId = identity.deriveProviderId
          role.realmId "display" "wayland-${role.workloadId}";
        inherit (role) realmId workloadId;
        ownerRoleId = role.roleId;
        controllerRole = controllerRoleFor role.realmId;
        endpointIds = {
          wayland = "wayland-${role.roleId}";
          crossDomain = "cross-domain-${role.roleId}";
          waypipe = "waypipe-${role.roleId}";
          proxy = "proxy-${role.roleId}";
        };
      })
    enabledWaylandRoles;

  realmStorage = lib.concatMap
    (realm: [
      (mkResource {
        resourceId = "realm/${realm.realmId}/config";
        kind = "realm-config";
        realmId = realm.realmId;
        path = "/etc/d2b/r/${realm.realmId}";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/state";
        kind = "realm-state";
        realmId = realm.realmId;
        path = "/var/lib/d2b/r/${realm.realmId}";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/controller-state";
        kind = "realm-controller-state";
        realmId = realm.realmId;
        path = "/var/lib/d2b/r/${realm.realmId}/controller";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/broker-state";
        kind = "realm-broker-state";
        realmId = realm.realmId;
        path = "/var/lib/d2b/r/${realm.realmId}/broker";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/audit";
        kind = "realm-audit";
        realmId = realm.realmId;
        path = "/var/lib/d2b/r/${realm.realmId}/audit";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/cache";
        kind = "realm-cache";
        realmId = realm.realmId;
        path = "/var/cache/d2b/r/${realm.realmId}";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/runtime";
        kind = "realm-runtime";
        realmId = realm.realmId;
        path = "/run/d2b/r/${realm.realmId}";
      })
      (mkResource {
        resourceId = "realm/${realm.realmId}/locks";
        kind = "realm-locks";
        realmId = realm.realmId;
        path = "/run/d2b/r/${realm.realmId}/locks";
      })
    ])
    realmIndex.list;

  providerStorage = lib.concatMap
    (provider: [
      (mkResource {
        resourceId = "provider/${provider.providerId}/state";
        kind = "provider-state";
        realmId = provider.realmId;
        providerId = provider.providerId;
        path = "/var/lib/d2b/r/${provider.realmId}/providers/${provider.providerId}";
      })
      (mkResource {
        resourceId = "provider/${provider.providerId}/runtime";
        kind = "provider-runtime";
        realmId = provider.realmId;
        providerId = provider.providerId;
        path = "/run/d2b/r/${provider.realmId}/p/${provider.providerId}";
      })
    ])
    providerRows;

  workloadStorage = lib.concatMap
    (workload: [
      (mkResource {
        resourceId = "workload/${workload.workloadId}/config";
        kind = "workload-config";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/etc/d2b/r/${workload.realmId}/w/${workload.workloadId}";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/state";
        kind = "workload-state";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/state-data";
        kind = "workload-state-data";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/state";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/disks";
        kind = "workload-disks";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/disks";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/store-view-live";
        kind = "workload-store-view-live";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/store-view/live";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/store-view-meta";
        kind = "workload-store-view-meta";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/store-view/meta";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/store-view-state";
        kind = "workload-store-view-state";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/store-view/state";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/store-view-gcroots";
        kind = "workload-store-view-gcroots";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/store-view/gcroots";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/tpm";
        kind = "workload-tpm";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/tpm";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/media";
        kind = "workload-media";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/media";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/audio";
        kind = "workload-audio";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/audio";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/keys";
        kind = "workload-keys";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}/keys";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/runtime";
        kind = "workload-runtime";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/run/d2b/r/${workload.realmId}/w/${workload.workloadId}";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/sockets";
        kind = "workload-sockets";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/run/d2b/r/${workload.realmId}/w/${workload.workloadId}/sockets";
      })
      (mkResource {
        resourceId = "workload/${workload.workloadId}/leases";
        kind = "workload-leases";
        realmId = workload.realmId;
        workloadId = workload.workloadId;
        path = "/run/d2b/r/${workload.realmId}/w/${workload.workloadId}/leases";
      })
    ])
    workloadIndex.list;

  roleResources = map
    (role: mkResource {
      resourceId = "role/${role.roleId}/runtime";
      kind = "role-runtime";
      realmId = role.realmId;
      workloadId = role.workloadId;
      roleId = role.roleId;
      path = "/run/d2b/r/${role.realmId}/w/${role.workloadId}/roles/${role.roleId}";
    })
    roleRows;

  transportResources = lib.concatMap
    (mapping:
      map
        (resourceId: mkResource {
          inherit resourceId;
          kind = "transport-binding";
          inherit (mapping) providerId realmId;
        })
        mapping.transportBindingIds)
    transportMappings;

  roleRuntimeResourceFor = roleId:
    lib.findFirst
      (resource: resource.roleId == roleId && resource.kind == "role-runtime")
      (throw "normalized index: display owner role ${roleId} has no runtime resource")
      roleResources;
  displayResources = lib.concatMap
    (mapping:
      lib.mapAttrsToList
        (endpointKind: resourceId: mkResource {
          inherit resourceId;
          kind = "display-endpoint-${
            if endpointKind == "crossDomain" then "cross-domain" else endpointKind
          }";
          inherit (mapping) providerId realmId workloadId;
          roleId = mapping.ownerRoleId;
          path =
            if endpointKind == "wayland"
            then "${(roleRuntimeResourceFor mapping.ownerRoleId).path}/wayland-0"
            else null;
        })
        mapping.endpointIds)
    displayMappings;

  storageRows = realmStorage ++ providerStorage ++ workloadStorage ++ roleResources;
  resourceRows = storageRows ++ transportResources ++ displayResources;
  byId = rows: lib.listToAttrs (map (row: {
    name = row.resourceId;
    value = row;
  }) rows);
  providerById = lib.listToAttrs (map (row: {
    name = row.providerId;
    value = row;
  }) providerRows);
  roleById = lib.listToAttrs (map (row: {
    name = row.roleId;
    value = row;
  }) roleRows);

  identitiesValid = identity.validateGlobalIdentities {
    realms = realmIndex.ids;
    workloads = workloadIndex.ids;
    providers = map (row: row.providerId) providerRows;
    roles = map (row: row.roleId) roleRows;
  };
  resourcesValid = requireUnique "resource id"
    (map (row: row.resourceId) resourceRows);
  providerMappingsValid =
    requireUnique "transport provider mapping"
      (map (row: row.providerId) transportMappings)
    && requireUnique "substrate provider mapping"
      (map (row: row.providerId) substrateMappings)
    && requireUnique "display provider mapping"
      (map (row: row.providerId) displayMappings);
in
assert identitiesValid && resourcesValid && providerMappingsValid;
{
  providers = {
    list = providerRows;
    enabledList = lib.filter (row: row.enabled) providerRows;
    byId = providerById;
    byRealmId = lib.groupBy (row: row.realmId) providerRows;
    bindingsByWorkloadId = providerBindingsByWorkloadId;
    ids = map (row: row.providerId) providerRows;
  };
  roles = {
    list = roleRows;
    enabledList = lib.filter (row: row.enabled) roleRows;
    byId = roleById;
    byWorkloadId = lib.groupBy (row: row.workloadId) roleRows;
    ids = map (row: row.roleId) roleRows;
  };
  storage = {
    list = storageRows;
    byId = byId storageRows;
    byRealmId = groupBy "realmId" storageRows;
    byWorkloadId = groupBy "workloadId" storageRows;
  };
  resources = {
    list = resourceRows;
    byId = byId resourceRows;
    byRealmId = groupBy "realmId" resourceRows;
    byWorkloadId = groupBy "workloadId" resourceRows;
    byProviderId = groupBy "providerId" resourceRows;
    byRoleId = groupBy "roleId" resourceRows;
  };
  providerRegistryV2Mappings = {
    transport = transportMappings;
    substrate = substrateMappings;
    display = displayMappings;
  };
}
