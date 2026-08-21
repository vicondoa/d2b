{ lib
, pkgs
, system
, nixpkgs
, inputs
, d2bModule
, d2bLib
, flakeRoot
, caseFiles
, modules
, moduleFixtures ? [ ]
, fixtures ? [ ]
, name
, pins ? [ ]
}:

let
  moduleValues = map
    (path:
      let
        module = import path;
      in
      if builtins.isFunction module
        && builtins.hasAttr "inputs" (builtins.functionArgs module)
      then
        module { inherit inputs; }
      else if builtins.isFunction module
        && builtins.hasAttr "packageFor" (builtins.functionArgs module)
      then
        module { }
      else
        module)
    modules;
  context = import ./eval.nix {
    inherit lib pkgs system nixpkgs d2bLib flakeRoot;
    d2bModule = { imports = moduleValues; };
    inherit moduleFixtures;
  };
  moduleEvaluation = context.mkModuleEval [ ];
  caseSelection = import ./select-cases.nix {
    inherit lib context;
    surfaceName = name;
  };
  selectedCases = caseSelection.selectCaseFiles caseFiles;
  cases = (import ../default.nix { cases = selectedCases; }) // {
    "${name}/modules-evaluate" = {
      # This shared smoke case forces only the declared module structure.
      # Owner-local cases force behavior without importing unrelated surfaces.
      expr =
        builtins.deepSeq
          (builtins.attrNames moduleEvaluation.options)
          (builtins.isAttrs moduleEvaluation.config);
      expected = true;
      propagateError = true;
    };
  };
in
{
  inherit modules fixtures pins;
  helpers = [
    ./eval.nix
    ./select-cases.nix
    ./surface.nix
  ];
  inherit cases;
}
