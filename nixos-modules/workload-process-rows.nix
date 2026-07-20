{ config, lib, pkgs ? null }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;

  sortBy = field:
    lib.sort (left: right: lib.lessThan left.${field} right.${field});
  attrPathOr = path: fallback: attrs:
    lib.attrByPath path fallback attrs;

  networkRows = import ./realm-network-rows.nix {
    inherit config lib;
  };
  storageRows = import ./realm-storage-rows.nix {
    inherit config lib;
  };
  audioRows =
    if pkgs == null
    then { processes = [ ]; }
    else import ./realm-audio-rows.nix {
      inherit config lib pkgs;
    };
  observabilityRows =
    if cfg.observability.enable or false
    then import ./realm-observability-rows.nix {
      inherit config lib;
    }
    else {
      workload.workloadId = null;
      paths = [ ];
    };

  runtimeBindingFor = workload:
    attrPathOr
      [ "providerBindings" "runtime" ]
      (cfg._index.providers.bindingsByWorkloadId.${workload.workloadId}.runtime or null)
      workload;

  runtimeImplementationFor = workload:
    let binding = runtimeBindingFor workload;
    in if binding == null then null else binding.implementationId;

  realmNetworkFor = realmId:
    lib.findFirst
      (realm: realm.canonicalRealmId == realmId)
      null
      networkRows.realms;

  workloadNetworkFor = workload:
    let
      realm = realmNetworkFor workload.realmId;
      tap =
        if realm == null
        then null
        else lib.findFirst
          (row: row.workloadId == workload.workloadId)
          null
          realm.resources.taps.workloads;
      guest =
        if realm == null
        then null
        else realm.guest.workloads.${workload.workloadName} or null;
    in
    if realm == null || tap == null || guest == null
    then null
    else {
      inherit tap;
      inherit (guest) ip mac;
    };

  storageRefsFor = workload:
    map
      (row: row.id)
      (lib.filter
        (row: row.scope == "workload:${workload.workloadId}")
        storageRows.paths);

  allocatorRefsFor = workload:
    let
      realmRequests = lib.filter
        (request: request.realmPath == workload.realmPath)
        (cfg._realmAllocatorRows.resources or [ ]);
      networkRequests = lib.filter
        (request:
          request.realmPath == workload.realmPath
          && (
            request.resourceId == "tap-${workload.workloadId}"
            || lib.hasPrefix "net-${workload.realmId}-" request.resourceId
          ))
        networkRows.allocatorRequests;
      deviceLeaseIds = map
        (resource: resource.allocatorLeaseId)
        (cfg._index.devices.byWorkloadId.${workload.workloadId} or [ ]);
    in
    map (request: request.resourceId)
      (sortBy "resourceId" (realmRequests ++ networkRequests))
    ++ lib.sort lib.lessThan (lib.unique deviceLeaseIds);

  rowFor = workload:
    let
      realmId = identity.validateShortId workload.realmId;
      workloadId = identity.validateShortId workload.workloadId;
      runtimeBinding = runtimeBindingFor workload;
      runtimeImplementation = runtimeImplementationFor workload;
      roles = sortBy "roleId"
        (cfg._index.roles.byWorkloadId.${workloadId} or [ ]);
      runtimeRoleKind =
        if runtimeImplementation == "cloud-hypervisor"
        then "cloud-hypervisor"
        else if runtimeImplementation == "qemu-media"
        then "qemu-media"
        else throw
          "workload ${workloadId} has unsupported local runtime ${runtimeImplementation}";
      runtimeRole = lib.findFirst
        (role: role.roleKind == runtimeRoleKind)
        (throw
          "workload ${workloadId} is missing normalized runtime role ${runtimeRoleKind}")
        roles;
      runtimeRoleId = runtimeRole.roleId;
      cgroupRoot =
        "/sys/fs/cgroup/d2b.slice/r-${realmId}/workloads/w-${workloadId}";
      network = workloadNetworkFor workload;
      normalized = kind:
        lib.findFirst
          (resource: resource.kind == kind)
          (throw "workload ${workloadId} is missing normalized ${kind}")
          (cfg._index.resources.byWorkloadId.${workloadId} or [ ]);
      keys = normalized "workload-keys";
    in
    {
      inherit
        realmId
        workloadId
        runtimeBinding
        runtimeImplementation
        runtimeRoleId
        runtimeRoleKind
        roles
        cgroupRoot
        ;
      inherit (workload) canonicalTarget realmPath workloadName;
      stateRoot = (normalized "workload-state").path;
      runtimeRoot = (normalized "workload-runtime").path;
      storeViewLive = (normalized "workload-store-view-live").path;
      storeViewMeta = (normalized "workload-store-view-meta").path;
      keyRoot = keys.path;
      controller = "d2bd-r-${realmId}";
      broker = "d2bbr-r-${realmId}";
      vmStartIntentId =
        "vm-start:workload:${workloadId}:role:${runtimeRoleId}";
      runnerIntentId =
        "runner:workload:${workloadId}:role:${runtimeRoleId}";
      autostart = workload.spec.autostart or false;
      guestModule = workload.spec.config or { };
      networkInterface =
        if network == null
        then null
        else {
          type = "tap";
          id = network.tap.ifName;
          mac = network.mac;
          resourceRef = network.tap.resourceId;
        };
      shares = [
        {
          source = "/nix/store";
          servedSource = (normalized "workload-store-view-live").path;
          mountPoint = "/nix/.ro-store";
          tag = "ro-store";
          proto = "virtiofs";
          readOnly = true;
        }
        {
          source = (normalized "workload-store-view-meta").path;
          mountPoint = "/run/d2b-store-meta";
          tag = "d2b-meta";
          proto = "virtiofs";
          readOnly = true;
        }
        {
          source = "${keys.path}/host";
          mountPoint = "/run/d2b-host-keys";
          tag = "d2b-hkeys";
          proto = "virtiofs";
          readOnly = true;
        }
        {
          source = "${keys.path}/sshd";
          mountPoint = "/run/d2b-sshd-host-keys";
          tag = "d2b-ssh-host";
          proto = "virtiofs";
          readOnly = true;
        }
        {
          source = "${(normalized "workload-runtime").path}/guest-session";
          mountPoint = "/run/d2b-guest-control-host";
          tag = "d2b-gctl";
          proto = "virtiofs";
          readOnly = true;
        }
      ];
      resourceRefs = {
        allocator = allocatorRefsFor workload;
        normalized = map
          (resource: resource.resourceId)
          (cfg._index.resources.byWorkloadId.${workloadId} or [ ]);
        storage = storageRefsFor workload;
        audio = map (row: row.processId)
          (lib.filter
            (row: row.workloadId == workloadId)
            audioRows.processes);
        observability =
          lib.optionals
            (observabilityRows.workload.workloadId == workloadId)
            (map (row: row.id) observabilityRows.paths);
      };
      processOwner = "realm-controller";
      supervision = "realm-controller-pidfd";
      cgroupPlacement = "direct-role-leaf";
      workloadInteriorProcessFree = true;
      materializedSystemdUnit = false;
    };

  rows = map rowFor
    (lib.filter
      (workload:
        workload.enabled
        && (cfg._index.realms.byId.${workload.realmId}.placement or null)
          == "host-local"
        && builtins.elem (runtimeImplementationFor workload)
          [ "cloud-hypervisor" "qemu-media" ])
      cfg._index.workloads.enabledList);
in
sortBy "workloadId" rows
