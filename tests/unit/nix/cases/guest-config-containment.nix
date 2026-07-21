{ mkEval, lib, pkgs, flakeRoot, ... }:

let
  host = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.acceptDestructiveV2Cutover = true;
    d2b.realms.work = {
      path = "work";
      placement = "host-local";
      broker = {
        enable = true;
        hostMutation = true;
      };
      network = {
        mode = "declared";
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
      };
      providers.runtime = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.corp = {
        providerRefs.runtime = "runtime";
        config = {
          d2b.sshUser = "alice";
          networking.hostName = lib.mkDefault "corp";
          users.users.alice = { isNormalUser = true; uid = 1000; };
          environment.etc."workload-config".text = "guest-only";
        };
      };
    };
  };

  cfg = (mkEval [ host ]).config;
  workload = lib.findFirst
    (row: row.workloadName == "corp")
    (throw "normalized corp workload missing")
    cfg.d2b._index.workloads.enabledList;
  workloadRows = import (flakeRoot + "/nixos-modules/workload-process-rows.nix") {
    config = cfg;
    inherit lib pkgs;
  };
  workloadRow = lib.findFirst
    (row: row.workloadId == workload.workloadId)
    (throw "rendered corp workload row missing")
    workloadRows;
  roleRows = builtins.filter
    (row: row.workloadId == workload.workloadId)
    (import (flakeRoot + "/nixos-modules/role-process-rows.nix") {
      config = cfg;
      inherit lib pkgs;
    });
  processDag = lib.findFirst
    (row: row.vm == workload.workloadId)
    (throw "rendered corp process DAG missing")
    cfg.d2b._bundle.processesJson.data.vms;
  computed = cfg.d2b._computedWorkloads.${workload.workloadId}.config;
  storeShare = builtins.head
    (builtins.filter (share: share.tag == "ro-store") workloadRow.shares);
in
{
  "guest-config-containment/workload-config-evaluated" = {
    expr = computed.environment.etc."workload-config".text;
    expected = "guest-only";
  };
  "guest-config-containment/canonical-target" = {
    expr = workload.canonicalTarget;
    expected = "corp.work.local-root.d2b";
  };
  "guest-config-containment/workload-id-not-name" = {
    expr = workload.workloadId != workload.workloadName;
    expected = true;
  };
  "guest-config-containment/runtime-role-canonical" = {
    expr =
      let
        runtimeRole = builtins.head
          (builtins.filter
            (role: role.roleKind == "cloud-hypervisor")
            workloadRow.roles);
      in
      workloadRow.runtimeRoleId == runtimeRole.roleId;
    expected = true;
  };
  "guest-config-containment/vm-start-intent-canonical" = {
    expr = workloadRow.vmStartIntentId;
    expected =
      "vm-start:workload:${workload.workloadId}:role:${workloadRow.runtimeRoleId}";
  };
  "guest-config-containment/runner-intent-canonical" = {
    expr = workloadRow.runnerIntentId;
    expected =
      "runner:workload:${workload.workloadId}:role:${workloadRow.runtimeRoleId}";
  };
  "guest-config-containment/no-materialized-workload-unit" = {
    expr = workloadRow.materializedSystemdUnit
      || builtins.any (role: role.materializedSystemdUnit) roleRows;
    expected = false;
  };
  "guest-config-containment/direct-role-leaves" = {
    expr = builtins.all
      (role:
        role.cgroupLeaf
          == "${workloadRow.cgroupRoot}/${role.roleId}"
        && role.cgroupPlacement == "direct-role-leaf")
      roleRows;
    expected = true;
  };
  "guest-config-containment/process-dag-keyed-by-workload-id" = {
    expr = processDag.vm;
    expected = workload.workloadId;
  };
  "guest-config-containment/process-identity-canonical" = {
    expr = processDag.workloadIdentity.workloadId == workload.workloadId
      && processDag.workloadIdentity.realmId == workload.realmId
      && processDag.workloadIdentity.canonicalTarget == workload.canonicalTarget;
    expected = true;
  };
  "guest-config-containment/store-source-sentinel" = {
    expr = storeShare.source;
    expected = "/nix/store";
  };
  "guest-config-containment/store-served-from-farm" = {
    expr = storeShare.servedSource == workloadRow.storeViewLive
      && storeShare.servedSource != "/nix/store";
    expected = true;
  };
  "guest-config-containment/guest-control-share-canonical" = {
    expr = builtins.any
      (share:
        share.tag == "d2b-gctl"
        && share.mountPoint == "/run/d2b-guest-control-host"
        && share.readOnly)
      workloadRow.shares;
    expected = true;
  };
}
