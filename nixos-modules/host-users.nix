{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  identity = import ./v2-identity.nix;
  inherit (d2bLib) stablePrincipalId;

  # Reuse the exact same enabled/host-local-scoped role rows that
  # minijail-profiles.nix / processes-json.nix build their sandbox
  # principals from (role-process-rows.nix, itself built over
  # workload-process-rows.nix's host-local/runtime-eligible workload
  # rows), so the accounts created here can never drift from the
  # numeric uid/gid the sandbox profile independently derives for the
  # identical principal string.
  roleRows = import ./role-process-rows.nix { inherit config lib; };

  # roleKinds that run with a REAL host uid/gid (see
  # minijail-profiles.nix's `profileForRole`: every processRole gets
  # `userNamespace = null` except "gpu-render-node", which fake-roots
  # inside a private user namespace and therefore needs no distinct
  # host account). These seven are the exact distinct sidecar/runner
  # categories the framework's process and ownership contracts name.
  hostPrincipalRoleKinds = [
    "gpu"
    "video"
    "wayland-proxy"
    "audio"
    "swtpm"
    "cloud-hypervisor"
    "qemu-media"
  ];

  hostPrincipalRoles = lib.filter
    (role: builtins.elem role.roleKind hostPrincipalRoleKinds)
    roleRows;

  rolePrincipal = role: "d2b-role-${identity.validateShortId role.roleId}";

  # The narrow guest-control fs principal (minijail-profiles.nix
  # `shareProfilesFor`, guest-control-host.nix, realm-storage-rows.nix)
  # exists only for workloads with a "virtiofsd" role — i.e. only
  # cloud-hypervisor-runtime workloads (qemu-media has no virtiofsd
  # role at all; see index-resources.nix's `rolesFor`). guest-control-host.nix
  # already declares this principal's *group* for its credential
  # directory; the matching user account is declared here alongside
  # every other role principal so the identity is complete and
  # symmetric with the other seven categories.
  gctlfsWorkloadIds = lib.unique (map
    (role: role.workloadId)
    (lib.filter (role: role.roleKind == "virtiofsd") roleRows));

  gctlfsPrincipal = workloadId:
    "d2b-gctlfs-${identity.validateShortId workloadId}";

  # Sibling runtime-role principal for a workload, so the historical
  # GPU sidecar <-> hypervisor runner cross-membership (needed for the
  # shared virtio-gpu vsock/eventfd wiring) survives the move from
  # per-VM-named groups to per-roleId principals.
  runtimeRoleFor = workloadId:
    lib.findFirst
      (role: role.workloadId == workloadId
        && builtins.elem role.roleKind [ "cloud-hypervisor" "qemu-media" ])
      null
      roleRows;

  extraGroupsFor = role:
    if role.roleKind == "gpu"
    then
      let runtimeRole = runtimeRoleFor role.workloadId;
      in [ "kvm" ] ++ lib.optional (runtimeRole != null) (rolePrincipal runtimeRole)
    else if role.roleKind == "audio"
    then [ "audio" ]
    else [ ];

  roleGroupRows = map
    (role: { name = rolePrincipal role; id = stablePrincipalId (rolePrincipal role); })
    hostPrincipalRoles;
  gctlfsGroupRows = map
    (workloadId: {
      name = gctlfsPrincipal workloadId;
      id = stablePrincipalId (gctlfsPrincipal workloadId);
    })
    gctlfsWorkloadIds;
  allPrincipalRows = roleGroupRows ++ gctlfsGroupRows;
  principalNames = map (row: row.name) allPrincipalRows;
  principalIds = map (row: row.id) allPrincipalRows;
in
{
  imports = [
    ./realm-users.nix
    ./realm-access.nix
  ];

  users.groups = {
    # This remains the sole local-root lifecycle admission group.
    d2b = { };
  }
  // (lib.listToAttrs (map
    (role: lib.nameValuePair (rolePrincipal role) { gid = stablePrincipalId (rolePrincipal role); })
    hostPrincipalRoles))
  // (lib.listToAttrs (map
    (workloadId: lib.nameValuePair (gctlfsPrincipal workloadId) {
      gid = stablePrincipalId (gctlfsPrincipal workloadId);
    })
    gctlfsWorkloadIds));

  users.users = lib.mkMerge [
    (lib.genAttrs (cfg.site.launcherUsers or [ ]) (_: {
      extraGroups = [ "d2b" ];
    }))
    (lib.listToAttrs (map
      (role:
        let principal = rolePrincipal role;
        in
        lib.nameValuePair principal {
          isSystemUser = true;
          uid = stablePrincipalId principal;
          group = principal;
          extraGroups = extraGroupsFor role;
          description =
            "d2b ${role.roleKind} sidecar for workload ${role.workloadId} role ${role.roleId}";
        })
      hostPrincipalRoles))
    (lib.listToAttrs (map
      (workloadId:
        let principal = gctlfsPrincipal workloadId;
        in
        lib.nameValuePair principal {
          isSystemUser = true;
          uid = stablePrincipalId principal;
          group = principal;
          description =
            "d2b narrow guest-control fs principal for workload ${workloadId}";
        })
      gctlfsWorkloadIds))
  ];

  assertions = [
    {
      assertion = builtins.length principalNames == builtins.length (lib.unique principalNames);
      message = "d2b workload principal collision: canonical role/workload principal names must be unique";
    }
    {
      assertion = builtins.length principalIds == builtins.length (lib.unique principalIds);
      message = "d2b workload principal collision: stable UID/GID allocation must be unique";
    }
  ];
}
