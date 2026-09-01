{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../..)
}:

let
  inherit (pkgs) lib;
  expected = [
    flake.packages.${system}.d2bd-guest-static
    flake.packages.${system}.d2b-broker-guest-static
    flake.packages.${system}.d2b-guest-shell-runner-static
  ];
  guest = flake.lib.evalGuest {
    inherit system;
    name = "static-consumption";
    shellEnable = true;
  };
  actual = guest.config.environment.systemPackages;
  names = packages: map (package: package.pname or (lib.getName package)) packages;
in
assert lib.all (package: builtins.elem package actual) expected;
builtins.toJSON {
  expected = names expected;
  actual = names actual;
}
