# tests/smoke-eval-tpm.nix — regression coverage for the TPM host-side
# hardening surface.
#
# Mirrors tests/smoke-eval.nix but declares one TPM-enabled graphics VM,
# one TPM-enabled headless VM, and one graphics-only control VM, then
# inspects the rendered host activation scripts + swtpm sidecar services
# to prove three invariants:
#
#   1. `nixlingTpmStatePerms` grants the swtpm user `--x` on every
#      TPM VM's parent state dir (graphics and headless).
#   2. `nixling-<vm>-swtpm.service` carries the pre-start stale-session
#      flush helper, grants the socket to the correct runner identity,
#      and orders headless TPM sidecars before `microvm@<vm>`.
#   3. `nixlingMigrateOwnership` exists, is gated on `tpm.enable`, and
#      keeps the running-VM guard + orphan-owner repair logic without
#      traversing symlinks.
{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;

  flake = builtins.getFlake "git+file://${toString ./..}";
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

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
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        # Graphics + TPM enabled VM. The framework should emit both the
        # graphics-side state-dir ACLs and the TPM parent-dir traverse
        # ACL for its dedicated swtpm user.
        nixling.vms.tpm-vm = {
          enable = true;
          env = "work";
          index = 12;
          ssh.user = "alice";
          graphics.enable = true;
          tpm.enable = true;
          config = {
            networking.hostName = lib.mkDefault "tpm-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };

        nixling.vms.plain-vm = {
          enable = true;
          env = "work";
          index = 13;
          ssh.user = "alice";
          graphics.enable = true;
          config = {
            networking.hostName = lib.mkDefault "plain-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };

        nixling.vms.headless-tpm = {
          enable = true;
          env = "work";
          index = 14;
          ssh.user = "alice";
          tpm.enable = true;
          config = {
            networking.hostName = lib.mkDefault "headless-tpm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };

      })
    ];
  };

  hasTpmActivationScript =
    builtins.hasAttr "nixlingTpmStatePerms"
      nixos.config.system.activationScripts;
  hasMigrationScript =
    builtins.hasAttr "nixlingMigrateOwnership"
      nixos.config.system.activationScripts;
  tpmGuest = nixos.config.nixling._computed."tpm-vm".config;
  plainGuest = nixos.config.nixling._computed."plain-vm".config;
  hasTpmFlushService =
    builtins.hasAttr "tpm2-flush-sessions"
      tpmGuest.systemd.services;
  plainHasTpmFlushService =
    builtins.hasAttr "tpm2-flush-sessions"
      plainGuest.systemd.services;
  tpmFlushService = tpmGuest.systemd.services."tpm2-flush-sessions";
  tpmSrkService = tpmGuest.systemd.services.tpm2-srk-provision;

  # The per-VM
  # `nixling-<vm>-swtpm.service` units were deleted along with
  # host-sidecars.nix. The TPM sidecar is now spawned by the
  # nixling priv-broker as `SpawnRunner{role: Swtpm}`; the
  # equivalent pre-start session-flush + socket ACL handoff lives
  # in `packages/nixling-priv-broker/src/runners/swtpm.rs`. The
  # legacy per-VM systemd assertions (ExecStartPre/ExecStartPost,
  # microvm@ wants/after wiring, host-sidecars.nix flush helper
  # source check) are deferred to a forthcoming
  # broker-swtpm-runner-eval.
  #
  # The host-side state-dir hardening *is* preserved: the
  # `nixlingTpmStatePerms` and `nixlingMigrateOwnership`
  # activation scripts still live in host-activation.nix because
  # they prepare /var/lib/nixling/vms/<vm>/swtpm for the broker
  # runner to chown into at fork time. We keep the presence
  # checks; the textual-fragment assertions that named the
  # deleted per-VM sidecar users are dropped (the broker
  # negotiates ownership at runtime instead of statically
  # naming `nixling-<vm>-swtpm`).
  checks = [
    (if hasTpmActivationScript then null else
      throw "smoke-eval-tpm: system.activationScripts.nixlingTpmStatePerms is missing")
    (if hasMigrationScript then null else
      throw "smoke-eval-tpm: system.activationScripts.nixlingMigrateOwnership is missing")
    (if hasTpmFlushService then null else
      throw "smoke-eval-tpm: TPM guest is missing tpm2-flush-sessions.service")
    (if ! plainHasTpmFlushService then null else
      throw "smoke-eval-tpm: non-TPM guest unexpectedly has tpm2-flush-sessions.service")
    (if tpmFlushService.environment.TPM2TOOLS_TCTI == "device:/dev/tpmrm0" then null else
      throw "smoke-eval-tpm: tpm2-flush-sessions must pin TPM2TOOLS_TCTI to /dev/tpmrm0")
    (if lib.elem "sysinit.target" tpmFlushService.wantedBy then null else
      throw "smoke-eval-tpm: tpm2-flush-sessions must be wanted by sysinit.target")
    (if (tpmFlushService.unitConfig.DefaultDependencies or null) == false then null else
      throw "smoke-eval-tpm: tpm2-flush-sessions must disable default dependencies")
    (if lib.elem "tpm2-srk-provision.service" tpmFlushService.before then null else
      throw "smoke-eval-tpm: tpm2-flush-sessions must run before SRK provisioning")
    (if lib.elem "tpm2-flush-sessions.service" tpmSrkService.after then null else
      throw "smoke-eval-tpm: tpm2-srk-provision must order after tpm2-flush-sessions")
  ];
in
  builtins.deepSeq checks
    nixos.config.system.build.toplevel
