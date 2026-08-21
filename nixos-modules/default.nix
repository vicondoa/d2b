# d2b: a generic framework for declaring microVMs on this host.
#
# This module is the public entry point - pulled in by
# `nixosModules.default = import ./nixos-modules { inherit inputs; }`
# in the flake. The closure-passed `inputs` argument lets each
# sub-module that needs flake inputs get them via partial
# application instead of through `_module.args.inputs`. The latter
# infinite-recurses on host.nix's import-list resolution; the
# deep-eval regression test in the smoke flake covers this wiring.
#
# Sub-modules consuming `inputs`
#   * `host.nix` - `imports = [ inputs.microvm.nixosModules.host ]`
#     (the original case the partial-application wiring was built
#     for).
#   * `components/home-manager.nix` - `imports =
#     [ inputs.home-manager.nixosModules.home-manager ]`. Imported
#     conditionally by host.nix per-VM when `homeManager.enable =
#     true`; the partial application flows through there.
#
# Components live in sibling files (components/graphics.nix,
# components/audit.nix, etc.) and are conditionally imported per-VM
# by host.nix.
{ inputs }:

{ config, lib, pkgs, ... }:

let
  d2bHostTools = import ./rust-host-tools.nix {
    inherit inputs lib pkgs;
  };
in
{
  _module.args.d2bHostTools = d2bHostTools;

  imports = [
    ./options.nix
    ./host-generation-rebuild-ref.nix
    ../packages/d2b-provider-volume-local/nix/options-volumes.nix
    ./resources-zone-control.nix
    ./resource-compiler.nix
    ./bundle-artifacts.nix
    ./options-observability.nix
    ./provider-catalog.nix
    ./provider-runtime-contracts.nix
    ../packages/d2b-provider-activation-nixos/nix/default.nix
    ./providers/system-minijail.nix
    ./providers/system-systemd.nix
    ./provider-projection-validate.nix
    ./options-ownership-matrix.nix
    ./index.nix
    ./assertions.nix
    ../packages/d2b-provider-network-local/nix/network.nix
    ./gateway-vm.nix
    (import ./host.nix { inherit inputs; })
    ./unsafe-local-helper.nix
    # host-otel-relay-acl.nix retired per ADR 0018.
    # The OTel host-bridge + per-VM relay ACL contract moved into the
    # broker pre-spawn pipeline (`SpawnRunner{role: OtelHostBridge}`
    # in `packages/d2b-broker/src/runtime.rs`). The retired
    # module file is kept as a stub for one release for diff
    # readability; consumers should not import it directly. A future
    # commit deletes the stub file outright.
    # ./host-otel-relay-acl.nix
    # ./vms.nix is INTENTIONALLY OMITTED from the public flake - VM
    # registrations are consumer-specific. Downstream users declare
    # their VMs via `d2b.vms.<name> = ...` in their own NixOS
    # module, which is merged into d2b.vms here via option-system
    # semantics. There is no public file with example VMs (yet -
    # examples/ will demonstrate the pattern).
    ./observability-vm.nix
    ../packages/d2b-provider-clipboard-wayland/nix/site.nix
    ../packages/d2b-provider-notification-desktop/nix/site.nix
    ../packages/d2b-provider-volume-local/nix/store.nix
    ./manifest.nix
    ./bundle.nix
    ./guest-control-host.nix
    ./host-json.nix
    ./processes-json.nix
    ../packages/d2b-provider-volume-local/nix/storage-json.nix
    ../packages/d2b-provider-volume-local/nix/zone-storage-json.nix
    ../packages/d2b-provider-volume-local/nix/sync-json.nix
    ./allocator-json.nix
    ./realm-controller-config-json.nix
    ./realm-identity-config-json.nix
    ./realm-workloads-launcher-json.nix
    ./realm-workloads-launcher-v2-json.nix
    ./unsafe-local-workloads-json.nix
    ./privileges-json.nix
    ./closures-json.nix
    ./minijail-profiles.nix
    ./ui-colors.nix
    ../packages/d2b-provider-display-wayland/nix/default.nix
    ../packages/d2b-provider-notification-desktop/nix/default.nix
    ../packages/d2b-provider-clipboard-wayland/nix/default.nix
    # Both cli.nix (bash CLI package) and host-ch-exporter.nix (host
    # singleton scraper folded into daemon /metrics) are now retired.
    # See tests/cli-nix-consumers-eval.sh + tests/legacy-unit-denylist-eval.sh
    # for the static gates.
    (import ./host-broker.nix { inherit inputs; })
    ../packages/d2b-provider-audio-pipewire/nix/host.nix
    ../packages/d2b-provider-audio-pipewire/nix/default.nix
    ./components/observability/default.nix
    ../packages/d2b-provider-display-wayland/nix/niri-vm-borders.nix
  ];

  # Entra ID / Himmelblau is NOT auto-imported here - it lives in
  # the sibling `vicondoa/entrablau.nix` flake. Consumers bring
  # it in per-VM
  #
  #   d2b.vms.<vm>.config.imports = [
  #     inputs.entrablau.nixosModules.default
  #   ];
  #
  # That keeps the himmelblau NixOS module out of d2b's eval
  # graph entirely.
}
