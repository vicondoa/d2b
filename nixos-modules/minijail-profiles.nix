{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  workloadRows = import ./workload-process-rows.nix {
    inherit config lib pkgs;
  };
  roleRows = import ./role-process-rows.nix {
    inherit config lib pkgs;
  };
  audioRows = import ./realm-audio-rows.nix {
    inherit config lib pkgs;
  };

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = "d2bd";
  };
  writable = path: purpose: { inherit path purpose; };
  profileIdFor = nodeId: "role-${nodeId}";
  roleRowFor = workloadId: roleKind:
    lib.findFirst
      (row: row.workloadId == workloadId && row.roleKind == roleKind)
      null
      roleRows;
  resource = workloadId: kind:
    lib.findFirst
      (row: row.kind == kind)
      (throw "workload ${workloadId} is missing normalized ${kind}")
      (cfg._index.resources.byWorkloadId.${workloadId} or [ ]);
  roleResource = roleId: kind:
    lib.findFirst
      (row: row.kind == kind)
      (throw "role ${roleId} is missing normalized ${kind}")
      (cfg._index.resources.byRoleId.${roleId} or [ ]);
  audioFor = workloadId:
    lib.findFirst
      (row: row.workloadId == workloadId)
      null
      audioRows.workloads;

  defaultNamespaces = {
    ipc = true;
    mount = true;
    net = false;
    pid = false;
    user = false;
    uts = false;
  };

  mkProfile =
    {
      profileId,
      processRole,
      principal,
      cgroupSubtree,
      readOnlyPaths ? [ "/nix/store" ],
      writablePaths ? [ ],
      deviceBinds ? [ ],
      bindMounts ? [ ],
      capabilities ? [ ],
      seccompPolicyRef ? null,
      userNamespace ? null,
      umask ? null,
    }:
    let
      uid = d2bLib.stablePrincipalId principal;
      gid = d2bLib.stablePrincipalId principal;
    in
    {
      inherit
        profileId
        principal
        capabilities
        seccompPolicyRef
        uid
        gid
        userNamespace
        umask
        ;
      role = processRole;
      requiresStartRoot = false;
      exceptionRef = null;
      adr_carve_out = null;
      namespaces = defaultNamespaces // {
        user = userNamespace != null;
      };
      mountPolicy = {
        inherit readOnlyPaths writablePaths deviceBinds bindMounts;
        nixStoreReadOnly = true;
        hideDeviceNodesByDefault = true;
      };
      cgroupPlacement = {
        subtree = cgroupSubtree;
        controllers = [ "cpu" "memory" "pids" ];
        delegated = false;
      };
    };

  toRoleProfile = profile: {
    inherit (profile)
      profileId
      uid
      gid
      namespaces
      seccompPolicyRef
      mountPolicy
      cgroupPlacement
      userNamespace
      umask
      ;
    adr_carve_out = profile.adr_carve_out;
    caps = profile.capabilities;
  };

  seccompFor = processRole: {
    "cloud-hypervisor-runner" = "w1-cloud-hypervisor";
    "qemu-media-runner" = "w1-qemu-media";
    "store-virtiofs-preflight" = "w1-store-virtiofs-preflight";
    "swtpm-pre-start-flush" = "w1-swtpm";
    swtpm = "w1-swtpm";
    virtiofsd = "w1-virtiofsd";
    video = "w1-video";
    gpu = "w1-gpu";
    "gpu-render-node" = "w1-gpu-render-node";
    audio = "w1-audio";
    "vsock-relay" = "w1-vsock-relay";
    "guest-control-health" = "w1-guest-control-health";
    usbip = "w1-usbip";
    "security-key-frontend" = "w1-security-key-frontend";
    "wayland-proxy" = "w1-wayland-proxy";
  }.${processRole} or null;

  deviceBindsFor = processRole:
    if builtins.elem processRole
      [ "cloud-hypervisor-runner" "qemu-media-runner" ]
    then [ "/dev/kvm" "/dev/vhost-net" ]
    else if processRole == "gpu"
    then [ "/dev/dri/renderD128" ]
    else if processRole == "video"
    then [ "/dev/dri/renderD128" ]
    else [ ];

  profileForRole = role:
    let
      principal = "d2b-role-${role.roleId}";
      principalId = d2bLib.stablePrincipalId principal;
      state = (resource role.workloadId "workload-state").path;
      runtime = (resource role.workloadId "workload-runtime").path;
      roleRuntime = (roleResource role.roleId "role-runtime").path;
      isTpm = builtins.elem role.processRole
        [ "swtpm" "swtpm-pre-start-flush" ];
      writablePaths =
        if role.processRole == "cloud-hypervisor-runner"
        then [
          (writable state "Reach workload disks, store metadata, and the vsock endpoint.")
          (writable runtime "Reach role-owned workload endpoints.")
        ]
        else if isTpm
        then [
          (writable "${state}/tpm" "Preserve TPM state across workload and controller restarts.")
          (writable roleRuntime "Create or flush the role-owned TPM endpoint.")
        ]
        else if role.processRole == "audio"
        then
          let audio = audioFor role.workloadId;
          in [
            (writable
              (builtins.dirOf audio.endpoint.path)
              "Create the allocator-declared vhost-user sound endpoint.")
            (writable
              (lib.findFirst
                (row: row.kind == "audio-mediation-runtime")
                null
                audio.storage).path
              "Use the allocator-delivered PipeWire endpoint lease.")
          ]
        else [
          (writable roleRuntime "Create only this role's runtime endpoints.")
        ];
    in
    mkProfile {
      profileId = profileIdFor role.nodeId;
      inherit (role) processRole;
      inherit principal;
      cgroupSubtree =
        "d2b.slice/r-${role.realmId}/workloads/w-${role.workloadId}/${role.roleId}";
      inherit writablePaths;
      deviceBinds = deviceBindsFor role.processRole;
      seccompPolicyRef = seccompFor role.processRole;
      userNamespace =
        if role.processRole == "gpu-render-node"
        then {
          hostUidForZero = principalId;
          hostGidForZero = principalId;
        }
        else null;
      umask =
        if builtins.elem role.processRole
          [ "swtpm" "gpu" "gpu-render-node" "video" "audio" "wayland-proxy" ]
        then 7
        else null;
    };

  shareProfilesFor = workload:
    let
      role = roleRowFor workload.workloadId "virtiofsd";
    in
    if role == null
    then [ ]
    else map
      (share:
        let
          nodeId = "${role.roleId}-${share.tag}";
          principal =
            if share.tag == "d2b-gctl"
            then "d2b-gctlfs-${workload.workloadId}"
            else "d2b-role-${role.roleId}";
          uid = d2bLib.stablePrincipalId principal;
          gid = d2bLib.stablePrincipalId principal;
          servedSource = share.servedSource or share.source;
        in
        mkProfile {
          profileId = profileIdFor nodeId;
          processRole = "virtiofsd";
          inherit principal;
          cgroupSubtree =
            "d2b.slice/r-${role.realmId}/workloads/w-${role.workloadId}/${role.roleId}";
          capabilities = [ ];
          seccompPolicyRef = "w1-virtiofsd";
          readOnlyPaths = [ "/nix/store" servedSource ];
          writablePaths = [
            (writable
              "/run/d2b/r/${role.realmId}/w/${role.workloadId}/roles/${role.roleId}"
              "Create the role-owned virtiofs endpoint.")
          ];
          userNamespace = {
            hostUidForZero = uid;
            hostGidForZero = gid;
          };
        })
      workload.shares;

  singletonProfiles = map profileForRole
    (lib.filter (row: row.roleKind != "virtiofsd") roleRows);
  shareProfiles = lib.concatMap shareProfilesFor workloadRows;
  profiles = singletonProfiles ++ shareProfiles;
  profileTable = lib.listToAttrs (map
    (profile: {
      name = profile.profileId;
      value = profile;
    })
    profiles);
  renderedProfiles = lib.mapAttrs
    (profileId: data:
      let
        file = pkgs.writeText "d2b-${profileId}.json"
          (builtins.toJSON data);
      in
      {
        inherit data;
        path = file;
        relativePath = "minijail-profiles/${profileId}.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
        roleProfile = toRoleProfile data;
      })
    profileTable;

  uidPairs = map
    (profile: {
      inherit (profile) principal uid profileId;
    })
    profiles;
  collisions = lib.filterAttrs
    (_: pairs:
      builtins.length (lib.unique (map (pair: pair.principal) pairs)) > 1)
    (lib.groupBy (pair: toString pair.uid) uidPairs);
in
{
  config = {
    d2b._bundle.minijailProfiles = renderedProfiles;
    environment.etc = lib.mapAttrs'
      (_: profile:
        lib.nameValuePair "d2b/${profile.relativePath}"
          (privateEtc profile.path))
      renderedProfiles;

    assertions = lib.mapAttrsToList
      (uid: pairs: {
        assertion = false;
        message =
          "stable workload role principal collision at uid ${uid}: ${
            lib.concatStringsSep ", " (map (pair: pair.principal) pairs)
          }";
      })
      collisions;
  };
}
