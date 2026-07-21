{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, scenario ? "enabled"
}:

let
  inherit (pkgs) lib;
  scenarios = {
    enabled = { exec = true; user = "alice"; };
    default = { exec = false; user = "alice"; };
    control-no-exec = { exec = false; user = "alice"; };
    detached-ceiling = { exec = true; user = "alice"; };
    interactive-ceiling = { exec = true; user = "alice"; };
    exec-no-control = { exec = true; user = "alice"; };
    exec-no-user = { exec = true; user = null; };
    root-user = { exec = true; user = "root"; };
    invalid-user = { exec = true; user = "Alice"; };
    missing-user = { exec = true; user = "bob"; };
    uid-zero-alias = {
      exec = true;
      user = "toor";
      extraUsers.toor = {
        isSystemUser = true;
        group = "root";
        uid = 0;
      };
    };
  };
  selected =
    scenarios.${scenario} or (throw "unknown guest-exec-policy scenario: ${scenario}");

  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text =
          "00000000000000000000000000000000";
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
            launcher.capabilities = lib.optional selected.exec "exec";
            config = {
              d2b.sshUser = selected.user;
              networking.hostName = lib.mkDefault "corp";
              users.users = {
                alice = { isNormalUser = true; uid = 1000; };
              } // (selected.extraUsers or { });
            };
          };
        };
      })
    ];
  };

  cfg = nixos.config;
  workload = builtins.head cfg.d2b._index.workloads.enabledList;
  guest = cfg.d2b._computedWorkloads.${workload.workloadId}.config;
  guestd = guest.systemd.services.d2b-guestd;
  execStart = guestd.serviceConfig.ExecStart;
  processDag = builtins.head cfg.d2b._bundle.processesJson.data.vms;
  unitNames = lib.attrNames cfg.systemd.services;
  positive =
    assert guest.d2b.guestControl.enable;
    assert guest.d2b.guestControl.exec.enable == selected.exec;
    assert processDag.vm == workload.workloadId;
    assert processDag.workloadIdentity.workloadId == workload.workloadId;
    assert lib.all
      (name:
        !(lib.hasInfix workload.workloadId name)
        && !(lib.hasPrefix "d2b@" name))
      unitNames;
    assert lib.hasInfix "--vm-id ${workload.workloadId}" execStart;
    assert (lib.hasInfix "--exec-enable" execStart) == selected.exec;
    assert (lib.hasInfix "--exec-user" execStart) == selected.exec;
    assert !(lib.hasInfix "--exec-allow-root" execStart);
    builtins.toJSON {
      inherit scenario;
      controlEnable = guest.d2b.guestControl.enable;
      execEnable = guest.d2b.guestControl.exec.enable;
      execUser = guest.d2b.guestControl.exec.execUser;
      detachedMaxRuntimeSec =
        guest.d2b.guestControl.exec.detachedMaxRuntimeSec;
      interactiveMaxRuntimeSec =
        guest.d2b.guestControl.exec.interactiveMaxRuntimeSec;
      processWorkloadIdentity =
        processDag.workloadIdentity.workloadId == workload.workloadId;
      noPerWorkloadUnits = true;
    };
in
if builtins.elem scenario [
  "enabled"
  "default"
  "control-no-exec"
  "detached-ceiling"
  "interactive-ceiling"
  "exec-no-control"
]
then positive
else builtins.seq
  (builtins.unsafeDiscardStringContext guest.system.build.toplevel.drvPath)
  "unexpected successful negative scenario"
