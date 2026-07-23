# nix-unit cases migrated from tests/bridge-ipv6-boot-sysctl-eval.sh
# (group E).
#
# Every bridge declared by the multi-env consumer example (br-work-lan,
# br-work-up, br-personal-lan, br-personal-up) MUST get a
# `boot.kernel.sysctl."net.ipv6.conf.<bridge>.<knob>"` entry in the
# rendered NixOS config: disable_ipv6 = 1, accept_ra = 0, autoconf = 0.
#
# This pins the invariant that NixOS activation applies bridge IPv6
# suppression declaratively BEFORE any d2bd/broker invocation, closing
# the boot-time window where bridges had IPv6 active until the first VM in
# the env started.
#
# Uses `mkEval` to render real `config.boot.kernel.sysctl`; the config
# mirrors examples/multi-env/configuration.nix (work + personal envs, no
# graphics/audio, so it evaluates on any host platform).
{ mkEval, ... }:

let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = { launcherUsers = [ "alice" ]; yubikey.enable = false; };
    d2b.hostLanCidrs = [ "192.168.1.0/24" ];
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.envs.personal = {
      lanSubnet = "10.30.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
    };
    d2b.vms.work-app = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "work-app";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.vms.personal-app = {
      enable = true;
      env = "personal";
      index = 10;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "personal-app";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  sysctl = (mkEval [ base ]).config.boot.kernel.sysctl;
  keyOf = bridge: knob: sysctl."net.ipv6.conf.${bridge}.${knob}" or null;
in
{
  "bridge-ipv6-boot-sysctl/br-work-lan-disable-ipv6" = {
    expr = keyOf "br-work-lan" "disable_ipv6";
    expected = 1;
  };
  "bridge-ipv6-boot-sysctl/br-work-lan-accept-ra" = {
    expr = keyOf "br-work-lan" "accept_ra";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-work-lan-autoconf" = {
    expr = keyOf "br-work-lan" "autoconf";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-work-up-disable-ipv6" = {
    expr = keyOf "br-work-up" "disable_ipv6";
    expected = 1;
  };
  "bridge-ipv6-boot-sysctl/br-work-up-accept-ra" = {
    expr = keyOf "br-work-up" "accept_ra";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-work-up-autoconf" = {
    expr = keyOf "br-work-up" "autoconf";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-personal-lan-disable-ipv6" = {
    expr = keyOf "br-personal-lan" "disable_ipv6";
    expected = 1;
  };
  "bridge-ipv6-boot-sysctl/br-personal-lan-accept-ra" = {
    expr = keyOf "br-personal-lan" "accept_ra";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-personal-lan-autoconf" = {
    expr = keyOf "br-personal-lan" "autoconf";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-personal-up-disable-ipv6" = {
    expr = keyOf "br-personal-up" "disable_ipv6";
    expected = 1;
  };
  "bridge-ipv6-boot-sysctl/br-personal-up-accept-ra" = {
    expr = keyOf "br-personal-up" "accept_ra";
    expected = 0;
  };
  "bridge-ipv6-boot-sysctl/br-personal-up-autoconf" = {
    expr = keyOf "br-personal-up" "autoconf";
    expected = 0;
  };
}
