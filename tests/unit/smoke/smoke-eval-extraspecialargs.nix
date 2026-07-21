# tests/unit/smoke/smoke-eval-extraspecialargs.nix — regression test for Spec
# correction #30 (v0.1.1: `d2b.site.extraSpecialArgs` is merged
# into the per-VM `specialArgs` in the d2b-owned VM evaluator).
#
# Mirrors tests/unit/smoke/smoke-eval.nix but declares `d2b.site.extraSpecialArgs
# = { sentinel = "ok"; }` and a per-VM `config` module that consumes
# `sentinel` directly via a positional-attribute argument. If
# extraSpecialArgs ever stops flowing through (e.g. a refactor of
# the per-VM evaluator drops the `// cfg.site.extraSpecialArgs`),
# the per-VM module fails to evaluate ("anonymous function at … called
# without required argument 'sentinel'"). An assertion inside the VM
# config additionally pins the *value* of `sentinel`, so a future refactor
# that passed the argument as `null` (or shadowed it) is also caught.
{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;

  flake = builtins.getFlake "git+file://${toString ./../../..}";
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

        d2b.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
          # The crux of this regression test: an arbitrary extra
          # specialArg that the per-VM module below picks up by
          # positional argument. The framework MUST propagate this
          # through the framework's per-VM evaluator
          # (`specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;`):
          # `specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;`).
          extraSpecialArgs = { sentinel = "ok"; };
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
          workloads.corp-vm = {
            providerRefs.runtime = "runtime";
            config = { lib, sentinel, ... }: {
              networking.hostName = lib.mkDefault "corp-vm";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
              environment.etc."d2b-extraspecialargs-sentinel".text =
                if sentinel == "ok"
                then "ok"
                else throw "d2b.site.extraSpecialArgs did not flow to per-VM specialArgs (got sentinel=${toString sentinel})";
            };
          };
        };
      })
    ];
  };
  workload = lib.findFirst
    (row: row.workloadName == "corp-vm")
    (throw "corp-vm workload missing from normalized index")
    nixos.config.d2b._index.workloads.enabledList;
in
  # Force the per-VM d2b-owned evaluator so the specialArgs
  # merge path is actually evaluated. Reading the per-VM
  # environment.etc sentinel file's `.text` value forces
  # both the module-argument resolution (proves `sentinel` was
  # propagated) AND the inline throw above (proves the *value*
  # is the expected one).
  builtins.deepSeq
    nixos.config.d2b._computedWorkloads.${workload.workloadId}
      .config.environment.etc."d2b-extraspecialargs-sentinel".text
    nixos.config.system.build.toplevel
