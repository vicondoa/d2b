{ root
, specPath
}:

let
  spec = builtins.fromJSON (builtins.readFile specPath);
  lock = builtins.fromJSON (builtins.readFile (root + "/flake.lock"));
  nixpkgsInput = lock.nodes.root.inputs.nixpkgs;
  nixpkgsNode =
    if builtins.isString nixpkgsInput
    then nixpkgsInput
    else builtins.elemAt nixpkgsInput ((builtins.length nixpkgsInput) - 1);
  nixpkgs = builtins.fetchTree lock.nodes.${nixpkgsNode}.locked;
  inputs = { inherit nixpkgs; };
  system = spec.system;
  pkgs = import nixpkgs.outPath { inherit system; };
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
