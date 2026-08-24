{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "component-session";
  caseFiles = [{
    path = ../cases/guest-component-session.nix;
    names = [
      "guest-component-session/starts-d2bd-guest"
      "guest-component-session/uses-guest-broker-and-no-public-socket"
      "guest-component-session/binds-enrollment-inputs-at-start"
      "guest-component-session/does-not-install-retired-guest-agent"
    ];
  } {
    path = ../cases/gateway-component-session.nix;
    names = [
      "gateway-component-session/uses-separate-daemon-modes"
      "gateway-component-session/uses-separate-broker-profiles-and-sockets"
    ];
  }];
}
