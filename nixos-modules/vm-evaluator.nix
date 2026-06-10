# nixos-modules/vm-evaluator.nix
#
# Nixling-owned per-VM NixOS evaluator. Replaces the upstream
# `inputs.microvm.nixosModules.host` per-VM evaluation
# pipeline (which used `microvm.vms = lib.mapAttrs ...` + the
# microvm.nix host module's `lib.evalModules` invocation).
#
# Usage from `host.nix`:
#
#   composeVm = (import ./vm-evaluator.nix { inherit inputs; })
#     { inherit config lib pkgs; };
#   nixling.vms = lib.mapAttrs (name: vm: vm // {
#     computed = composeVm name vm;
#   }) cfg.vms;
#
# The resulting `nixling.vms.<name>.computed.config` is a fully-
# evaluated NixOS config attrset containing:
#   - `config.system.build.toplevel` (the per-VM closure)
#   - `config.microvm.*` (the runner options from vm-options.nix
#     above; consumer-set or default)
#   - everything else a NixOS module evaluation produces (boot,
#     networking, services, etc. — driven by the consumer's
#     `vm.config` module list).
{ inputs }:
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;

  # Build a per-VM NixOS evaluation using the host's nixpkgs path.
  # `nixos/lib/eval-config.nix` is the standard NixOS eval entrypoint —
  # it sets up `pkgs`, the module system, and the standard NixOS
  # module set. We layer our nixling-owned vm-options.nix on top so
  # the per-VM config can set `microvm.mem`, etc.
  #
  # The caller (host.nix's composeVm wrapper) already merges
  # `./base.nix`, `./guest-sshd-host-keys.nix`, the per-component
  # guest modules, and `vm.config` (the consumer's module list)
  # into the `composedConfig` it passes here, so we do NOT layer
  # those again — double-imports of `./base.nix` would multiply
  # evaluate the framework baseline.
  # Build a per-VM NixOS evaluation using the host's nixpkgs path.
  # The caller passes a LIST of modules (`composedModules`) that
  # together describe the per-VM config. We layer vm-options.nix
  # and the per-VM `_module.args.name` on top.
  evalVm = name: composedModules:
    import (pkgs.path + "/nixos/lib/eval-config.nix") {
      modules = [
        ./vm-options.nix
        ./vm-guest-base.nix
        ./guest-control.nix
        # inherit the host's nixpkgs.config so per-VM evals
        # honor the consumer's allowUnfree / allowUnfreePredicate /
        # permittedInsecurePackages settings without re-stating them
        # in each per-VM module.
        { nixpkgs.config = config.nixpkgs.config; }
        { _module.args.name = name; }
      ] ++ composedModules;
      specialArgs = { inherit inputs; } // cfg.site.extraSpecialArgs;
      inherit (pkgs.stdenv.hostPlatform) system;
    };

  composeVm = name: composedModules:
    let
      evaluated = evalVm name composedModules;
    in {
      inherit (evaluated) config options;
    };
in
{
  # The module body exposes composeVm via a top-level let-binding
  # for host.nix consumers, plus an empty `config = {}` block to
  # satisfy NixOS module loading rules.
  _composeVm = composeVm;
  config = { };
}
