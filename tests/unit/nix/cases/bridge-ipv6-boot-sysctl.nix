{ flakeRoot, lib, ... }:

let
  source = builtins.readFile
    (flakeRoot + "/nixos-modules/realm-network-rows.nix");
in
{
  "bridge-ipv6-boot-sysctl/realm-links-disable-ipv6" = {
    expr =
      lib.hasInfix "ipv6Disabled = true;" source
      && lib.hasInfix "disable = true;" source;
    expected = true;
  };

  "bridge-ipv6-boot-sysctl/realm-links-disable-ra" = {
    expr = lib.hasInfix "acceptRa = false;" source;
    expected = true;
  };

  "bridge-ipv6-boot-sysctl/realm-links-disable-autoconf" = {
    expr = lib.hasInfix "autoconf = false;" source;
    expected = true;
  };
}
