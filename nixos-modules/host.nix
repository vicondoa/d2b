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
          (lib.optional (row.networkInterface != null) {
            inherit (row.networkInterface) type id mac;
          });
        shares = lib.mkForce row.shares;
        graphics.enable =
          hasRole row "gpu" || hasRole row "gpu-render-node";
        graphics.renderNodeOnly = lib.mkForce renderNodeOnly;
      };

      d2b.guestControl = {
        enable = lib.mkForce true;
        guestConfigPath = lib.mkForce null;
        sessionCredential = {
          name = lib.mkForce "d2b-guest-session-v2";
          sourcePath = lib.mkForce
            "/run/d2b-guest-control-host/d2b-guest-session-v2";
        };
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
    ];
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
