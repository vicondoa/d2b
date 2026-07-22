{ branch
, pathsJson
, plan ? import ./w8-integration-wave-plan.nix { }
}:

let
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
    unique
      (plan.globalExternalDependencies
        ++ componentExternalDependencies
        ++ pathExternalDependencies);
  blockedExternalDependencies = builtins.filter
    (dependency:
      !(builtins.hasAttr dependency plan.externalDependencies)
      || plan.externalDependencies.${dependency}.status != "ready")
    externalDependencies;

  unmetDependencies =
    if component == null
    then [ ]
    else builtins.filter
      (dependency: !(builtins.hasAttr dependency plan.landedComponents))
      plan.components.${component}.dependsOn;
  landedDependencyCommits = builtins.map
    (dependency: {
      inherit dependency;
      commit = plan.landedComponents.${dependency};
    })
    (if component == null
     then [ ]
     else builtins.filter
       (dependency: builtins.hasAttr dependency plan.landedComponents)
       plan.components.${component}.dependsOn);
  invalidLandedDependencies = builtins.filter
    (row:
      !(builtins.isString row.commit)
      || builtins.match "[0-9a-f]{40}" row.commit == null)
    landedDependencyCommits;

  hasPrefix = prefix: value:
    builtins.stringLength value >= builtins.stringLength prefix
    && builtins.substring 0 (builtins.stringLength prefix) value == prefix;

  ownedFiles =
    if component == null
    then [ ]
    else plan.components.${component}.ownedFiles;
  forbiddenEditExceptions =
    if component == null
    then [ ]
    else plan.components.${component}.forbiddenEditExceptions or [ ];
  invalidForbiddenEditExceptions = builtins.filter
    (path:
      !(builtins.elem path ownedFiles)
      || !(builtins.any
        (forbidden: path == forbidden || hasPrefix forbidden path)
        plan.forbiddenEdits))
    forbiddenEditExceptions;
  forbiddenViolations = builtins.filter
    (path:
      !(builtins.elem path forbiddenEditExceptions)
      && builtins.any
        (forbidden: path == forbidden || hasPrefix forbidden path)
        plan.forbiddenEdits)
    paths;
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
    forbiddenViolations
    invalidForbiddenEditExceptions
    invalidLandedDependencies
    landedDependencyCommits
    ownedFiles
    paths
    unmetDependencies
    violations
    ;
  valid =
    component != null
    && validPathList
    && violations == [ ]
    && forbiddenViolations == [ ]
    && invalidForbiddenEditExceptions == [ ]
    && blockedExternalDependencies == [ ]
    && invalidLandedDependencies == [ ]
    && unmetDependencies == [ ];
}
