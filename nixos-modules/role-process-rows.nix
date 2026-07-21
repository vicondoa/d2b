{ config, lib, pkgs ? null }:

let
  cfg = config.d2b;
  identity = import ./v2-identity.nix;
  workloadRows = import ./workload-process-rows.nix {
    inherit config lib pkgs;
  };

  roleName = roleKind: {
    "cloud-hypervisor" = "cloud-hypervisor-runner";
    "qemu-media" = "qemu-media-runner";
    "store-virtiofs-preflight" = "store-virtiofs-preflight";
    "swtpm-pre-start-flush" = "swtpm-pre-start-flush";
    swtpm = "swtpm";
    virtiofsd = "virtiofsd";
    video = "video";
    gpu = "gpu";
    "gpu-render-node" = "gpu-render-node";
    audio = "audio";
    "vsock-relay" = "vsock-relay";
    "guest-control-health" = "guest-control-health";
    usbip = "usbip";
    "security-key-frontend" = "security-key-frontend";
    "wayland-proxy" = "wayland-proxy";
  }.${roleKind} or (throw "unsupported realm workload role ${roleKind}");

  # The canonical realm observability rows declare exactly one
  # "vsock-relay"-kind role that is purpose-built as the CH host-egress
  # OTel bridge (see realm-observability-rows.nix). It shares its
  # roleKind with every other workload's generic guest-control relay
  # role (same deriveRoleId formula/inputs), so it is identified here
  # by roleId membership, not by workload/vmName, and only that single
  # role's processRole is overridden to route it to the dedicated
  # otel-host-bridge process node in processes-json.nix.
  observabilityBridgeRoleIds =
    map (row: row.roleId) (cfg._realmObservability.roles or [ ]);

  rowFor = workload: role:
    let
      roleId = identity.validateShortId role.roleId;
      roleRoot =
        "${workload.cgroupRoot}/${roleId}";
      normalizedResources =
        cfg._index.resources.byRoleId.${roleId} or [ ];
      deviceResources =
        cfg._index.devices.byRoleId.${roleId} or [ ];
      isObservabilityBridge =
        role.roleKind == "vsock-relay"
        && lib.elem roleId observabilityBridgeRoleIds;
    in
    {
      inherit roleId roleRoot;
      inherit (workload)
        realmId
        realmPath
        workloadId
        canonicalTarget
        controller
        broker
        ;
      inherit (role) roleKind;
      processRole =
        if isObservabilityBridge
        then "otel-host-bridge"
        else roleName role.roleKind;
      nodeId = roleId;
      profileId = "role-${roleId}";
      cgroupLeaf = roleRoot;
      resourceRefs =
        map (resource: resource.resourceId) normalizedResources
        ++ map (resource: resource.resourceId) deviceResources;
      processOwner = workload.controller;
      supervision = "realm-controller-pidfd";
      cgroupPlacement = "direct-role-leaf";
      materializedSystemdUnit = false;
      receivesSystemdListenFds = false;
      declarativeOnly = true;
    };

  rows = lib.concatMap
    (workload: map (rowFor workload) workload.roles)
    workloadRows;
in
lib.sort
  (left: right:
    lib.lessThan
      "${left.workloadId}:${left.roleId}"
      "${right.workloadId}:${right.roleId}")
  rows
