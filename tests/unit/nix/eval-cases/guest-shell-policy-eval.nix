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

        d2b.site = {
          stateDir = lib.mkForce "/var/lib/d2b";
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        d2b.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        d2b.vms.corp-vm = mkCorpVm;
      })
    ];
  };

  corpGuest = nixos.config.d2b._computed.corp-vm.config;

  guestdExecStart = corpGuest.systemd.services.d2b-guestd.serviceConfig.ExecStart or "";
  shpoolService = corpGuest.systemd.services.d2b-shpool-daemon or null;
  shpoolExecStart =
    if shpoolService == null then "" else shpoolService.serviceConfig.ExecStart or "";
  shpoolPam = corpGuest.security.pam.services.d2b-shpool-daemon or null;
  aliceLinger = corpGuest.users.users.alice.linger or false;
  shellManifest = nixos.config.d2b.manifest.corp-vm.shell;
  opShell = nixos.config.d2b.manifest.corp-vm.runtime.operationCapabilities.guest.shell;

  positiveEnabled =
    assert corpGuest.d2b.guestControl.shell.enable == true;
    assert corpGuest.d2b.guestControl.shell.defaultName == "default";
    assert corpGuest.d2b.guestControl.shell.maxSessions == 8;
    assert corpGuest.d2b.guestControl.shell.maxAttached == 1;
    assert lib.hasInfix "--shell-enable" guestdExecStart;
    assert lib.hasInfix "--shell-default-name default" guestdExecStart;
    assert lib.hasInfix "--shell-max-sessions 8" guestdExecStart;
    assert lib.hasInfix "--shell-max-attached 1" guestdExecStart;
    assert lib.hasInfix "--shell-runner-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "/bin/d2b-guest-shell-runner" guestdExecStart;
    assert lib.hasInfix "--shell-systemctl-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "/bin/systemctl" guestdExecStart;
    assert shpoolService != null;
    assert shpoolService.serviceConfig.User == "alice";
    assert shpoolService.serviceConfig.PAMName == "d2b-shpool-daemon";
    assert shpoolService.serviceConfig.Delegate == true;
    assert lib.hasInfix "/nix/store/" shpoolExecStart;
    assert lib.hasInfix "d2b-shpool-daemon-start" shpoolExecStart;
    assert !(lib.hasInfix "%U" shpoolExecStart);
    assert !(lib.hasInfix "%h" shpoolExecStart);
    assert (shpoolService.wantedBy or [ ]) == [ ];
    assert shpoolPam.startSession == false;
    assert shpoolPam.setEnvironment == true;
    assert shpoolPam.setLoginUid == true;
    assert aliceLinger == true;
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
      linger = aliceLinger;
    };

  positiveDefaults =
    assert !(builtins.hasAttr "d2b-guestd" corpGuest.systemd.services);
    assert shpoolService == null;
    assert shpoolPam == null;
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
    assert corpGuest.d2b.guestControl.shell.enable == true;
    assert corpGuest.d2b.guestControl.shell.defaultName == "ops_1";
    assert corpGuest.d2b.guestControl.shell.maxSessions == 16;
    assert corpGuest.d2b.guestControl.shell.maxAttached == 2;
    assert lib.hasInfix "--shell-default-name ops_1" guestdExecStart;
    assert lib.hasInfix "--shell-max-sessions 16" guestdExecStart;
    assert lib.hasInfix "--shell-max-attached 2" guestdExecStart;
    assert lib.hasInfix "--shell-runner-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "--shell-systemctl-path /nix/store/" guestdExecStart;
    assert shpoolService.serviceConfig.User == "alice";
    assert shpoolService.serviceConfig.Delegate == true;
    assert lib.hasInfix "/nix/store/" shpoolExecStart;
    assert lib.hasInfix "d2b-shpool-daemon-start" shpoolExecStart;
    assert !(lib.hasInfix "%U" shpoolExecStart);
    assert !(lib.hasInfix "%h" shpoolExecStart);
    assert shpoolPam.startSession == false;
    assert aliceLinger == true;
    assert shellManifest.defaultName == "ops_1";
    assert shellManifest.maxSessions == 16;
    assert shellManifest.maxAttached == 2;
    builtins.toJSON {
      scenario = "custom";
      defaultName = shellManifest.defaultName;
      maxSessions = shellManifest.maxSessions;
      maxAttached = shellManifest.maxAttached;
      linger = aliceLinger;
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
