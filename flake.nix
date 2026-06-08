{
  description = "Opinionated NixOS desktop microVM workspaces on microvm.nix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    microvm = {
      url = "github:microvm-nix/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, microvm, home-manager, ... }@inputs:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
    in
    {
      # The public surface area — populated incrementally by the
      # refactor plan. Phase 2b (this commit) wires `nixosModules.default`
      # for real after refactoring `host.nix`'s `{ inputs, ... }:`
      # module-arg into a closure-passed partial application (see
      # `./nixos-modules/default.nix` for the wiring + rationale).
      #
      # Downstream consumers:
      #
      #   imports = [ inputs.nixling.nixosModules.default ];
      #
      # Phase 1+ will populate the remaining surface:
      #   packages.<sys>       — patched cloud-hypervisor, crosvm, etc.
      #   apps.<sys>           — the `nixling` CLI as a runnable app
      #   templates.default    — `nix flake init -t github:vicondoa/nixling`
      #   checks.<sys>         — flake-eval CI gates
      #   lib                  — re-exported helpers (subnetIp, mkMac, …)
      #   overlays.default     — adds vhostDeviceSound, crosvmPatched, …
      nixosModules.default = import ./nixos-modules { inherit inputs; };

      packages = forAllSystems (system: { });

      apps = forAllSystems (system: { });

      templates.default = {
        path = ./templates/default;
        description = "Minimal nixling host scaffold — one env, one headless workload VM";
      };

      # Eval-only gates for the in-tree examples + template. The
      # `system.build.toplevel.drvPath` access is enough to force a
      # full module-system instantiation (option types, assertions,
      # CIDR validators, etc.) without actually realising the closure
      # — which is what we want from a `nix flake check` gate.
      #
      # `with-entra-id` is intentionally absent: it imports
      # `nixos-entra-id.nixosModules.default` from a separate sibling
      # flake, and the root flake doesn't (and shouldn't) pull that
      # in as an input. The example's own `flake.nix` still gates
      # eval via `nix flake check` in its own directory; the
      # `tests/static.sh` examples-iteration step exercises it.
      #
      # The template's `configuration.nix` carries sentinel
      # assertions that fail eval until the operator replaces
      # placeholder values (TODOs 2/3). To eval-check the template
      # without disturbing those assertions for real users, we layer
      # a third module on top that uses `lib.mkForce` to replace
      # just the sentinel-gated fields with valid stand-ins. Sentinel
      # detection logic stays in the template; the override is
      # local to this check.
      checks = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        nixlingModule = import ./nixos-modules { inherit inputs; };
        mkEval = modules: nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [ nixlingModule ] ++ modules;
        };
        mkCheck = name: cfg: pkgs.runCommand "nixling-check-${name}" { } ''
          echo ${builtins.unsafeDiscardStringContext cfg.config.system.build.toplevel.drvPath} > $out
        '';
      in {
        eval-minimal = mkCheck "eval-minimal"
          (mkEval [ (import ./examples/minimal/configuration.nix) ]);

        eval-multi-env = mkCheck "eval-multi-env"
          (mkEval [ (import ./examples/multi-env/configuration.nix) ]);

        # Template eval-check: override the three sentinel-gated
        # fields (TODOs 2 + 3) so the assertion block passes. The
        # template module itself is imported unchanged so any
        # regression in the sentinel logic still surfaces here.
        eval-template = mkCheck "eval-template" (mkEval [
          (import ./templates/default/configuration.nix)
          ({ lib, ... }: {
            # Minimal NixOS baseline the template intentionally
            # omits (TODO 1 — hardware-configuration). Without this
            # the eval would fail on `fileSystems."/"`.
            boot.loader.systemd-boot.enable = lib.mkForce false;
            boot.loader.grub.enable = false;
            boot.initrd.includeDefaultModules = false;
            fileSystems."/" = {
              device = "tmpfs";
              fsType = "tmpfs";
            };
            environment.etc."machine-id".text =
              "00000000000000000000000000000000";

            # Sentinel overrides — these are the three fields gated
            # by the template's assertion block. Each `mkForce`
            # replaces a sentinel with a valid stand-in so the
            # assertions pass and the rest of the module eval runs.
            networking.hostName = lib.mkForce "check-template";
            nixling.site.launcherUsers = lib.mkForce [ "check-user" ];
            nixling.site.userAuthorizedKeys = lib.mkForce [
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBcheckcheckcheckcheckcheckcheckcheckchecky check@template-check"
            ];

            # The launcherUsers principal must be a real user.
            users.users.check-user = {
              isNormalUser = true;
              uid = 1100;
            };
          })
        ]);
      } // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
        # graphics-workstation transitively depends on x86_64-only
        # packages (spectrum-ch, crosvm-patched, vhost-device-sound)
        # and the framework's `checkVmPlatform` gate refuses to
        # evaluate a graphics-enabled VM on a non-x86_64 host. Gate
        # the check on `system == "x86_64-linux"` so aarch64-linux
        # `nix flake check` stays green.
        eval-graphics = mkCheck "eval-graphics"
          (mkEval [ (import ./examples/graphics-workstation/configuration.nix) ]);
      });

      lib = nixpkgs.lib.makeExtensible (_: { });

      overlays.default = _final: _prev: { };
    };
}
