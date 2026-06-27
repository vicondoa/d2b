# d2b option schema.
#
# Per-VM declarations live under `d2b.vms.<name>`. Component
# toggles (graphics.enable / tpm.enable / usbip.* / audio.* /
# audit.*) are defined here on the same submodule; the matching
# component file
# under `nixos-modules/components/` is conditionally imported by
# host.nix.
#
# Isolated environments live under `d2b.envs.<env>`. Each env
# is materialised by network.nix into two host bridges (`br-<env>-up`
# point-to-point host↔net-VM, `br-<env>-lan` net-VM↔workload-VMs),
# an auto-generated headless net VM (`sys-<env>-net`), NAT/firewall,
# and a per-env `d2b-sys-<env>-usbipd-proxy` instance. Workload
# VMs join an env by setting `d2b.vms.<name>.env = "<env>"` and
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

  # Internal compatibility alias for d2b._index.envMeta. host.nix
  # reads it to derive workload-VM tap names, MACs, IPs, USBIP host
  # IP, etc. Don't set this manually.
  options.d2b._envMeta = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    internal = true;
    description = "Internal: per-env computed metadata aliasing d2b._index.envMeta.";
  };

}
