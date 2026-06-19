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

  # Internal: populated by network.nix from the resolved
  # nixling.envs config. host.nix reads it to derive
  # workload-VM tap names, MACs, IPs, USBIP host IP, etc. Don't set
  # this manually.
  options.nixling._envMeta = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    internal = true;
    description = "Internal: per-env computed metadata (set by network.nix).";
  };

  # ---------------------------------------------------------------------------
  # the following internal options were
  # retired together with the bash CLI consumer surface
  #
  #   nixling.cliBin             — set by cli.nix to point at the
  #                                bash CLI; consumed by
  #                                host-audit.nix (deleted in the
  #                                same commit; the audit-check
  #                                service is on the  denylist).
  #   nixling.audioStateHelperPath — set by cli.nix to point at
  #                                nixling-read-audio-state.sh; the
  #                                only consumer (tests/integration/live/audio.sh)
  #                                now discovers the helper at the
  #                                daemon-managed path.
  #   nixling._desktopWrappers   — set by cli.nix to pin the per-VM
  #.desktop launcher contract. The
  #                                daemon-native launcher module
  #                                will re-introduce this option
  #                                when it lands; until then no
  #                                graphics VM gets a .desktop
  #                                wrapper through the framework.
  # ---------------------------------------------------------------------------
}
