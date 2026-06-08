# nixling option schema.
#
# Per-VM declarations live under `nixling.vms.<name>`. Component
# toggles (graphics.enable / tpm.enable / usbip.* / audio.*) are
# defined here on the same submodule; the matching component file
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
  ];

  # Internal: store path of the nixling-read-audio-state.sh helper.
  # Baked in by cli.nix so tests can resolve it via nix eval instead of
  # scanning the Nix store (which may find a stale previous generation).
  options.nixling.audioStateHelperPath = lib.mkOption {
    type = lib.types.str;
    default = "";
    internal = true;
    description = "Store path of nixling-read-audio-state.sh (set by cli.nix).";
  };

  # Internal: absolute path to the 'nixling' CLI binary for the current
  # generation, baked in by cli.nix. Used by host.nix to reference the
  # nixling binary in systemd ExecStart lines without relying on PATH.
  options.nixling.cliBin = lib.mkOption {
    type = lib.types.str;
    default = "";
    internal = true;
    description = "Absolute path to the nixling binary (set by cli.nix).";
  };

  # Internal: populated by network.nix from the resolved
  # nixling.envs config. host.nix and cli.nix read it to derive
  # workload-VM tap names, MACs, IPs, USBIP host IP, etc. Don't set
  # this manually.
  options.nixling._envMeta = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    internal = true;
    description = "Internal: per-env computed metadata (set by network.nix).";
  };
}
