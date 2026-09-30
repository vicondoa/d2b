{ lib, pkgs, system, nixpkgs, inputs, d2bModule, d2bLib, flakeRoot, modules }:

import ../helpers/surface.nix {
  inherit lib pkgs system nixpkgs inputs d2bModule d2bLib flakeRoot modules;
  name = "declaration-projection";
  caseFiles = [{
    path = ../cases/declaration-projection.nix;
    names = [
      "declaration-projection/consumer-request-shorthand-is-canonical-and-byte-stable"
      "declaration-projection/duplicate-consumer-slot-refuses"
      "declaration-projection/binding-request-outside-the-source-policy-refuses"
      "declaration-projection/presentation-capability-is-the-projected-one"
      "declaration-projection/executable-digest-mismatch-refuses"
      "declaration-projection/declaration-projection-carries-no-signing-or-inference-field"
      "declaration-projection/catalog-agreement-refuses-a-row-no-declaration-produces"
    ];
  }];
}
