{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./..)
, scenario ? "enabled"
}:

let
  inherit (pkgs) lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  scenarios = {
    enabled = {
      controlEnable = true;
      exec = {
        enable = true;
        allowRoot = false;
        users = [ "alice" ];
      };
    };
    default = {
      controlEnable = false;
      exec = { };
    };
    exec-no-control = {
      controlEnable = false;
      exec = {
        enable = true;
        users = [ "alice" ];
      };
    };
    exec-disabled-users = {
      controlEnable = true;
      exec = {
        users = [ "alice" ];
      };
    };
    exec-empty = {
      controlEnable = true;
      exec = {
        enable = true;
      };
    };
    duplicate-user = {
      controlEnable = true;
      exec = {
        enable = true;
        users = [ "alice" "alice" ];
      };
    };
    root-user = {
      controlEnable = true;
      exec = {
        enable = true;
        users = [ "root" ];
      };
    };
    wildcard-user = {
      controlEnable = true;
      exec = {
        enable = true;
        users = [ "*" ];
      };
    };
    missing-user = {
      controlEnable = true;
      exec = {
        enable = true;
        users = [ "bob" ];
      };
    };
    allow-root-only = {
      controlEnable = true;
      exec = {
        enable = true;
        allowRoot = true;
      };
    };
    allow-root-ceiling = {
      controlEnable = true;
      exec = {
        enable = true;
        allowRoot = true;
        detachedMaxRuntimeSec = 3600;
      };
    };
    allow-root-interactive-ceiling = {
      controlEnable = true;
      exec = {
        enable = true;
        allowRoot = true;
        interactiveMaxRuntimeSec = 7200;
      };
    };
    internal-override = {
      controlEnable = true;
      exec = {
        enable = true;
        users = [ "alice" ];
      };
      guestImports = [
        ({ ... }: {
          nixling.guestControl.exec.users = [ "bob" ];
        })
      ];
    };
  };

  selected =
    scenarios.${scenario} or (throw "unknown guest-exec-policy scenario: ${scenario}");

  mkCorpVm = {
    enable = true;
    env = "work";
    index = 10;
    ssh.user = "alice";
    guest.control.enable = selected.controlEnable;
    guest.exec = selected.exec;
    config = {
      imports = selected.guestImports or [ ];
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
  sideGuest = nixos.config.nixling._computed.side-vm.config;
  userdNames = guestConfig:
    lib.filter (name: lib.hasPrefix "nixling-userd-" name)
      (lib.attrNames guestConfig.systemd.services);
  corpUserdNames = userdNames corpGuest;
  sideUserdNames = userdNames sideGuest;
  hostUserdNames = userdNames nixos.config;

  # Detached-runtime surface, all in the COMPUTED GUEST config.
  guestdExecStart = corpGuest.systemd.services.nixling-guestd.serviceConfig.ExecStart;
  guestHasExecSlice = builtins.hasAttr "nixling-exec" corpGuest.systemd.slices;
  guestTmpfilesRules = corpGuest.systemd.tmpfiles.rules or [ ];
  guestHasRunDir = lib.any (r: lib.hasInfix "/run/nixling-exec" r) guestTmpfilesRules;
  # Host systemd attrs must never carry the guest-internal slice/dir
  # (legacy-unit-denylist parity).
  hostHasExecSlice = builtins.hasAttr "nixling-exec" (nixos.config.systemd.slices or { });
  hostTmpfilesRules = nixos.config.systemd.tmpfiles.rules or [ ];
  hostHasRunDir = lib.any (r: lib.hasInfix "/run/nixling-exec" r) hostTmpfilesRules;

  positiveEnabled =
    assert corpGuest.nixling.guestControl.enable == true;
    assert corpGuest.nixling.guestControl.exec.enable == true;
    assert corpGuest.nixling.guestControl.exec.allowRoot == false;
    assert corpGuest.nixling.guestControl.exec.users == [ "alice" ];
    assert corpUserdNames == [ "nixling-userd-alice" ];
    assert sideUserdNames == [ ];
    assert hostUserdNames == [ ];
    assert !(builtins.hasAttr "nixling-userd-root" corpGuest.systemd.services);
    assert !(builtins.hasAttr "nixling-userd@" corpGuest.systemd.services);
    assert corpGuest.systemd.services.nixling-userd-alice.wantedBy == [ ];
    assert corpGuest.systemd.services.nixling-userd-alice.serviceConfig.User == "alice";
    assert corpGuest.systemd.services.nixling-userd-alice.serviceConfig.RuntimeDirectory == "nixling-userd-alice";
    # Detached availability follows exec.enable && allowRoot, NOT merely
    # guestControl.enable: with allowRoot = false the detached surface is absent.
    assert !guestHasExecSlice;
    assert !guestHasRunDir;
    assert !(lib.hasInfix "--systemd-run-path" guestdExecStart);
    assert !(lib.hasInfix "--exec-runner-path" guestdExecStart);
    # Interactive TTY shares the detached gate (it needs the exec-runner
    # helper): with allowRoot = false the interactive ceiling flag is absent.
    assert corpGuest.nixling.guestControl.exec.interactiveMaxRuntimeSec == 0;
    assert !(lib.hasInfix "--interactive-max-runtime-sec" guestdExecStart);
    builtins.toJSON {
      scenario = "enabled";
      userd = corpUserdNames;
      sideUserd = sideUserdNames;
      hostUserd = hostUserdNames;
    };

  positiveDefault =
    assert corpGuest.nixling.guestControl.exec.enable == false;
    assert corpGuest.nixling.guestControl.exec.allowRoot == false;
    assert corpGuest.nixling.guestControl.exec.users == [ ];
    assert corpUserdNames == [ ];
    assert sideUserdNames == [ ];
    assert hostUserdNames == [ ];
    builtins.toJSON {
      scenario = "default";
      userd = corpUserdNames;
      sideUserd = sideUserdNames;
      hostUserd = hostUserdNames;
    };

  positiveAllowRoot =
    assert corpGuest.nixling.guestControl.exec.enable == true;
    assert corpGuest.nixling.guestControl.exec.allowRoot == true;
    assert corpGuest.nixling.guestControl.exec.users == [ ];
    assert corpUserdNames == [ ];
    assert sideUserdNames == [ ];
    assert hostUserdNames == [ ];
    assert !(builtins.hasAttr "nixling-userd-root" corpGuest.systemd.services);
    # Detached surface present in the GUEST config only.
    assert guestHasExecSlice;
    assert guestHasRunDir;
    assert !hostHasExecSlice;
    assert !hostHasRunDir;
    # guestd ExecStart carries absolute store paths for both helpers.
    assert lib.hasInfix "--systemd-run-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "--exec-runner-path /nix/store/" guestdExecStart;
    assert lib.hasInfix "/bin/systemd-run" guestdExecStart;
    assert lib.hasInfix "/bin/nixling-exec-runner" guestdExecStart;
    # detachedMaxRuntimeSec defaults to 0 (indefinite).
    assert lib.hasInfix "--detached-max-runtime-sec 0" guestdExecStart;
    # interactiveMaxRuntimeSec also defaults to 0 (unlimited); the flag is
    # emitted alongside the detached surface so the TTY ceiling is explicit.
    assert corpGuest.nixling.guestControl.exec.interactiveMaxRuntimeSec == 0;
    assert lib.hasInfix "--interactive-max-runtime-sec 0" guestdExecStart;
    builtins.toJSON {
      scenario = "allow-root-only";
      userd = corpUserdNames;
      sideUserd = sideUserdNames;
      hostUserd = hostUserdNames;
    };

  positiveAllowRootCeiling =
    assert corpGuest.nixling.guestControl.exec.enable == true;
    assert corpGuest.nixling.guestControl.exec.allowRoot == true;
    assert corpGuest.nixling.guestControl.exec.detachedMaxRuntimeSec == 3600;
    # A nonzero ceiling propagates as the guestd flag; still no host-side unit.
    assert lib.hasInfix "--detached-max-runtime-sec 3600" guestdExecStart;
    assert guestHasExecSlice;
    assert !hostHasExecSlice;
    assert !hostHasRunDir;
    builtins.toJSON {
      scenario = "allow-root-ceiling";
      maxRuntimeSec = corpGuest.nixling.guestControl.exec.detachedMaxRuntimeSec;
    };
  positiveAllowRootInteractiveCeiling =
    assert corpGuest.nixling.guestControl.exec.enable == true;
    assert corpGuest.nixling.guestControl.exec.allowRoot == true;
    assert corpGuest.nixling.guestControl.exec.interactiveMaxRuntimeSec == 7200;
    # A nonzero interactive ceiling propagates as the guestd flag. The
    # detached ceiling stays at its 0 default in this scenario.
    assert lib.hasInfix "--interactive-max-runtime-sec 7200" guestdExecStart;
    assert lib.hasInfix "--detached-max-runtime-sec 0" guestdExecStart;
    assert guestHasExecSlice;
    assert !hostHasExecSlice;
    assert !hostHasRunDir;
    builtins.toJSON {
      scenario = "allow-root-interactive-ceiling";
      interactiveMaxRuntimeSec = corpGuest.nixling.guestControl.exec.interactiveMaxRuntimeSec;
    };
in
if scenario == "enabled" then
  positiveEnabled
else if scenario == "default" then
  positiveDefault
else if scenario == "allow-root-only" then
  positiveAllowRoot
else if scenario == "allow-root-ceiling" then
  positiveAllowRootCeiling
else if scenario == "allow-root-interactive-ceiling" then
  positiveAllowRootInteractiveCeiling
else
  builtins.seq
    (builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath)
    (builtins.unsafeDiscardStringContext corpGuest.system.build.toplevel.drvPath)
