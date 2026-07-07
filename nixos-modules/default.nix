# d2b: a generic framework for declaring microVMs on this host.
#
# This module is the public entry point — pulled in by
# `nixosModules.default = import ./nixos-modules { inherit inputs; }`
# in the flake. The closure-passed `inputs` argument lets each
# sub-module that needs flake inputs get them via partial
# application instead of through `_module.args.inputs`. The latter
# infinite-recurses on host.nix's import-list resolution; the
# deep-eval regression test in the smoke flake covers this wiring.
#
# Sub-modules consuming `inputs`
#   * `host.nix` — `imports = [ inputs.microvm.nixosModules.host ]`
#     (the original case the partial-application wiring was built
#     for).
#   * `components/home-manager.nix` — `imports =
#     [ inputs.home-manager.nixosModules.home-manager ]`. Imported
#     conditionally by host.nix per-VM when `homeManager.enable =
#     true`; the partial application flows through there.
#
# Components live in sibling files (components/graphics.nix,
# components/audit.nix, etc.) and are conditionally imported per-VM
# by host.nix.
{ inputs }:

{ ... }:

{
  imports = [
    ./options.nix
    ./bundle-artifacts.nix
    ./options-observability.nix
    ./options-ownership-matrix.nix
    ./index.nix
    ./assertions.nix
    ./network.nix
    ./gateway-vm.nix
    (import ./host.nix { inherit inputs; })
    # host-otel-relay-acl.nix retired per ADR 0018.
    # The OTel host-bridge + per-VM relay ACL contract moved into the
    # broker pre-spawn pipeline (`SpawnRunner{role: OtelHostBridge}`
    # in `packages/d2b-priv-broker/src/runtime.rs`). The retired
    # module file is kept as a stub for one release for diff
    # readability; consumers should not import it directly. A future
    # commit deletes the stub file outright.
    # ./host-otel-relay-acl.nix
    # ./vms.nix is INTENTIONALLY OMITTED from the public flake — VM
    # registrations are consumer-specific. Downstream users declare
    # their VMs via `d2b.vms.<name> = ...` in their own NixOS
    # module, which is merged into d2b.vms here via option-system
    # semantics. There is no public file with example VMs (yet —
    # examples/ will demonstrate the pattern).
    ./observability-vm.nix
    ./clipboard.nix
    ./notifications.nix
    ./store.nix
    ./manifest.nix
    ./bundle.nix
    ./guest-control-host.nix
    ./host-json.nix
    ./processes-json.nix
    ./storage-json.nix
    ./sync-json.nix
    ./allocator-json.nix
    ./realm-controller-config-json.nix
    ./privileges-json.nix
    ./closures-json.nix
    ./minijail-profiles.nix
    ./ui-colors.nix
    # Both cli.nix (bash CLI package) and host-ch-exporter.nix (host
    # singleton scraper folded into daemon /metrics) are now retired.
    # See tests/cli-nix-consumers-eval.sh + tests/legacy-unit-denylist-eval.sh
    # for the static gates.
    (import ./host-broker.nix { inherit inputs; })
    ./components/audio/host.nix
    ./components/observability/default.nix
    ./niri-vm-borders.nix
  ];

  # Entra ID / Himmelblau is NOT auto-imported here — it lives in
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
