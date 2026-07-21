{ lib, flakeRoot, ... }:

let
  linesOf = path:
    lib.splitString "\n" (builtins.readFile (flakeRoot + path));
  broker = linesOf "/nixos-modules/host-broker.nix";
  daemon = linesOf "/nixos-modules/host-daemon.nix";
  has = lines: needle:
    lib.any (line: lib.hasInfix needle line) lines;
in
{
  "broker-bundle-path/canonical-fallback" = {
    expr =
      has broker ''cfg.site.bundle.currentManifest or "/etc/d2b/bundle.json"'';
    expected = true;
  };

  "broker-bundle-path/broker-flag" = {
    expr = has broker "--bundle-path";
    expected = true;
  };

  "broker-bundle-path/daemon-config" = {
    expr = has daemon ''bundlePath = "/etc/d2b/bundle.json"'';
    expected = true;
  };
}
