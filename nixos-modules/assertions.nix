{ config, lib, ... }:

let
  cfg = config.d2b;
  index = cfg._index;
  identity = import ./v2-identity.nix;

  parentAssertions = map
    (realm: {
      assertion =
        realm.parentPath == null
        || realm.parentPath == "local-root"
        || builtins.hasAttr realm.parentPath index.realms.byPath;
      message = "Realm ${realm.realmPath} refers to undeclared parent ${toString realm.parentPath}.";
    })
    index.realms.enabledList;

  parentCycle = start:
    let
      walk = path: seen:
        if path == null || !(builtins.hasAttr path index.realms.enabledByPath)
        then false
        else if builtins.elem path seen
        then true
        else walk index.realms.enabledByPath.${path}.parentPath (seen ++ [ path ]);
    in
    walk start [ ];

  parentCycleAssertions = map
    (realm: {
      assertion = !(parentCycle realm.realmPath);
      message = "Realm parent graph contains a cycle at ${realm.realmPath}.";
    })
    index.realms.enabledList;

  providerBindingAssertions = lib.concatMap
    (workload:
      lib.mapAttrsToList
        (providerType: providerRef: {
          assertion =
            (workload.providerBindings.${providerType} or null) != null;
          message = "Workload ${workload.canonicalTarget} selects undeclared ${providerType} provider ${providerRef}.";
        })
        workload.providerRefs)
    index.workloads.enabledList;

  providerImplementationAssertions = map
    (provider: {
      assertion = provider.implementationId != null;
      message = "Provider ${provider.providerId} must select a canonical implementation.";
    })
    index.providers.enabledList;

  rawRuntimeComponents =
    (map (realm: realm.metadata.configuredId) index.realms.list)
    ++ (map (workload: workload.configuredName) index.workloads.list)
    ++ (lib.concatMap
      (provider: [ provider.providerName provider.configuredProviderId ])
      index.providers.list);
  canonicalComponents =
    index.identities.realmIds
    ++ index.identities.workloadIds
    ++ index.identities.providerIds
    ++ index.identities.roleIds;
  forbiddenComponents = lib.filter
    (component: !(builtins.elem component canonicalComponents))
    (lib.unique rawRuntimeComponents);

  pathUsesRawComponent = path:
    let components = lib.filter (component: component != "") (lib.splitString "/" path);
    in lib.any (component: builtins.elem component forbiddenComponents) components;

  pathAssertions = map
    (resource: {
      assertion =
        resource.path == null
        || (!pathUsesRawComponent resource.path
          && (builtins.tryEval (identity.unixPathHeadroom resource.path)).success);
      message = "Resource ${resource.resourceId} uses a raw identifier or invalid Unix runtime path.";
    })
    index.resources.list;

  identityInventoryAssertion = {
    assertion = builtins.deepSeq index.identities true;
    message = "Normalized realm identity inventory is invalid.";
  };
in
{
  assertions =
    [ identityInventoryAssertion ]
    ++ parentAssertions
    ++ parentCycleAssertions
    ++ providerBindingAssertions
    ++ providerImplementationAssertions
    ++ pathAssertions;
}
