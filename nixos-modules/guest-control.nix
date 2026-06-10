{ nixlingInputs, pkgs, ... }:

let
  guestPackages = nixlingInputs.self.packages.${pkgs.stdenv.hostPlatform.system};
in
{
  environment.systemPackages = [
    guestPackages.nixling-guestd-static
    guestPackages.nixling-userd-static
    guestPackages.nixling-exec-runner-static
  ];
}
