{ inputs }:

{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  composeWorkload =
    ((import ./vm-evaluator.nix { inherit inputs; })
      { inherit config lib pkgs; })._composeWorkload;

  workloadRows = import ./workload-process-rows.nix {
    inherit config lib pkgs;
  };
  localVmRows = lib.filter
    (row: row.runtimeImplementation == "cloud-hypervisor")
    workloadRows;

  # Auto-declare the realm-local net VM as a real workload for every realm
  # that declared a network (`d2b.realms.<realm>.network.mode = "declared"`).
  # The auto-declaration itself lives in options-realms-workloads.nix as a
  # self-referential sibling default resolved inside each realm instance's
  # own submodule fixed point (config.network.mode -> config.workloads /
  # config.providers); that keeps it from reading the fully-merged
  # `d2b.realms` from outside, which would recurse. Because the workload
  # is literally named "network", `identity.deriveWorkloadId realmId
  # "network"` lines up exactly with the `netVmWorkloadId`/`netVmRoleId`
  # realm-network-rows.nix independently derives, and it flows through the
  # ordinary `cfg._index.workloads` / `.roles` / `.resources` pipeline like
  # any other cloud-hypervisor workload (workload-process-rows.nix already
  # special-cases only its `networkInterfaces` shape via `isNetVm`). The
  # one thing options-realms-workloads.nix cannot supply from inside the
  # realm submodule is the realm-derived network guest metadata
  # (`realm.guest`, computed by realm-network-rows.nix from data outside
  # that one realm instance), so it is threaded in here as an extra
  # `_module.args.realmNetwork` override alongside the workload's own
  # `guestModule` (./net.nix) in `composedModules`.
  netVmGuestFor = realmId:
    (lib.findFirst
      (realm: realm.canonicalRealmId == realmId)
      (throw "workload composition: realm ${realmId} has no network rows for its auto-declared net VM")
      (cfg._realmNetwork.realms or [ ])).guest;

  hasRole = row: roleKind:
    lib.any (role: role.roleKind == roleKind) row.roles;
  hasCapability = row: capability:
    let workload = cfg._index.workloads.byId.${row.workloadId};
    in builtins.elem capability (workload.capabilityRefs or [ ]);
  roleIdsFor = row:
    lib.listToAttrs (map
      (role: {
        name = role.roleKind;
        value = role.roleId;
      })
      row.roles);
  resource = row: kind:
    lib.findFirst
      (candidate: candidate.kind == kind)
      (throw "workload ${row.workloadId} is missing normalized ${kind}")
      (cfg._index.resources.byWorkloadId.${row.workloadId} or [ ]);

  guestPolicyModule = row:
    { config, lib, ... }:
    let
      workload = cfg._index.workloads.byId.${row.workloadId};
      shellEnabled = workload.spec.shell.enable or false;
      execEnabled = shellEnabled || hasCapability row "exec";
      maxSessions = workload.spec.shell.maxSessions or 8;
      roleIds = roleIdsFor row;
      state = resource row "workload-state";
      renderNodeOnly =
        lib.attrByPath [ "graphics" "renderNodeOnly" ] false workload.spec;
      vsockCid = 3 + builtins.mod
        (d2bLib.stablePrincipalId "vsock-${row.workloadId}")
        4294967290;
    in
    {
      microvm = {
        vsock = {
          cid = lib.mkForce vsockCid;
          socket = lib.mkForce "${state.path}/vsock.sock";
        };
        interfaces = lib.mkForce
          (map
            (iface: { inherit (iface) type id mac; }
              // lib.optionalAttrs (iface ? macvtap) { inherit (iface) macvtap; })
            row.networkInterfaces);
        shares = lib.mkForce row.shares;
        graphics.enable =
          hasRole row "gpu" || hasRole row "gpu-render-node";
        graphics.renderNodeOnly = lib.mkForce renderNodeOnly;
      };

      d2b.guestControl = {
        enable = lib.mkForce true;
        guestConfigPath = lib.mkForce null;
        exec = {
          enable = lib.mkForce execEnabled;
          execUser = lib.mkForce config.d2b.sshUser;
          detachedMaxRuntimeSec = lib.mkForce 0;
          interactiveMaxRuntimeSec = lib.mkForce 0;
        };
        shell = {
          enable = lib.mkForce shellEnabled;
          defaultName = lib.mkForce (workload.spec.shell.defaultName or "default");
          maxSessions = lib.mkForce maxSessions;
          maxAttached = lib.mkForce (lib.min 4 maxSessions);
        };
      };
    };

  composedModules = row:
    [
      ./base.nix
      ./guest-sshd-host-keys.nix
    ]
    ++ lib.optional (hasRole row "gpu" || hasRole row "gpu-render-node")
      ./components/graphics.nix
    ++ lib.optional (hasRole row "swtpm") ./components/tpm.nix
    ++ lib.optional (hasRole row "usbip") ./components/usbip.nix
    ++ lib.optional (hasRole row "security-key-frontend")
      ./components/security-key-guest.nix
    ++ lib.optional (hasRole row "audio") ./components/audio/guest.nix
    ++ lib.optional (hasRole row "video") ./components/video/guest.nix
    ++ lib.optional (hasCapability row "home-manager")
      ./components/home-manager.nix
    ++ [
      row.guestModule
      (guestPolicyModule row)
    ]
    ++ lib.optional (row.workloadName == "network")
      { _module.args.realmNetwork = netVmGuestFor row.realmId; };
in
{
  imports = [
    ./host-users.nix
    ./host-activation.nix
    ./host-keys.nix
    ./host-ssh-host-keys.nix
    ./observability-host-secrets.nix
    ./host-daemon.nix
  ];

  options.d2b._computedWorkloads = lib.mkOption {
    type = lib.types.attrs;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config = {
    d2b._computedWorkloads = lib.listToAttrs (map
      (row: {
        name = row.workloadId;
        value = composeWorkload
          cfg._index.workloads.byId.${row.workloadId}
          (composedModules row);
      })
      localVmRows);

    boot.kernelModules = [
      "vhost_net"
      "tun"
      "virtio_blk"
      "virtio_console"
      "virtio_net"
      "virtio_pci"
      "virtiofs"
    ] ++ lib.optional
      (lib.any (row: hasRole row "usbip") localVmRows)
      "usbip-host";

    services.udev.extraRules = ''
      KERNEL=="kvm", GROUP="kvm", MODE="0660"
    '';

    environment.systemPackages = [
      pkgs.linuxPackages.usbip
      pkgs.swtpm
      pkgs.tpm2-tools
      pkgs.acl
    ];

    assertions = [
      {
        assertion = lib.all
          (row:
            !(hasRole row "gpu" || hasRole row "audio")
            || pkgs.stdenv.hostPlatform.system == "x86_64-linux")
          localVmRows;
        message =
          "realm workload graphics and audio roles require x86_64-linux";
      }
    ];
  };
}
