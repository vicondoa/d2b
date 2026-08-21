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
  selectedCases = map
    (spec:
      let
        path =
          if builtins.typeOf spec == "path"
          then spec
          else spec.path;
        imported = import path context;
      in
      if builtins.typeOf spec == "path" || !(spec ? names) then
        imported
      else
        lib.filterAttrs (name: _: builtins.elem name spec.names) imported)
    caseFiles;
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
    ./surface.nix
  ];
  inherit cases;
}
