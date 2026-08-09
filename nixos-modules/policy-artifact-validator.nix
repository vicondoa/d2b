{ lib }:

let
  getField = name: value:
    if builtins.isAttrs value && builtins.hasAttr name value
    then builtins.getAttr name value
    else null;

  uniqueList = values:
    builtins.isList values
    && builtins.length values == builtins.length (lib.unique values);

  exactSet = left: right:
    builtins.isList left
    && builtins.isList right
    && uniqueList left
    && uniqueList right
    && lib.sort builtins.lessThan left == lib.sort builtins.lessThan right;

  identityKey = value:
    let
      source = getField "source" value;
    in
    if builtins.isAttrs value
      && builtins.isString (getField "name" value)
      && getField "name" value != ""
      && builtins.isString (getField "version" value)
      && getField "version" value != ""
      && (source == null || builtins.isString source)
    then
      "${getField "name" value}|${getField "version" value}|${if source == null then "" else source}"
    else null;

  identityKeys = values:
    if builtins.isList values then map identityKey values else [ ];

  validIdentityKeys = values:
    builtins.isList values
    && builtins.length values > 0
    && builtins.all (value: value != null) values
    && uniqueList values;

  packageForId = packages: id:
    lib.findFirst (package: getField "id" package == id) null packages;

  nodeForId = nodes: id:
    lib.findFirst (node: getField "id" node == id) null nodes;

  edgeKind = kind:
    let value = getField "kind" kind;
    in if value == null then "normal" else value;

  edgeKey = edge:
    let
      package = getField "pkg" edge;
      name = getField "name" edge;
      kinds = getField "dep_kinds" edge;
    in
    if builtins.isString package
      && builtins.isString name
      && builtins.isList kinds
    then "${package}|${name}|${builtins.toJSON kinds}"
    else null;

  lockPackageIdentities = lock:
    let packages = getField "package" lock;
    in
    if builtins.isList packages
    then map (package: {
      name = getField "name" package;
      version = getField "version" package;
      source = getField "source" package;
    }) packages
    else [ ];

  lockDependenciesClosed = lock:
    let
      packages = getField "package" lock;
      names =
        if builtins.isList packages
        then map (package: getField "name" package) packages
        else [ ];
      dependencyName = token:
        let parts = lib.splitString " " token;
        in if parts == [ ] then "" else builtins.head parts;
      packageOk = package:
        let dependencies = getField "dependencies" package;
        in
        dependencies == null
        || (builtins.isList dependencies
          && builtins.all (token:
            builtins.isString token
            && token != ""
            && builtins.elem (dependencyName token) names)
            dependencies);
    in
    builtins.isAttrs lock
    && builtins.isList packages
    && builtins.length packages > 0
    && builtins.all packageOk packages;

  lockMatches = lock: identities:
    let
      lockKeys = identityKeys (lockPackageIdentities lock);
      expected = identityKeys identities;
    in
    lockDependenciesClosed lock
    && validIdentityKeys lockKeys
    && validIdentityKeys expected
    && exactSet expected lockKeys;

  resolveNodeEdgesClosed = { node, nodeIds, packages, allowedKinds }:
    let
      dependencies = getField "dependencies" node;
      dependenciesOk =
        builtins.isList dependencies
        && builtins.all builtins.isString dependencies
        && uniqueList dependencies
        && builtins.all (id: builtins.elem id nodeIds) dependencies;
      details = getField "deps" node;
      detailIds =
        if builtins.isList details
        then map (detail: getField "pkg" detail) details
        else [ ];
      detailKeys =
        if builtins.isList details
        then map edgeKey details
        else [ ];
      detailsOk =
        builtins.isList details
        && builtins.all (detail:
          let
            packageId = getField "pkg" detail;
            packageName = getField "name" detail;
            kinds = getField "dep_kinds" detail;
            target = getField "target" detail;
            targetPackage = packageForId packages packageId;
          in
          builtins.isAttrs detail
          && builtins.isString packageId
          && builtins.elem packageId nodeIds
          && builtins.isString packageName
          && builtins.isAttrs targetPackage
          && packageName != ""
          && (target == null || builtins.isString target)
          && builtins.isList kinds
          && builtins.length kinds > 0
          && builtins.all (kind:
            builtins.isAttrs kind
            && (getField "kind" kind == null
              || builtins.isString (getField "kind" kind))
            && ((getField "target" kind) == null
              || builtins.isString (getField "target" kind))
            && builtins.elem (edgeKind kind) allowedKinds)
            kinds)
          details
        && builtins.all (key: key != null) detailKeys
        && uniqueList detailKeys
        && uniqueList detailIds
        && exactSet dependencies detailIds;
    in
    dependenciesOk && detailsOk;

  reachableNodeIds = nodes: seen: frontier:
    if frontier == [ ] then
      seen
    else
      let
        fresh = lib.filter (id: !(builtins.elem id seen)) frontier;
        next = lib.concatMap (id:
          let node = nodeForId nodes id;
          in if node == null then [ ] else getField "dependencies" node)
          fresh;
      in
      reachableNodeIds nodes (lib.unique (seen ++ fresh)) next;

  policyArtifactShapeOk =
    { artifact
    , lock
    , expected
    , variant
    , expectedEdgeKinds
    }:
    let
      packages = getField "packages" artifact;
      identities = getField "identities" artifact;
      resolve = getField "resolve" artifact;
      nodes =
        if builtins.isAttrs resolve
        then getField "nodes" resolve
        else null;
      packageIds =
        if builtins.isList packages
        then map (package: getField "id" package) packages
        else [ ];
      nodeIds =
        if builtins.isList nodes
        then map (node: getField "id" node) nodes
        else [ ];
      rootPackages =
        if builtins.isList packages
        then lib.filter
          (package: getField "name" package == expected.package)
          packages
        else [ ];
      resolveRoot =
        if builtins.isAttrs resolve
        then getField "root" resolve
        else null;
      rootNodes =
        if builtins.isList nodes
        then lib.filter (node: getField "id" node == resolveRoot) nodes
        else [ ];
      rootPackageId =
        if builtins.length rootPackages == 1
        then getField "id" (builtins.head rootPackages)
        else null;
      graphOk =
        builtins.isAttrs resolve
        && builtins.isList packages
        && builtins.length packages > 0
        && builtins.all (package:
          builtins.isAttrs package
          && identityKey package != null
          && builtins.isString (getField "id" package)
          && getField "id" package != "")
          packages
        && uniqueList packageIds
        && builtins.isList nodes
        && builtins.length nodes > 0
        && builtins.all (node:
          builtins.isAttrs node
          && builtins.isString (getField "id" node)
          && getField "id" node != "")
          nodes
        && uniqueList nodeIds
        && exactSet packageIds nodeIds
        && builtins.all (node:
          resolveNodeEdgesClosed {
            inherit node nodeIds packages;
            allowedKinds = lib.splitString "," expectedEdgeKinds;
          })
          nodes
        && builtins.isString resolveRoot
        && builtins.elem resolveRoot nodeIds
        && builtins.length rootPackages == 1
        && builtins.length rootNodes == 1
        && rootPackageId == resolveRoot
        && exactSet nodeIds (reachableNodeIds nodes [ ] [ resolveRoot ]);
    in
    builtins.isAttrs artifact
    && getField "system" artifact == expected.system
    && getField "target" artifact == expected.target
    && getField "package" artifact == expected.package
    && getField "root" artifact == expected.package
    && getField "variant" artifact == variant
    && getField "edgeKinds" artifact == expectedEdgeKinds
    && getField "defaultFeatures" artifact == (expected.defaultFeatures or false)
    && exactSet expected.features (getField "features" artifact)
    && builtins.isString (getField "sourceCensusSha256" artifact)
    && builtins.match "[0-9a-fA-F]{64}" (getField "sourceCensusSha256" artifact) != null
    && validIdentityKeys (identityKeys identities)
    && identitiesEqual identities packages
    && graphOk
    && lockMatches lock packages;

  identitiesEqual = left: right:
    let
      leftKeys = identityKeys left;
      rightKeys = identityKeys right;
    in
    validIdentityKeys leftKeys
    && validIdentityKeys rightKeys
    && exactSet leftKeys rightKeys;
in
{
  inherit policyArtifactShapeOk;
}
