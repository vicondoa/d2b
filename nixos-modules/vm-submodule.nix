# nixos-modules/vm-submodule.nix
#
# Wraps the nixling-owned per-VM evaluator (`vm-evaluator.nix`) for
# `host.nix`'s consumption. This file
# stays as the single entry-point for `composeVm` so host.nix
# imports it once. The actual NixOS evaluation logic lives in
# `vm-evaluator.nix`; `vm-options.nix` defines the per-VM
# `microvm.*` option set the evaluator layers in.
#
# No upstream microvm.nix dependency anywhere in this graph.
{ inputs }:
{ config, lib, pkgs, ... }:

let
  evaluator = (import ./vm-evaluator.nix { inherit inputs; })
    { inherit config lib pkgs; };
in
{
  _composeVm = evaluator._composeVm;
  config = { };
}
