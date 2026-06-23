# nixling option schema.
#
# Per-VM declarations live under `nixling.vms.<name>`. Component
# toggles (graphics.enable / tpm.enable / usbip.* / audio.* /
# audit.*) are defined here on the same submodule; the matching
# component file
# under `nixos-modules/components/` is conditionally imported by
# host.nix.
#
# Isolated environments live under `nixling.envs.<env>`. Each env
# is materialised by network.nix into two host bridges (`br-<env>-up`
# point-to-point host↔net-VM, `br-<env>-lan` net-VM↔workload-VMs),
# an auto-generated headless net VM (`sys-<env>-net`), NAT/firewall,
# and a per-env `nixling-sys-<env>-usbipd-proxy` instance. Workload
# VMs join an env by setting `nixling.vms.<name>.env = "<env>"` and
# `index = <N>`.
{ lib, ... }:

{
  imports = [
    ./options-site.nix
    ./options-envs.nix
    ./options-vms.nix
    ./options-daemon.nix
    ./options-gateway.nix
  ];

  # Internal compatibility alias for nixling._index.envMeta. host.nix
  # reads it to derive workload-VM tap names, MACs, IPs, USBIP host
  # IP, etc. Don't set this manually.
  options.nixling._envMeta = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    internal = true;
    description = "Internal: per-env computed metadata aliasing nixling._index.envMeta.";
  };

}
