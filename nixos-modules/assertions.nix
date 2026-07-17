{ config, lib, ... }:

let
  cfg = config.d2b;
  index = cfg._index;
  identity = import ./v2-identity.nix;
  modulePkgs =
    config._module.args.pkgs
      or (config._module.specialArgs.pkgs or null);
  platformSystem =
    if modulePkgs == null
    then null
    else modulePkgs.stdenv.hostPlatform.system;
  waylandUser = lib.attrByPath [ "d2b" "site" "waylandUser" ] null config;
  declaredUsers = lib.attrByPath [ "users" "users" ] { } config;
  guestSessionRows = cfg._workloadGuestSessionCredentialRows or [ ];
  guestSessionRequiredFields = [
    "sessionGeneration"
    "parentPublicKey"
    "channelBinding"
    "guestIdentity"
    "guestPublicKey"
  ];
  guestSessionOperationPskBindingFields = [
    "operationId"
    "realmId"
    "workloadId"
    "controllerGeneration"
    "workloadGeneration"
    "runtimeInstanceHandleDigest"
    "transportEndpointDigest"
    "purpose"
    "expiresAtUnixMs"
    "replayNonce"
  ];

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

  workloadFeatureAssertions = lib.concatMap
    (workload:
      let
        spec = workload.spec;
        graphics = lib.attrByPath [ "graphics" "enable" ] false spec;
        video = lib.attrByPath [ "graphics" "videoSidecar" ] false spec;
        nvidiaVideo =
          lib.attrByPath [ "graphics" "videoNvidiaDecode" ] false spec;
        audio = lib.attrByPath [ "audio" "enable" ] false spec;
        wayland = lib.attrByPath [ "display" "wayland" ] false spec;
        device =
          lib.attrByPath [ "tpm" "enable" ] false spec
          || graphics
          || lib.attrByPath [ "usbip" "enable" ] false spec
          || lib.attrByPath [ "securityKey" "enable" ] false spec;
        needsDesktop = graphics || audio || wayland;
        hasBinding = authority:
          (workload.providerBindings.${authority} or null) != null;
      in
      [
        {
          assertion =
            !(graphics || audio)
            || platformSystem == null
            || platformSystem == "x86_64-linux";
          message =
            "Workload ${workload.canonicalTarget}: graphics/audio components "
            + "are supported only on x86_64-linux.";
        }
        {
          assertion =
            !needsDesktop
            || !(config.d2b ? site)
            || waylandUser != null;
          message =
            "Workload ${workload.canonicalTarget} requires "
            + "d2b.site.waylandUser for graphics, audio, or Wayland display.";
        }
        {
          assertion =
            !needsDesktop
            || !(config.d2b ? site)
            || (waylandUser != null
              && builtins.hasAttr waylandUser declaredUsers);
          message =
            "Workload ${workload.canonicalTarget} requires its "
            + "d2b.site.waylandUser to name a declared host user.";
        }
        {
          assertion =
            !(graphics || audio)
            || !(workload.spec.autostart or false);
          message =
            "Workload ${workload.canonicalTarget}: graphics/audio mediation "
            + "is incompatible with autostart.";
        }
        {
          assertion = !video || graphics;
          message =
            "Workload ${workload.canonicalTarget}: video mediation requires graphics.enable.";
        }
        {
          assertion = !nvidiaVideo || video;
          message =
            "Workload ${workload.canonicalTarget}: NVIDIA video decode requires videoSidecar.";
        }
        {
          assertion = !device || hasBinding "device";
          message =
            "Workload ${workload.canonicalTarget}: TPM, graphics, USBIP, and "
            + "security-key features require an explicit device provider binding.";
        }
        {
          assertion = !audio || hasBinding "audio";
          message =
            "Workload ${workload.canonicalTarget}: audio requires an explicit "
            + "audio provider binding.";
        }
        {
          assertion = !wayland || hasBinding "display";
          message =
            "Workload ${workload.canonicalTarget}: Wayland display requires an "
            + "explicit display provider binding.";
        }
      ])
    index.workloads.enabledList;

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

  guestSessionAssertions = [
    {
      assertion = lib.all
        (row:
          row.format == "GuestSessionCredentialV1"
          && row.schemaVersion == 1
          && row.encoding == "d2b-guest-session-v2"
          && row.payloadContract.requiredFields == guestSessionRequiredFields
          && row.payloadContract.forbiddenFields
            == [ "parentPrivateKey" "guestPrivateKey" ]
          && row.payloadContract.optionalOperationPskFields
            == [ "binding" "secret" ]
          && row.payloadContract.operationPskBindingFields
            == guestSessionOperationPskBindingFields
          && row.payloadContract.operationPskAllOrNone
          && row.payloadContract.operationPskSingleUse)
        guestSessionRows;
      message = "Guest session credentials must bind the exact generation, channel, parent, and guest public identities without private keys.";
    }
    {
      assertion = lib.all
        (row:
          lib.hasPrefix
            "/run/d2b/r/${row.realmId}/w/${row.workloadId}/guest-session/"
            row.target
          && !(lib.hasPrefix "/nix/store" row.target)
          && row.owner == "root"
          && row.mode == "0440"
          && row.guestDelivery.owner == "root"
          && row.guestDelivery.mode == "0400"
          && !row.guestDelivery.ambientFallback
          && row.hostMaterialization.mechanism
            == "authenticated-component-session-fd"
          && row.hostMaterialization.service == "d2b.broker.v2"
          && row.hostMaterialization.method == "Apply"
          && row.hostMaterialization.methodId == 2253834528
          && row.hostMaterialization.attachmentCount == 1
          && row.hostMaterialization.attachmentKind == "file-descriptor"
          && row.hostMaterialization.descriptor == "memfd"
          && row.hostMaterialization.access == "read-only"
          && row.hostMaterialization.purpose == "request-input"
          && row.hostMaterialization.sealedRequired
          && row.hostMaterialization.cloexecRequired
          && row.hostMaterialization.exactStorageRefRequired
          && !row.hostMaterialization.pathPayloadAllowed
          && !row.hostMaterialization.ambientFallback
          && !row.bundleArtifact
          && !row.derivationMaterial
          && !row.observability.logsCredential
          && !row.observability.auditsCredential
          && !row.observability.metricsCredential
          && !row.materializedByHostActivation)
        guestSessionRows;
      message = "Guest session credentials must remain private runtime material with no store, bundle, activation, or ambient delivery path.";
    }
    {
      assertion = lib.all
        (row:
          row.authority.generation == "d2bd-r-${row.realmId}"
          && row.authority.materialization == "d2bbr-r-${row.realmId}"
          && row.authority.workloadId == row.workloadId
          && row.lifecycle.restart == "rotate-before-publish"
          && row.lifecycle.adoption == "exact-binding-or-quarantine"
          && row.lifecycle.stale == "fail-closed"
          && row.lifecycle.ambiguous == "fail-closed")
        guestSessionRows;
      message = "Guest session credential rotation and adoption must remain confined to the owning child realm and fail closed.";
    }
  ];
in
{
  assertions =
    [ identityInventoryAssertion ]
    ++ parentAssertions
    ++ parentCycleAssertions
    ++ providerBindingAssertions
    ++ providerImplementationAssertions
    ++ workloadFeatureAssertions
    ++ pathAssertions
    ++ guestSessionAssertions;
}
