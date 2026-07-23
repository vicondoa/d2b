# Shared node configuration for d2b runNixOSTest (type-G) integration
# tests. These are the additive, real-kernel coverage layer: a runNixOSTest VM
# boots a real NixOS system with the d2b daemon surface
# (`d2b.daemonExperimental.enable`) and the test script asserts live broker
# / daemon behaviour (socket activation, SO_PEERCRED, the public.sock wire
# surface, audited host mutations) that the PR-tier fake-backed Rust canaries
# and pure-eval gates cannot exercise.
#
# This file is NOT a flake check: the VM tests live under the `vmChecks` flake
# output (selected explicitly by `make test-host-integration`), so the Layer-1
# `nix flake check --no-build --all-systems` never realizes a VM.
{ self, lib }:

let
  # The minimal, hermetic d2b site/env/VM declaration every daemon-host
  # node shares. Mirrors the consumer-style config the smoke evals use: one
  # isolated env with RFC1918 / RFC5737 ranges and a single headless workload
  # VM. No graphics / TPM / USBIP (those are device-bearing G-hw concerns).
  baseD2bConfig = {
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
      usePrebuiltHostTools = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
    };
    # The full daemon + broker systemd surface under test.
    d2b.daemonExperimental.enable = true;
  };
in
{
  # A NixOS module for a runNixOSTest node that boots the d2b daemon host.
  # `extra` is merged as an additional module so individual tests can add
  # per-test config (extra VMs, tampering helpers, a larger disk, etc.). The
  # node provisions the `alice` workload user the base config references.
  #
  # Structured as an attrset-module with everything in `imports` (an attrset is
  # a valid module): `imports` must be top-level, NOT wrapped in `lib.mkMerge`,
  # or the module system rejects it ("option nodes.machine.imports does not
  # exist").
  d2bDaemonNode =
    { extra ? { }, writableStore ? false }:
    { config, ... }:
    {
      imports = [
        self.nixosModules.default
        baseD2bConfig
        extra
        {
          # Headroom for building/activating the bundle + daemon closure inside
          # the VM; the default 1024 MiB is tight once the broker spawns
          # runners.
          virtualisation.memorySize = 3072;
          virtualisation.diskSize = 8192;

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          environment.variables.D2B_MANIFEST_PATH = config.d2b._manifestJsonPath;

          # runNixOSTest runs first-boot activation before systemd-tmpfiles has
          # materialized the d2b state tree. Pre-create the key directory so
          # d2bGenerateKeys can open its flock during the initrd activation
          # path without relying on tmpfiles ordering.
          system.activationScripts.d2bTestStateDirs = {
            deps = [ "users" ];
            text = ''
              install -d -m 0750 -o root -g d2bd /var/lib/d2b
              install -d -m 0710 -o root -g d2b /var/lib/d2b/keys
              : > /var/lib/d2b/keys/.lock
              chown root:root /var/lib/d2b/keys/.lock
              chmod 0600 /var/lib/d2b/keys/.lock
            '';
          };
          system.activationScripts.d2bGenerateKeys.deps = [
            "d2bTestStateDirs"
          ];

          system.stateVersion = "25.11";
        }
        # Opt-in writable same-fs store. ONLY needed by tests that drive the
        # per-VM /nix/store hardlink farm (which requires /var/lib/d2b and
        # /nix/store on the SAME filesystem — hardlinks can't cross FS — and the
        # default runNixOSTest read-only store image splits them). It is OFF by
        # default: `virtualisation.writableStore = true` copies the entire guest
        # closure into a writable overlay at boot, which adds many minutes to
        # (and can hang) VM startup. The daemon/broker activation + host-posture
        # tests (daemon-smoke, bridge-isolation, state-dir-acl, privilege-oracle)
        # never boot a microVM, so they never touch the farm — keep this off for
        # a fast, reliable boot.
        (lib.mkIf writableStore {
          virtualisation.writableStore = true;
        })
      ];
    };

  # Re-exported so tests can assert against the shared declaration.
  inherit baseD2bConfig;
}
