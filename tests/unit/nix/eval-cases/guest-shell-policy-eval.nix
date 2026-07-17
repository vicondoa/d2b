{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, scenario ? "enabled"
}:

let
  inherit (pkgs) lib;
  scenarios = {
    enabled = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = { enable = true; };
    };
    defaults = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = { };
    };
    custom = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = {
        enable = true;
        defaultName = "ops_1";
        maxSessions = 16;
      };
    };
    shell-no-control = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = { enable = true; };
    };
    shell-no-exec = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = { enable = true; };
    };
    shell-no-user = {
      runtime = "cloud-hypervisor";
      user = null;
      shell = { enable = true; };
    };
    shell-root-user = {
      runtime = "cloud-hypervisor";
      user = "root";
      shell = { enable = true; };
    };
    shell-invalid-name = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = {
        enable = true;
        defaultName = "bad/name";
      };
    };
    shell-too-many-attached = {
      runtime = "cloud-hypervisor";
      user = "alice";
      shell = {
        enable = true;
        maxSessions = 65;
      };
    };
    shell-qemu-media = {
      runtime = "qemu-media";
      user = null;
      shell = { enable = true; };
    };
  };
  selected =
    scenarios.${scenario} or (throw "unknown guest-shell-policy scenario: ${scenario}");

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
            implementationId = selected.runtime;
          };
          workloads.corp = {
            providerRefs.runtime = "runtime";
            shell = selected.shell;
            config = {
              d2b.sshUser = selected.user;
              networking.hostName = lib.mkDefault "corp";
              users.users.alice = { isNormalUser = true; uid = 1000; };
            };
          };
        };
      })
    ];
  };

  cfg = nixos.config;
  workload = builtins.head cfg.d2b._index.workloads.enabledList;
  processDag = builtins.head cfg.d2b._bundle.processesJson.data.vms;
  isCloud = selected.runtime == "cloud-hypervisor";
  guest =
    if isCloud
    then cfg.d2b._computedWorkloads.${workload.workloadId}.config
    else null;
  shellEnabled = selected.shell.enable or false;
  positive =
    if !isCloud then
      assert !(builtins.hasAttr workload.workloadId cfg.d2b._computedWorkloads);
      assert processDag.vm == workload.workloadId;
      assert processDag.workloadIdentity.runtimeKind == "qemu-media";
      builtins.toJSON {
        inherit scenario;
        computedGuest = false;
        runtimeKind = processDag.workloadIdentity.runtimeKind;
        materializedSystemdUnit = false;
      }
    else
      let
        guestd = guest.systemd.services.d2b-guestd;
        execStart = guestd.serviceConfig.ExecStart;
        shpool = guest.systemd.services.d2b-shpool-daemon or null;
      in
      assert guest.d2b.guestControl.enable;
      assert guest.d2b.guestControl.shell.enable == shellEnabled;
      assert guest.d2b.guestControl.exec.enable == shellEnabled;
      assert (shpool != null) == shellEnabled;
      assert (lib.hasInfix "--shell-enable" execStart) == shellEnabled;
      assert !(lib.hasInfix "--exec-allow-root" execStart);
      builtins.toJSON {
        inherit scenario;
        controlEnable = guest.d2b.guestControl.enable;
        execEnable = guest.d2b.guestControl.exec.enable;
        shellEnable = guest.d2b.guestControl.shell.enable;
        defaultName = guest.d2b.guestControl.shell.defaultName;
        maxSessions = guest.d2b.guestControl.shell.maxSessions;
        maxAttached = guest.d2b.guestControl.shell.maxAttached;
        processWorkloadIdentity =
          processDag.workloadIdentity.workloadId == workload.workloadId;
      };
in
if builtins.elem scenario [
  "enabled"
  "defaults"
  "custom"
  "shell-no-control"
  "shell-no-exec"
  "shell-qemu-media"
]
then positive
else builtins.seq
  (builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath)
  "unexpected successful negative scenario"
