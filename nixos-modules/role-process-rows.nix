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

  rowFor = workload: role:
    let
      roleId = identity.validateShortId role.roleId;
      roleRoot =
        "${workload.cgroupRoot}/${roleId}";
      normalizedResources =
        cfg._index.resources.byRoleId.${roleId} or [ ];
      deviceResources =
        cfg._index.devices.byRoleId.${roleId} or [ ];
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
      processRole = roleName role.roleKind;
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
