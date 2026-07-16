{ branch, pathsJson }:

let
  plan = import ./realm-host-wave-plan.nix;
  componentNames = builtins.attrNames plan.components;
  matchingComponents = builtins.filter
    (name: plan.components.${name}.branch == branch)
    componentNames;
  component =
    if builtins.length matchingComponents == 1
    then builtins.head matchingComponents
    else null;

  paths = builtins.fromJSON pathsJson;
  concatMap = f: values: builtins.concatLists (builtins.map f values);
  unique = values:
    builtins.foldl'
      (result: value:
        if builtins.elem value result then result else result ++ [ value ])
      [ ]
      values;
  splitPath = value:
    let
      match = builtins.match "([^/]*)/(.*)" value;
    in
    if match == null
    then [ value ]
    else [ (builtins.elemAt match 0) ] ++ splitPath (builtins.elemAt match 1);
  safePath = path:
    builtins.isString path
    && path != ""
    && builtins.substring 0 1 path != "/"
    && builtins.all
      (part: part != "" && part != "." && part != "..")
      (splitPath path);
  validPathList =
    builtins.isList paths
    && paths != [ ]
    && builtins.all safePath paths
    && builtins.length paths == builtins.length (unique paths);

  dependencyClosure = name:
    [ name ] ++ concatMap dependencyClosure plan.components.${name}.dependsOn;
  closure =
    if component == null
    then [ ]
    else unique (dependencyClosure component);
  componentExternalDependencies = unique (concatMap
    (name: plan.components.${name}.externalDependsOn)
    closure);
  pathExternalDependencies = unique (concatMap
    (row:
      if builtins.any (path: builtins.elem path row.paths) paths
      then [ row.dependency ]
      else [ ])
    plan.pathExternalDependencies);
  externalDependencies =
    unique (componentExternalDependencies ++ pathExternalDependencies);
  blockedExternalDependencies = builtins.filter
    (dependency:
      !(builtins.hasAttr dependency plan.externalDependencies)
      || plan.externalDependencies.${dependency}.status != "ready")
    externalDependencies;

  ownedFiles =
    if component == null
    then [ ]
    else plan.components.${component}.ownedFiles;
  violations = builtins.filter
    (path: !(builtins.elem path ownedFiles))
    paths;
in
{
  schemaVersion = 1;
  inherit
    blockedExternalDependencies
    branch
    component
    externalDependencies
    ownedFiles
    paths
    violations
    ;
  valid =
    component != null
    && validPathList
    && violations == [ ]
    && blockedExternalDependencies == [ ];
}
