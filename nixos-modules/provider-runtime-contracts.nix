# Provider-specific runtime and transport boundary assertions.
#
# These checks are intentionally eval-time: a malformed Provider placement,
# raw locator, or cross-boundary credential scope must fail before any bundle
# or process effect is emitted.
{ config, lib, ... }:

let
  cfg = config.d2b;

  parseRef = value:
    let parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in if builtins.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resourcesFor = zone:
    lib.mapAttrsToList
      (name: resource: {
        inherit name resource zone;
        path = "d2b.zones.${zone}.resources.${name}";
      })
      (cfg.zones.${zone}.resources or { });

  allResources = lib.concatMap resourcesFor (lib.attrNames cfg.zones);

  refType = value:
    let parsed = parseRef value;
    in if parsed == null then null else parsed.type;

  providerRows = lib.filter
    (row: row.resource.type == "Provider")
    allResources;

  guestRows = lib.filter
    (row: row.resource.type == "Guest")
    allResources;

  processRows = lib.filter
    (row: builtins.elem row.resource.type [ "Process" "EphemeralProcess" ])
    allResources;

  zoneLinkRows = lib.filter
    (row: row.resource.type == "ZoneLink")
    allResources;

  providerAssertions = row:
    let
      spec = row.resource.spec or { };
      providerConfig = spec.config or { };
      providerRef =
        if row.resource.type == "Provider"
        then "Provider/${row.name}"
        else spec.providerRef or "";
      credentialRefs =
        if providerRef == "Provider/runtime-azure-container-apps" then
          lib.filter (value: value != null) [
            (providerConfig.controlCredentialRef or null)
            (providerConfig.pullCredentialRef or null)
          ]
        else if providerRef == "Provider/runtime-azure-virtual-machine" then
          lib.filter (value: value != null) [
            (providerConfig.armCredentialRef or null)
          ]
        else
          [ ];
      gatewayRef =
        if providerRef == "Provider/runtime-azure-container-apps"
        then providerConfig.gatewayExecutionRef or null
        else providerConfig.controllerExecutionRef or null;
      credentialScopeMatches = credentialRef:
        let
          parsed = parseRef credentialRef;
          credential =
            if parsed != null && builtins.hasAttr parsed.name cfg.zones.${row.zone}.resources
            then cfg.zones.${row.zone}.resources.${parsed.name}
            else null;
        in
        builtins.isString credentialRef
        && parsed != null
        && parsed.type == "Credential"
        && credential != null
        && (credential.spec.scope.executionRef or null) == gatewayRef;
    in
      (if providerRef == "Provider/runtime-cloud-hypervisor" then [
        {
          assertion = refType (providerConfig.controllerExecutionRef or null) == "Host";
          message = "${row.path}.spec.config.controllerExecutionRef must resolve to Host.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/runtime-azure-virtual-machine" then [
        {
          assertion = refType (providerConfig.controllerExecutionRef or null) == "Guest";
          message = "${row.path}.spec.config.controllerExecutionRef must resolve to the gateway Guest.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/runtime-azure-container-apps" then [
        {
          assertion = refType (providerConfig.gatewayExecutionRef or null) == "Guest";
          message = "${row.path}.spec.config.gatewayExecutionRef must resolve to the gateway Guest.";
        }
        {
          assertion = lib.all credentialScopeMatches credentialRefs;
          message = "${row.path}.spec.config credential scopes must match gatewayExecutionRef.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/transport-azure-relay" then [
        {
          assertion = refType (providerConfig.executionRef or null) == "Guest";
          message = "${row.path}.spec.config.executionRef must resolve to a gateway Guest.";
        }
        {
          assertion = refType (providerConfig.networkRef or null) == "Network";
          message = "${row.path}.spec.config.networkRef must resolve to a Network.";
        }
      ] else [ ]);

  guestAssertions = row:
    let
      spec = row.resource.spec or { };
      providerRef = spec.providerRef or "";
      providerEnvelope = spec.provider or { };
      settings = providerEnvelope.settings or { };
      forbidden = [
        "hostPath"
        "socketPath"
        "rawCid"
        "cid"
        "tapName"
        "tapFd"
        "argv"
        "commandLine"
      ];
      forbiddenPresent = lib.any (key: builtins.hasAttr key settings) forbidden;
      forbiddenSpecPresent = lib.any (key: builtins.hasAttr key spec) forbidden;
      zoneShape = lib.any
        (key: builtins.hasAttr key spec)
        [ "parentZone" "childZone" "zoneLink" "routeCursor" "authority" ];
    in
      (if providerRef == "Provider/runtime-cloud-hypervisor" then [
        {
          assertion = (spec.systemArtifactId or null) != null;
          message = "${row.path}.spec.systemArtifactId is required for runtime-cloud-hypervisor.";
        }
        {
          assertion = !((settings.memoryShared or true) == false
            && (spec.volumeAttachmentDefaults or [ ]) != [ ]);
          message = "${row.path}: memoryShared=false is incompatible with virtiofs attachments.";
        }
      ] else [ ])
      ++ (if lib.elem providerRef [
        "Provider/runtime-cloud-hypervisor"
        "Provider/runtime-azure-container-apps"
        "Provider/runtime-azure-virtual-machine"
      ] then [
        {
          assertion = !forbiddenPresent && !forbiddenSpecPresent;
          message = "${row.path}.spec.provider.settings must not contain raw host locators or argv.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/runtime-azure-virtual-machine" then [
        {
          assertion = (spec.systemArtifactId or null) == null;
          message = "${row.path}.spec.systemArtifactId must be null for runtime-azure-virtual-machine.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/runtime-azure-container-apps" then [
        {
          assertion = !zoneShape;
          message = "${row.path}: aca-sandbox-is-not-zone.";
        }
      ] else [ ]);

  processAssertions = row:
    let spec = row.resource.spec or { };
    providerRef = spec.ownerProviderRef or spec.providerRef or "";
    isGatewayProvider = lib.elem providerRef [
      "Provider/runtime-azure-container-apps"
      "Provider/runtime-azure-virtual-machine"
    ];
    executionRef = spec.executionRef or null;
    in lib.optionals isGatewayProvider [
      {
        assertion = refType executionRef == "Guest";
        message = "${row.path}.spec.executionRef must be the configured gateway Guest; Host placement is forbidden.";
      }
    ];

  zoneLinkAssertions = row:
    let
      settings = row.resource.spec.transportSettings or { };
      forbidden = [
        "socketPath"
        "hostPath"
        "password"
        "token"
        "key"
        "credential"
        "credentialRef"
      ];
      secretKey = key:
        builtins.elem key forbidden
        || lib.hasSuffix "CredRef" key
        || lib.hasSuffix "Credential" key;
    in [
      {
        assertion = lib.all (key: !secretKey key) (lib.attrNames settings);
        message = "${row.path}.spec.transportSettings must not contain credential or locator fields.";
      }
    ];

  allAssertions =
    lib.concatMap providerAssertions providerRows
    ++ lib.concatMap guestAssertions guestRows
    ++ lib.concatMap processAssertions processRows
    ++ lib.concatMap zoneLinkAssertions zoneLinkRows;
in
{
  config.assertions = allAssertions;
}
