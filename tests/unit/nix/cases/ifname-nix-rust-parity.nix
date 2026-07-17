{ flakeRoot, lib, ... }:

let
  source = builtins.readFile
    (flakeRoot + "/nixos-modules/realm-network-rows.nix");
in
{
  "ifname-rendered-host-json/realm-ifname-prefix" = {
    expr = lib.hasInfix ''"d2b-''${tag}'' source;
    expected = true;
  };

  "ifname-rendered-host-json/realm-ifname-hash-width" = {
    expr =
      lib.hasInfix
        ''builtins.substring 0 8 (builtins.hashString "sha256" seed)''
        source;
    expected = true;
  };

  "ifname-rendered-host-json/bridge-tap-veth-role-tags" = {
    expr =
      lib.hasInfix ''ifName "b"'' source
      && lib.hasInfix ''ifName "t"'' source
      && lib.hasInfix ''ifName "v"'' source;
    expected = true;
  };
}
