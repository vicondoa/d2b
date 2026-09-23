# nixos-modules/vm-evaluator.nix
#
# D2b-owned per-VM NixOS evaluator. Each VM is evaluated as a NixOS
# toplevel with the d2b-owned runner option family
# (`d2b.vms.<name>.runner.*` from vm-options.nix) layered in - the
# successor to the retired upstream per-VM evaluation pipeline.
#
# The resulting per-VM evaluation's `config` is a fully-evaluated
# NixOS config attrset containing:
#   - `config.system.build.toplevel` (the per-VM closure)
#   - `config.d2b.vms.<name>.runner.*` (the runner options from
#     vm-options.nix above; consumer-set or default)
#   - everything else a NixOS module evaluation produces (boot,
#     networking, services, etc. - driven by the consumer's module
#     list).
#
# The public `_evalGuest` entry point evaluates a named Guest from its own
# module list and Zone, without reading the host's VM or environment tables.
{ inputs }:
{ config, lib, pkgs, d2bHostTools ? null, d2bHostToolOverrides ? null, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib pkgs; };
  guestHostTools = d2bHostTools;

  # Build a per-VM NixOS evaluation using the host's nixpkgs path.
  # `nixos/lib/eval-config.nix` is the standard NixOS eval entrypoint -
  # it sets up `pkgs`, the module system, and the standard NixOS
  # module set. We layer our d2b-owned vm-options.nix on top so
  # the per-VM config can set `d2b.vms.<name>.runner.*`, etc.
  #
  # The caller passes a LIST of modules (`composedModules`) that
  # together describe the per-VM config. We layer vm-options.nix
  # and the per-VM `_module.args.name` on top.
  evalVm = name: composedModules:
    import (pkgs.path + "/nixos/lib/eval-config.nix") {
      modules = [
        (import ./vm-options.nix { inherit name config lib pkgs; })
        ./vm-guest-base.nix
        ./component-session.nix
        ./guest-broker.nix
        # Inherit host nixpkgs policy so per-VM evals honor the consumer's
        # allowUnfree / overlays / security fixes without re-stating them in
        # each per-VM module.
        {
          nixpkgs.config = config.nixpkgs.config;
          nixpkgs.overlays = config.nixpkgs.overlays;
        }
        { _module.args.name = name; }
      ] ++ composedModules;
      specialArgs =
        { inherit inputs; }
        // cfg.site.extraSpecialArgs
        // {
          d2bInputs = inputs;
          d2bHostTools = guestHostTools;
          d2bHostToolOverrides = d2bHostToolOverrides;
          d2bUsePrebuiltHostTools = cfg.site.usePrebuiltHostTools;
        };
      inherit (pkgs.stdenv.hostPlatform) system;
    };

  composeVm = name: composedModules:
    let
      evaluated = evalVm name composedModules;
    in {
      inherit (evaluated) config options;
    };

  evalGuest =
    { name
    , modules ? [ ]
    , zone ? "local-root"
    , stateDir ? "/var/lib/d2b/zones/${zone}/guests/${name}"
    , vsockCid ? null
    , vsockSocket ? "${stateDir}/vsock.sock"
    , componentSessionEnable ? true
    , guestConfigPath ? null
    }:
    let
      cid =
        if vsockCid != null
        then vsockCid
        else d2bLib.componentSessionVsockCid {
          name = "${zone}/${name}";
          index = null;
          envIndex = null;
        };
    in
    composeVm name ([
      ./base.nix
      {
        d2b.componentSession = {
          enable = componentSessionEnable;
          inherit guestConfigPath zone;
        };
        d2b.vms.${name}.runner = {
          vsock.cid = lib.mkDefault cid;
          vsock.socket = lib.mkDefault vsockSocket;
          shares = lib.mkDefault [
            {
              source = "/nix/store";
              mountPoint = "/nix/.ro-store";
              tag = "ro-store";
              proto = "virtiofs";
            }
            {
              source = "${stateDir}/store-view/meta";
              mountPoint = "/run/d2b-store-meta";
              tag = "d2b-meta";
              proto = "virtiofs";
              readOnly = true;
            }
          ];
        };
      }
    ] ++ modules);
in
{
  # The module body exposes composeVm via a top-level let-binding
  # for host.nix consumers, plus an empty `config = {}` block to
  # satisfy NixOS module loading rules.
  _composeVm = composeVm;
  _evalGuest = evalGuest;
  config = { };
}
