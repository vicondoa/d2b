# nixos-modules/vm-submodule.nix
#
# Wraps the d2b-owned per-VM evaluator (`vm-evaluator.nix`) for
# `host.nix`'s consumption. This file
# stays as the single entry-point for `composeVm` so host.nix
# imports it once. The actual NixOS evaluation logic lives in
# `vm-evaluator.nix`; `vm-options.nix` defines the per-VM
# `microvm.*` option set the evaluator layers in.
#
# No upstream microvm.nix dependency anywhere in this graph.
{ inputs }:
{ config, lib, pkgs, d2bHostTools ? null, d2bHostToolOverrides ? null, ... }:

let
  evaluator = (import ./vm-evaluator.nix { inherit inputs; })
    { inherit config lib pkgs d2bHostTools d2bHostToolOverrides; };
in
{
  _composeVm = evaluator._composeVm;
  config = { };
}
