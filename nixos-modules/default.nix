# nixling: a generic framework for declaring microVMs on this host.
#
# This module is the public entry point — pulled in by
# `nixosModules.default = import ./nixos-modules { inherit inputs; }`
# in the flake. The closure-passed `inputs` argument lets each
# sub-module that needs flake inputs get them via partial
# application instead of through `_module.args.inputs`. The latter
# infinite-recurses on host.nix's import-list resolution; the
# planning-round NixOS reviewer flagged this in Critical #5 and we
# carry the deep-eval regression test in the smoke flake under
# tests/.
#
# Sub-modules consuming `inputs`:
#   * `host.nix` — `imports = [ inputs.microvm.nixosModules.host ]`
#     (the original case the partial-application wiring was built
#     for).
#   * `components/home-manager.nix` — `imports =
#     [ inputs.home-manager.nixosModules.home-manager ]`. Imported
#     conditionally by host.nix per-VM when `homeManager.enable =
#     true`; the partial application flows through there.
#
# Components live in sibling files (components/graphics.nix, etc.)
# and are conditionally imported per-VM by host.nix.
{ inputs }:

{ ... }:

{
  imports = [
    ./options.nix
    ./assertions.nix
    ./network.nix
    (import ./host.nix { inherit inputs; })
    # ./vms.nix is INTENTIONALLY OMITTED from the public flake — VM
    # registrations are consumer-specific. Downstream users declare
    # their VMs via `nixling.vms.<name> = ...` in their own NixOS
    # module, which is merged into nixling.vms here via option-system
    # semantics. There is no public file with example VMs (yet —
    # examples/ in Phase 6 will demonstrate the pattern).
    ./store.nix
    ./manifest.nix
    ./cli.nix
    ./components/audio/host.nix
  ];

  # Entra ID / Himmelblau is NOT auto-imported here — it lives in
  # the sibling `vicondoa/nixos-entra-id` flake. Consumers bring
  # it in per-VM:
  #
  #   nixling.vms.<vm>.config.imports = [
  #     inputs.nixos-entra-id.nixosModules.default
  #   ];
  #
  # That keeps the himmelblau NixOS module out of nixling's eval
  # graph entirely.
}
