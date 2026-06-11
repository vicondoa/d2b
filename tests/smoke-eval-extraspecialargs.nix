# tests/smoke-eval-extraspecialargs.nix — regression test for Spec
# correction #30 (v0.1.1: `nixling.site.extraSpecialArgs` is merged
# into the per-VM `specialArgs` in the nixling-owned VM evaluator).
#
# Mirrors tests/smoke-eval.nix but declares `nixling.site.extraSpecialArgs
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

  flake = builtins.getFlake (toString ./..);
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
          # The crux of this regression test: an arbitrary extra
          # specialArg that the per-VM module below picks up by
          # positional argument. The framework MUST propagate this
          # through the framework's per-VM evaluator
          # (`specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;`):
          # `specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;`).
          extraSpecialArgs = { sentinel = "ok"; };
        };

        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          # Consume `sentinel` positionally and pin its value with
          # an inline `throw` (cheaper to force than threading an
          # assertion through NixOS's normal assertion machinery,
          # which would also force unrelated lazy assertions like
          # the fileSystems-cycle one that don't apply to a microvm
          # guest's tmpfs root). If extraSpecialArgs didn't flow
          # through, NixOS's module system fails the eval with
          # "called without required argument 'sentinel'" before
          # the throw is even reached. The throw catches the case
          # where the framework somehow defaulted `sentinel` to
          # null or shadowed it.
          config = { lib, sentinel, ... }: {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
            # Forces a strict comparison; eval fails noisily on mismatch.
            environment.etc."nixling-extraspecialargs-sentinel".text =
              if sentinel == "ok"
              then "ok"
              else throw "nixling.site.extraSpecialArgs did not flow to per-VM specialArgs (got sentinel=${toString sentinel})";
          };
        };
      })
    ];
  };
in
  # Force the per-VM nixling-owned evaluator so the specialArgs
  # merge path is actually evaluated. Reading the per-VM
  # environment.etc sentinel file's `.text` value forces
  # both the module-argument resolution (proves `sentinel` was
  # propagated) AND the inline throw above (proves the *value*
  # is the expected one).
  builtins.deepSeq
    nixos.config.nixling._computed.corp-vm.config.environment.etc."nixling-extraspecialargs-sentinel".text
    nixos.config.system.build.toplevel
