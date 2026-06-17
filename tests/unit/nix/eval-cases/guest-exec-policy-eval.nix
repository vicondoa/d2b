{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, scenario ? "enabled"
}:

let
  inherit (pkgs) lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  # Each scenario sets the host-side per-VM `guest.control.enable`, the
  # `guest.exec` policy block, and the workload user (`ssh.user`) every exec
  # runs as. The exec model is workload-user-only: there is no root exec and no
  # allowlist; the target user is always `ssh.user`, validated at eval time.
  scenarios = {
    enabled = {
      controlEnable = true;
      sshUser = "alice";
      exec.enable = true;
    };
    default = {
      controlEnable = false;
      sshUser = "alice";
      exec = { };
    };
    # Guest-control enabled, but the `guest.exec` block omitted entirely: exec
    # must default OFF and no exec wiring may leak into the guestd ExecStart.
    control-no-exec = {
      controlEnable = true;
      sshUser = "alice";
      exec = { };
    };
    detached-ceiling = {
      controlEnable = true;
      sshUser = "alice";
      exec = {
        enable = true;
        detachedMaxRuntimeSec = 3600;
      };
    };
    interactive-ceiling = {
      controlEnable = true;
      sshUser = "alice";
      exec = {
        enable = true;
        interactiveMaxRuntimeSec = 7200;
      };
    };

    # Negative scenarios (must fail eval).
    exec-no-control = {
      controlEnable = false;
      sshUser = "alice";
      exec.enable = true;
    };
    exec-no-user = {
      controlEnable = true;
      sshUser = null;
      exec.enable = true;
    };
    root-user = {
      controlEnable = true;
      sshUser = "root";
      exec.enable = true;
    };
    invalid-user = {
      controlEnable = true;
      sshUser = "Alice";
      exec.enable = true;
    };
    missing-user = {
      controlEnable = true;
      sshUser = "bob";
      exec.enable = true;
    };

    # A non-root NAME aliased to UID 0: passes the name pattern + `!= "root"`
    # check, but must be rejected by the effective-UID assertion.
    uid-zero-alias = {
      controlEnable = true;
      sshUser = "toor";
      exec.enable = true;
      extraUsers.toor = {
        isSystemUser = true;
        group = "root";
        uid = 0;
      };
    };
  };

  selected =
    scenarios.${scenario} or (throw "unknown guest-exec-policy scenario: ${scenario}");

  mkCorpVm = {
    enable = true;
    env = "work";
    index = 10;
    ssh.user = selected.sshUser;
    guest.control.enable = selected.controlEnable;
    guest.exec = selected.exec;
    config = {
      networking.hostName = lib.mkDefault "corp-vm";
      users.users = {
        alice = {
          isNormalUser = true;
          uid = 1000;
        };
      } // (selected.extraUsers or { });
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
        nixling.vms.side-vm = {
          enable = true;
          env = "work";
          index = 11;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "side-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };

  corpGuest = nixos.config.nixling._computed.corp-vm.config;

  # No per-user `nixling-userd-*` services exist anywhere anymore.
  userdNames = guestConfig:
    lib.filter (name: lib.hasPrefix "nixling-userd-" name)
      (lib.attrNames guestConfig.systemd.services);

  guestdExecStart = corpGuest.systemd.services.nixling-guestd.serviceConfig.ExecStart;
  # The guest-internal detached slice + /run/nixling-exec dir are emitted as
  # part of the exec runtime substrate when guest-control exec is enabled.
  guestHasExecSlice = builtins.hasAttr "nixling-exec" (corpGuest.systemd.slices or { });
  guestTmpfilesRules = corpGuest.systemd.tmpfiles.rules or [ ];
  guestHasRunDir = lib.any (r: lib.hasInfix "/run/nixling-exec" r) guestTmpfilesRules;

  positiveEnabled =
    assert corpGuest.nixling.guestControl.enable == true;
    assert corpGuest.nixling.guestControl.exec.enable == true;
    # The host-fixed workload user is derived from ssh.user.
    assert corpGuest.nixling.guestControl.exec.execUser == "alice";
    # No userd services anywhere (the stub + scaffolding were removed).
    assert userdNames corpGuest == [ ];
    assert userdNames nixos.config == [ ];
    assert !(builtins.hasAttr "nixling-userd-alice" corpGuest.systemd.services);
    # guestd ExecStart carries the workload user + the exec-runtime helper paths
    # (systemd-run + exec-runner), wired whenever exec is enabled.
    assert lib.hasInfix "--exec-user alice" guestdExecStart;
    assert lib.hasInfix "--systemd-run-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "--exec-runner-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "/bin/systemd-run" guestdExecStart;
    assert lib.hasInfix "/bin/nixling-exec-runner" guestdExecStart;
    # Both ceilings default to 0 (unlimited) and are emitted explicitly.
    assert lib.hasInfix "--interactive-max-runtime-sec 0" guestdExecStart;
    assert lib.hasInfix "--detached-max-runtime-sec 0" guestdExecStart;
    # No root-exec flag is ever emitted.
    assert !(lib.hasInfix "--exec-allow-root" guestdExecStart);
    # The detached runtime substrate (slice + boot-scoped parent dir) is
    # declared as part of the both-or-neither exec runtime bundle whenever
    # exec is enabled; guestd decides at runtime whether to serve detached.
    assert guestHasExecSlice;
    assert guestHasRunDir;
    builtins.toJSON {
      scenario = "enabled";
      execUser = corpGuest.nixling.guestControl.exec.execUser;
    };

  positiveDefault =
    assert corpGuest.nixling.guestControl.exec.enable == false;
    assert userdNames corpGuest == [ ];
    # With guest-control disabled the guestd service is not emitted at all.
    assert !(builtins.hasAttr "nixling-guestd" corpGuest.systemd.services);
    builtins.toJSON {
      scenario = "default";
      execUser = corpGuest.nixling.guestControl.exec.execUser;
    };

  positiveControlNoExec =
    # Control enabled but the `guest.exec` block omitted: exec defaults OFF.
    assert corpGuest.nixling.guestControl.enable == true;
    assert corpGuest.nixling.guestControl.exec.enable == false;
    # guestd IS emitted (the control plane is up), but with NO exec wiring.
    assert builtins.hasAttr "nixling-guestd" corpGuest.systemd.services;
    assert !(lib.hasInfix "--exec-enable" guestdExecStart);
    assert !(lib.hasInfix "--exec-user" guestdExecStart);
    assert !(lib.hasInfix "--systemd-run-path" guestdExecStart);
    assert !(lib.hasInfix "--exec-runner-path" guestdExecStart);
    # No root-exec flag is ever emitted.
    assert !(lib.hasInfix "--exec-allow-root" guestdExecStart);
    # The detached runtime substrate is not declared when exec is disabled.
    assert !guestHasExecSlice;
    assert !guestHasRunDir;
    builtins.toJSON {
      scenario = "control-no-exec";
      controlEnable = corpGuest.nixling.guestControl.enable;
      execEnable = corpGuest.nixling.guestControl.exec.enable;
    };

  positiveDetachedCeiling =
    assert corpGuest.nixling.guestControl.exec.enable == true;
    assert corpGuest.nixling.guestControl.exec.detachedMaxRuntimeSec == 3600;
    assert lib.hasInfix "--detached-max-runtime-sec 3600" guestdExecStart;
    builtins.toJSON {
      scenario = "detached-ceiling";
      maxRuntimeSec = corpGuest.nixling.guestControl.exec.detachedMaxRuntimeSec;
    };

  positiveInteractiveCeiling =
    assert corpGuest.nixling.guestControl.exec.enable == true;
    assert corpGuest.nixling.guestControl.exec.interactiveMaxRuntimeSec == 7200;
    assert lib.hasInfix "--interactive-max-runtime-sec 7200" guestdExecStart;
    assert lib.hasInfix "--detached-max-runtime-sec 0" guestdExecStart;
    builtins.toJSON {
      scenario = "interactive-ceiling";
      interactiveMaxRuntimeSec = corpGuest.nixling.guestControl.exec.interactiveMaxRuntimeSec;
    };
in
if scenario == "enabled" then
  positiveEnabled
else if scenario == "default" then
  positiveDefault
else if scenario == "control-no-exec" then
  positiveControlNoExec
else if scenario == "detached-ceiling" then
  positiveDetachedCeiling
else if scenario == "interactive-ceiling" then
  positiveInteractiveCeiling
else
  builtins.seq
    (builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath)
    (builtins.unsafeDiscardStringContext corpGuest.system.build.toplevel.drvPath)
