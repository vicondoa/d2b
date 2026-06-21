{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, scenario ? "enabled"
}:

let
  inherit (pkgs) lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  scenarios = {
    enabled = {
      controlEnable = true;
      sshUser = "alice";
      exec.enable = true;
      shell.enable = true;
    };
    defaults = {
      controlEnable = false;
      sshUser = "alice";
      exec = { };
      shell = { };
    };
    custom = {
      controlEnable = true;
      sshUser = "alice";
      exec.enable = true;
      shell = {
        enable = true;
        defaultName = "ops_1";
        maxSessions = 16;
        maxAttached = 2;
      };
    };

    shell-no-control = {
      controlEnable = false;
      sshUser = "alice";
      exec.enable = true;
      shell.enable = true;
    };
    shell-no-exec = {
      controlEnable = true;
      sshUser = "alice";
      exec = { };
      shell.enable = true;
    };
    shell-no-user = {
      controlEnable = true;
      sshUser = null;
      exec.enable = true;
      shell.enable = true;
    };
    shell-root-user = {
      controlEnable = true;
      sshUser = "root";
      exec.enable = true;
      shell.enable = true;
    };
    shell-invalid-name = {
      controlEnable = true;
      sshUser = "alice";
      exec.enable = true;
      shell = {
        enable = true;
        defaultName = "bad/name";
      };
    };
    shell-too-many-attached = {
      controlEnable = true;
      sshUser = "alice";
      exec.enable = true;
      shell = {
        enable = true;
        maxSessions = 1;
        maxAttached = 2;
      };
    };
    shell-qemu-media = {
      runtimeKind = "qemu-media";
      controlEnable = false;
      sshUser = null;
      exec = { };
      shell.enable = true;
    };
  };

  selected =
    scenarios.${scenario} or (throw "unknown guest-shell-policy scenario: ${scenario}");

  mkCorpVm = {
    enable = true;
    runtime.kind = selected.runtimeKind or "nixos";
    env = "work";
    index = 10;
    ssh.user = selected.sshUser;
    guest.control.enable = selected.controlEnable;
    guest.exec = selected.exec;
    guest.shell = selected.shell;
  } // lib.optionalAttrs ((selected.runtimeKind or "nixos") == "nixos") {
    config = {
      networking.hostName = lib.mkDefault "corp-vm";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
    };
  };

  nixos = nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = {
          device = "tmpfs";
          fsType = "tmpfs";
        };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        nixling.site = {
          stateDir = lib.mkForce "/var/lib/nixling";
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        nixling.vms.corp-vm = mkCorpVm;
      })
    ];
  };

  corpGuest = nixos.config.nixling._computed.corp-vm.config;
  guestdExecStart = corpGuest.systemd.services.nixling-guestd.serviceConfig.ExecStart or "";
  shellManifest = nixos.config.nixling.manifest.corp-vm.shell;
  opShell = nixos.config.nixling.manifest.corp-vm.runtime.operationCapabilities.guest.shell;

  positiveEnabled =
    assert corpGuest.nixling.guestControl.shell.enable == true;
    assert corpGuest.nixling.guestControl.shell.defaultName == "default";
    assert corpGuest.nixling.guestControl.shell.maxSessions == 8;
    assert corpGuest.nixling.guestControl.shell.maxAttached == 1;
    assert lib.hasInfix "--shell-enable" guestdExecStart;
    assert lib.hasInfix "--shell-default-name default" guestdExecStart;
    assert lib.hasInfix "--shell-max-sessions 8" guestdExecStart;
    assert lib.hasInfix "--shell-max-attached 1" guestdExecStart;
    assert shellManifest.enabled == true;
    assert shellManifest.defaultName == "default";
    assert shellManifest.maxSessions == 8;
    assert shellManifest.maxAttached == 1;
    assert opShell == true;
    builtins.toJSON {
      scenario = "enabled";
      defaultName = shellManifest.defaultName;
      maxSessions = shellManifest.maxSessions;
      maxAttached = shellManifest.maxAttached;
    };

  positiveDefaults =
    assert !(builtins.hasAttr "nixling-guestd" corpGuest.systemd.services);
    assert shellManifest.enabled == false;
    assert shellManifest.defaultName == "default";
    assert shellManifest.maxSessions == 8;
    assert shellManifest.maxAttached == 1;
    builtins.toJSON {
      scenario = "defaults";
      enabled = shellManifest.enabled;
      defaultName = shellManifest.defaultName;
    };

  positiveCustom =
    assert corpGuest.nixling.guestControl.shell.enable == true;
    assert corpGuest.nixling.guestControl.shell.defaultName == "ops_1";
    assert corpGuest.nixling.guestControl.shell.maxSessions == 16;
    assert corpGuest.nixling.guestControl.shell.maxAttached == 2;
    assert lib.hasInfix "--shell-default-name ops_1" guestdExecStart;
    assert lib.hasInfix "--shell-max-sessions 16" guestdExecStart;
    assert lib.hasInfix "--shell-max-attached 2" guestdExecStart;
    assert shellManifest.defaultName == "ops_1";
    assert shellManifest.maxSessions == 16;
    assert shellManifest.maxAttached == 2;
    builtins.toJSON {
      scenario = "custom";
      defaultName = shellManifest.defaultName;
      maxSessions = shellManifest.maxSessions;
      maxAttached = shellManifest.maxAttached;
    };
in
if scenario == "enabled" then
  positiveEnabled
else if scenario == "defaults" then
  positiveDefaults
else if scenario == "custom" then
  positiveCustom
else
  builtins.seq
    (builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath)
    (builtins.unsafeDiscardStringContext (corpGuest.system.build.toplevel.drvPath or ""))
