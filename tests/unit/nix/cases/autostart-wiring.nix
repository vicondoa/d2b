{ mkEval, lib, pkgs, flakeRoot, ... }:

let
  base = { lib, ... }: {
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
      workloads = {
        auto = {
          providerRefs.runtime = "runtime";
          autostart = true;
          config.networking.hostName = lib.mkDefault "auto";
        };
        manual = {
          providerRefs.runtime = "runtime";
          autostart = false;
          config.networking.hostName = lib.mkDefault "manual";
        };
      };
    };
  };

  cfg = (mkEval [ base ]).config;
  svcs = cfg.systemd.services;
  rows = import (flakeRoot + "/nixos-modules/workload-process-rows.nix") {
    config = cfg;
    inherit lib pkgs;
  };
  byName = name:
    builtins.head (builtins.filter (row: row.workloadName == name) rows);
  auto = byName "auto";
  manual = byName "manual";
  workloadUnitNames = lib.filter
    (name:
      lib.hasInfix auto.workloadId name
      || lib.hasInfix manual.workloadId name
      || lib.hasPrefix "d2b@" name)
    (lib.attrNames svcs);
  d2bdWantedBy =
    if builtins.hasAttr "d2bd" svcs then svcs.d2bd.wantedBy or [ ] else [ ];
in
{
  "autostart-wiring/no-per-workload-units" = {
    expr = workloadUnitNames;
    expected = [ ];
  };
  "autostart-wiring/auto-row-enabled" = {
    expr = auto.autostart;
    expected = true;
  };
  "autostart-wiring/manual-row-disabled" = {
    expr = manual.autostart;
    expected = false;
  };
  "autostart-wiring/auto-intent-canonical" = {
    expr = auto.vmStartIntentId;
    expected =
      "vm-start:workload:${auto.workloadId}:role:${auto.runtimeRoleId}";
  };
  "autostart-wiring/manual-intent-canonical" = {
    expr = manual.vmStartIntentId;
    expected =
      "vm-start:workload:${manual.workloadId}:role:${manual.runtimeRoleId}";
  };
  "autostart-wiring/d2bd-present" = {
    expr = builtins.hasAttr "d2bd" svcs;
    expected = true;
  };
  "autostart-wiring/d2bd-wired-multi-user" = {
    expr = builtins.elem "multi-user.target" d2bdWantedBy;
    expected = true;
  };
}
