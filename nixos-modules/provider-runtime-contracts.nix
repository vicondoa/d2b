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

  resourceFor = row: value:
    let
      parsed = parseRef value;
      resources = cfg.zones.${row.zone}.resources or { };
    in
      if parsed != null && builtins.hasAttr parsed.name resources
      then resources.${parsed.name}
      else null;

  resolvesAs = row: expectedType: value:
    let
      parsed = parseRef value;
      resource = resourceFor row value;
    in
      builtins.isString value
      && parsed != null
      && parsed.type == expectedType
      && resource != null
      && resource.type == expectedType;

  runtimeProviderRefs = [
    "Provider/runtime-azure-container-apps"
    "Provider/runtime-azure-virtual-machine"
    "Provider/runtime-cloud-hypervisor"
    "Provider/transport-azure-relay"
  ];

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
          [
            (providerConfig.controlCredentialRef or null)
            (providerConfig.pullCredentialRef or null)
          ]
        else if providerRef == "Provider/runtime-azure-virtual-machine" then
          [
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
          credential = resourceFor row credentialRef;
        in
        resolvesAs row "Credential" credentialRef
        && ((credential.spec or { }).scope or { }).executionRef == gatewayRef;
    in
      (if providerRef == "Provider/runtime-cloud-hypervisor" then [
        {
          assertion = resolvesAs row "Host" (providerConfig.controllerExecutionRef or null);
          message = "${row.path}.spec.config.controllerExecutionRef must resolve to Host.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/runtime-azure-virtual-machine" then [
        {
          assertion = resolvesAs row "Guest" (providerConfig.controllerExecutionRef or null);
          message = "${row.path}.spec.config.controllerExecutionRef must resolve to the gateway Guest.";
        }
        {
          assertion = lib.all credentialScopeMatches credentialRefs;
          message = "${row.path}.spec.config ARM credential scope must match controllerExecutionRef.";
        }
        {
          assertion = providerConfig.networkRef or null == null
            || resolvesAs row "Network" providerConfig.networkRef;
          message = "${row.path}.spec.config.networkRef must resolve to a same-Zone Network.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/runtime-azure-container-apps" then [
        {
          assertion = resolvesAs row "Guest" (providerConfig.gatewayExecutionRef or null);
          message = "${row.path}.spec.config.gatewayExecutionRef must resolve to the gateway Guest.";
        }
        {
          assertion = lib.all credentialScopeMatches credentialRefs;
          message = "${row.path}.spec.config credential scopes must match gatewayExecutionRef.";
        }
        {
          assertion = providerConfig.networkRef or null == null
            || resolvesAs row "Network" providerConfig.networkRef;
          message = "${row.path}.spec.config.networkRef must resolve to a same-Zone Network.";
        }
      ] else [ ])
      ++ (if providerRef == "Provider/transport-azure-relay" then [
        {
          assertion = resolvesAs row "Guest" (providerConfig.executionRef or null);
          message = "${row.path}.spec.config.executionRef must resolve to a gateway Guest.";
        }
        {
          assertion = resolvesAs row "Network" (providerConfig.networkRef or null);
          message = "${row.path}.spec.config.networkRef must resolve to a Network.";
        }
      ] else [ ]);

  guestAssertions = row:
    let
      spec = row.resource.spec or { };
      providerRef = spec.providerRef or "";
      providerEnvelope = spec.provider or { };
      settings = providerEnvelope.settings or { };
      providerResolution =
        if lib.elem providerRef runtimeProviderRefs then [
          {
            assertion = resolvesAs row "Provider" providerRef;
            message = "${row.path}.spec.providerRef must resolve to an existing same-Zone runtime Provider.";
          }
        ] else [ ];
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
      providerResolution
      ++ (if providerRef == "Provider/runtime-cloud-hypervisor" then [
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
    provider = resourceFor row providerRef;
    providerConfig =
      if provider == null then { } else (provider.spec or { }).config or { };
    isGatewayProvider = lib.elem providerRef [
      "Provider/runtime-azure-container-apps"
      "Provider/runtime-azure-virtual-machine"
    ];
    executionRef = spec.executionRef or null;
    providerExecutionRef =
      if providerRef == "Provider/runtime-azure-container-apps"
      then providerConfig.gatewayExecutionRef or null
      else providerConfig.controllerExecutionRef or null;
    in (lib.optionals (lib.elem providerRef runtimeProviderRefs) [
      {
        assertion = resolvesAs row "Provider" providerRef;
        message = "${row.path}.spec.providerRef must resolve to an existing same-Zone runtime Provider.";
      }
      {
        assertion = resolvesAs row "Host" providerExecutionRef
          || resolvesAs row "Guest" providerExecutionRef;
        message = "${row.path}.spec.owner Provider execution reference must resolve to a same-Zone Host or Guest.";
      }
      {
        assertion = executionRef == providerExecutionRef;
        message = "${row.path}.spec.executionRef must match the owning Provider execution reference.";
      }
    ]) ++ lib.optionals isGatewayProvider [
      {
        assertion = refType executionRef == "Guest";
        message = "${row.path}.spec.executionRef must be the configured gateway Guest; Host placement is forbidden.";
      }
    ];

  zoneLinkAssertions = row:
    let
      settings = row.resource.spec.transportSettings or { };
      providerRef = row.resource.spec.transportProviderRef or "";
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
      exactRelaySettings = [
        "relayEntityId"
        "relayNamespaceId"
      ];
    in [
      {
        assertion = lib.all (key: !secretKey key) (lib.attrNames settings);
        message = "${row.path}.spec.transportSettings must not contain credential or locator fields.";
      }
    ]
      ++ lib.optionals (providerRef == "Provider/transport-azure-relay") [
        {
          assertion = lib.sort builtins.lessThan (lib.attrNames settings) == exactRelaySettings;
          message = "${row.path}.spec.transportSettings must contain exactly relayNamespaceId and relayEntityId.";
        }
        {
          assertion = builtins.isString (settings.relayNamespaceId or null)
            && builtins.match "^[a-zA-Z0-9][a-zA-Z0-9-]{2,48}[a-zA-Z0-9]$" settings.relayNamespaceId != null;
          message = "${row.path}.spec.transportSettings.relayNamespaceId has an invalid Azure Relay namespace shape.";
        }
        {
          assertion = builtins.isString (settings.relayEntityId or null)
            && builtins.match "^[a-z][a-z0-9-]{1,49}$" settings.relayEntityId != null;
          message = "${row.path}.spec.transportSettings.relayEntityId has an invalid Azure Relay entity shape.";
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
