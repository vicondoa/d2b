{ packageFor ? null }:

{ lib, pkgs, ... }:

let
  package =
    if packageFor != null then
      packageFor pkgs.system
    else if pkgs ? gasCityContributor then
      pkgs.gasCityContributor
    else
      throw ''
        services.gasCityContributor requires the flake-provided
        nixosModules.gasCityContributor or a pkgs.gasCityContributor package
      '';
in
{
  _module.args.gasCityContributorPackage = package;

  imports = [
    ./options.nix
    ./service.nix
    ./integrations.nix
    ./network.nix
  ];
}
