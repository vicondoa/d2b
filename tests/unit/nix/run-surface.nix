{ root
, specPath
, nixpkgsPath
}:

let
  spec = builtins.fromJSON (builtins.readFile specPath);
  system = spec.system;
  pkgs = import nixpkgsPath { inherit system; };
  nixpkgs = { outPath = pkgs.path; };
  inputs = { inherit nixpkgs; };
  modules = map
    (path: root + "/${path}")
    spec.modules;
  surface = import (root + "/${spec.surface}") {
    inherit inputs modules nixpkgs pkgs system;
    inherit (pkgs) lib;
    d2bModule = { };
    d2bLib = import (root + "/nixos-modules/lib.nix") {
      inherit (pkgs) lib;
    };
    flakeRoot = root;
  };
  evaluator = import ./eval-jobs.nix {
    inherit (pkgs) lib;
    inherit pkgs system;
  };
in
(evaluator.evalSurface {
  name = spec.name;
  cases = surface.cases;
}).message + "\n"
