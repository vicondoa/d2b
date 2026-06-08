# examples/multi-env/configuration.nix
#
# Two isolated envs (`work` + `personal`), one headless workload VM
# in each. The goal is to make per-env network isolation observable:
# `work-app` and `personal-app` get distinct LAN bridges, distinct
# net VMs, distinct dnsmasq pools, and distinct usbipd-proxy
# backends. They cannot reach each other; the host can reach both.
#
# See ./README.md for the topology diagram and the rationale.
{ lib, ... }:

{
  # ---------------------------------------------------------------
  # Minimal NixOS baseline so `system.build.toplevel` is reachable.
  # A real consumer would import their own hardware-configuration.nix
  # and bootloader settings instead of this stub.
  # ---------------------------------------------------------------
  boot.loader.grub.enable = false;
  boot.loader.systemd-boot.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text = "00000000000000000000000000000000";
  system.stateVersion = "25.11";

  # One consumer-side user. Both VMs SSH in as this account; the
  # framework generates one Ed25519 key per VM under
  # `nixling.site.keysDir` (default `/var/lib/nixling/keys`).
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
  };

  # ---------------------------------------------------------------
  # Site-level knobs.
  # ---------------------------------------------------------------
  nixling.site = {
    launcherUsers = [ "alice" ];
    # Headless example — no Wayland forwarding, no host-side Yubikey
    # udev rules. Flip these on for a graphics + USBIP setup.
    yubikey.enable = false;
  };

  # CIDRs of the host's primary LAN(s). Auto-merged into EVERY env's
  # net-VM DROP rule, so VMs cannot reach the host's neighbours
  # (printer, NAS, other workstations) — not just the host's own IP.
  # Replace with whatever `ip route` says is your physical LAN.
  nixling.hostLanCidrs = [
    "192.168.1.0/24"
  ];

  # ---------------------------------------------------------------
  # Env 1: work.
  #
  # Materialises:
  #   br-work-up  (192.0.2.0/30)   host ↔ sys-work-net point-to-point
  #   br-work-lan (10.20.0.0/24)   sys-work-net + workload VMs
  #   sys-work-net VM (NAT + dnsmasq + nftables, autostarts)
  #   nixling-sys-work-usbipd-proxy.service bound to 192.0.2.1:3240
  # ---------------------------------------------------------------
  nixling.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";

    # `extraNetConfig` is the per-env escape hatch — see the README's
    # "extraNetConfig: when and when not" section. Empty here so the
    # example documents the option without changing behaviour.
    extraNetConfig = { };
  };

  # ---------------------------------------------------------------
  # Env 2: personal.
  #
  # Disjoint CIDRs from work — both LAN and uplink. The whole point
  # of this example is that these two envs share NOTHING at the
  # network layer.
  # ---------------------------------------------------------------
  nixling.envs.personal = {
    lanSubnet    = "10.30.0.0/24";
    uplinkSubnet = "192.0.2.4/30";
  };

  # ---------------------------------------------------------------
  # Workload VM #1 — joins `work`. Index 10 → IP 10.20.0.10.
  # ---------------------------------------------------------------
  nixling.vms.work-app = {
    enable = true;
    env = "work";
    index = 10;
    ssh.user = "alice";
    config = {
      networking.hostName = lib.mkDefault "work-app";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
    };
  };

  # ---------------------------------------------------------------
  # Workload VM #2 — joins `personal`. Index 10 → IP 10.30.0.10.
  # Same `index` as work-app is fine: index uniqueness is scoped
  # per-env.
  # ---------------------------------------------------------------
  nixling.vms.personal-app = {
    enable = true;
    env = "personal";
    index = 10;
    ssh.user = "alice";
    config = {
      networking.hostName = lib.mkDefault "personal-app";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
    };
  };
}
