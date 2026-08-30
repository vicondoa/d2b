# d2b: a Zone/resource framework for this host.
#
# This module is the public entry point - pulled in by
# `nixosModules.default = import ./nixos-modules { inherit inputs; }`
# in the flake. The closure-passed `inputs` argument remains available
# to host-tool package selection and consumer-supplied guest evaluators.
#
# Runtime effects are owned by Provider packages and the daemon/broker
# control plane. Nix emits semantic Zone bundles and host admission
# plumbing only.
{ inputs }:

{ config, lib, pkgs, ... }:

let
  d2bHostTools = import ./rust-host-tools.nix {
    inherit inputs lib pkgs;
  };
in
{
  _module.args.d2bHostTools = d2bHostTools;
  _module.args.d2bHostToolOverrides = lib.mkDefault null;

  imports = [
    ./options.nix
    ./host-generation-rebuild-ref.nix
    ../packages/d2b-provider-volume-local/nix/options-volumes.nix
    ./resources-zone-control.nix
    ./resource-compiler.nix
    ./bundle-artifacts.nix
    ./provider-catalog.nix
    ./provider-runtime-contracts.nix
    ../packages/d2b-provider-activation-nixos/nix/default.nix
    ../packages/d2b-provider-runtime-cloud-hypervisor/nix/default.nix
    ../packages/d2b-provider-runtime-qemu-media/nix/default.nix
    ../packages/d2b-provider-volume-virtiofs/nix/default.nix
    ../packages/d2b-provider-device-gpu/nix/default.nix
    ../packages/d2b-provider-device-security-key/nix/default.nix
    ../packages/d2b-provider-device-tpm/nix/default.nix
    ../packages/d2b-provider-device-usbip/nix/default.nix
    ../packages/d2b-provider-audio-pipewire/nix/default.nix
    ../packages/d2b-provider-clipboard-wayland/nix/default.nix
    ../packages/d2b-provider-display-wayland/nix/default.nix
    ../packages/d2b-provider-notification-desktop/nix/default.nix
    ./providers/system-minijail.nix
    ./providers/system-systemd.nix
    ./provider-projection-validate.nix
    ./index.nix
    ../packages/d2b-provider-network-local/nix/network.nix

    ./bundle.nix
    ./realm-workloads-launcher-v2-json.nix
    ./privileges-json.nix
    ../packages/d2b-provider-volume-local/nix/zone-storage-json.nix
    ./host-polkit.nix
    ./host-sccache.nix
    ./host-users.nix
    ./host-daemon.nix
    (import ./host-broker.nix { inherit inputs; })
  ];
}
