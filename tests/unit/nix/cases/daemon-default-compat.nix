{ lib, flakeRoot, ... }:

let
  daemon = builtins.readFile
    (flakeRoot + "/nixos-modules/host-daemon.nix");
  broker = builtins.readFile
    (flakeRoot + "/nixos-modules/host-broker.nix");
in
{
  "local-root-endpoints/controller-not-compat-gated" = {
    expr = !(lib.hasInfix "daemonExperimental" daemon);
    expected = true;
  };

  "local-root-endpoints/broker-not-compat-gated" = {
    expr = !(lib.hasInfix "daemonExperimental" broker);
    expected = true;
  };

  "local-root-endpoints/no-realm-unit-generation" = {
    expr =
      !(lib.hasInfix "_index" daemon)
      && !(lib.hasInfix "_index" broker)
      && !(lib.hasInfix "realmDaemonServices" daemon)
      && !(lib.hasInfix "realmBrokerServices" broker);
    expected = true;
  };
}
