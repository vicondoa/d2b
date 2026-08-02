# Semantic Service/Binding and ResourceExport/ResourceImport compilation.
#
# This module is deliberately a compiler seam, not a second public
# vocabulary. The four semantic pairs and their projection fingerprints are
# read from the committed semantic schemas. Security-key backing metadata is
# absent from that catalog and therefore remains fail-closed here.
{ config, lib, ... }:

let
  cfg = config.d2b;
  schemaRoot = ../docs/reference/schemas/v3;

  readSchema = file:
    builtins.fromJSON (builtins.readFile (schemaRoot + "/${file}"));

  factoryDefinitions = {
    "audio.d2bus.org.AudioService" = {
      serviceType = "audio.d2bus.org.AudioService";
      bindingType = "audio.d2bus.org.AudioBinding";
      allowedBackingRefTypes = [ "Endpoint" ];
      allowedBindingTargetRefTypes = [ "Guest" ];
      exportability = "explicit-export";
      projectionSchema = readSchema "audio.d2bus.org_projection_spec.schema.json";
      serviceSchema = readSchema "audio.d2bus.org_AudioService_spec.schema.json";
      bindingSchema = readSchema "audio.d2bus.org_AudioBinding_spec.schema.json";
      statusSchema = readSchema "audio.d2bus.org_AudioBinding_status.schema.json";
    };
    "security-key.d2bus.org.SecurityKeyService" = {
      serviceType = "security-key.d2bus.org.SecurityKeyService";
      bindingType = "security-key.d2bus.org.SecurityKeyBinding";
      # The Provider contract intentionally does not determine this set.
      allowedBackingRefTypes = null;
      allowedBindingTargetRefTypes = [ "Guest" "User" ];
      exportability = "explicit-export";
      projectionSchema = readSchema "security-key.d2bus.org_projection_spec.schema.json";
      serviceSchema = readSchema "security-key.d2bus.org_SecurityKeyService_spec.schema.json";
      bindingSchema = readSchema "security-key.d2bus.org_SecurityKeyBinding_spec.schema.json";
      statusSchema = readSchema "security-key.d2bus.org_SecurityKeyBinding_status.schema.json";
    };
    "telemetry.d2bus.org.TelemetryService" = {
      serviceType = "telemetry.d2bus.org.TelemetryService";
      bindingType = "telemetry.d2bus.org.TelemetryBinding";
      allowedBackingRefTypes = [ "Endpoint" ];
      allowedBindingTargetRefTypes = [ "Guest" "Zone" ];
      exportability = "explicit-export";
      projectionSchema = readSchema "telemetry.d2bus.org_projection_spec.schema.json";
      serviceSchema = readSchema "telemetry.d2bus.org_TelemetryService_spec.schema.json";
      bindingSchema = readSchema "telemetry.d2bus.org_TelemetryBinding_spec.schema.json";
      statusSchema = readSchema "telemetry.d2bus.org_TelemetryBinding_status.schema.json";
    };
    "usb.d2bus.org.UsbService" = {
      serviceType = "usb.d2bus.org.UsbService";
      bindingType = "usb.d2bus.org.UsbBinding";
      allowedBackingRefTypes = [ "Device" ];
      allowedBindingTargetRefTypes = [ "Guest" ];
      exportability = "policy-gated";
      projectionSchema = readSchema "usb.d2bus.org_projection_spec.schema.json";
      serviceSchema = readSchema "usb.d2bus.org_UsbService_spec.schema.json";
      bindingSchema = readSchema "usb.d2bus.org_UsbBinding_spec.schema.json";
      statusSchema = readSchema "usb.d2bus.org_UsbBinding_status.schema.json";
    };
  };

  bindingTypes = lib.mapAttrsToList (_: factory: factory.bindingType) factoryDefinitions;
  serviceTypes = builtins.attrNames factoryDefinitions;

  factoryForService = serviceType:
    if builtins.hasAttr serviceType factoryDefinitions
    then factoryDefinitions.${serviceType}
    else null;

  factoryForBinding = bindingType:
    lib.findFirst (factory: factory.bindingType == bindingType) null
      (lib.attrValues factoryDefinitions);

  parseRef = value:
    let parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  sameZoneRef = zoneName: resources: value:
    let parsed = parseRef value;
    in parsed != null
      && (
        (parsed.type == "Zone" && parsed.name == zoneName)
        || (
          builtins.hasAttr parsed.name resources
          && resources.${parsed.name}.type == parsed.type
        )
      );

  resolvesType = zoneName: resources: expectedType: value:
    let parsed = parseRef value;
    in sameZoneRef zoneName resources value
      && parsed != null
      && parsed.type == expectedType;

  resourceRows = lib.concatMap
    (zoneName:
      lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
          resources = cfg.zones.${zoneName}.resources;
        })
        cfg.zones.${zoneName}.resources)
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  semanticRows = lib.filter
    (row: builtins.elem row.resource.type (serviceTypes ++ bindingTypes))
    resourceRows;
  serviceRows = lib.filter (row: builtins.elem row.resource.type serviceTypes) semanticRows;
  bindingRows = lib.filter (row: builtins.elem row.resource.type bindingTypes) semanticRows;
  exportRows = lib.filter (row: row.resource.type == "ResourceExport") resourceRows;
  importRows = lib.filter (row: row.resource.type == "ResourceImport") resourceRows;

  specSchema = row:
    let factory =
      if builtins.elem row.resource.type serviceTypes
      then factoryForService row.resource.type
      else factoryForBinding row.resource.type;
    in if factory == null
    then null
    else if builtins.elem row.resource.type serviceTypes
    then factory.serviceSchema
    else factory.bindingSchema;

  schemaFieldNames = schema:
    if schema == null then [ ] else builtins.attrNames (schema.properties or { });

  schemaRequired = schema:
    if schema == null then [ ] else schema.required or [ ];

  statusFieldNames = row:
    let factory =
      if builtins.elem row.resource.type serviceTypes
      then factoryForService row.resource.type
      else factoryForBinding row.resource.type;
    in if factory == null then [ ] else schemaFieldNames factory.statusSchema;

  extensionAllowed = key:
    key == "provider";

  baseFieldAssertions = row:
    let
      schema = specSchema row;
      rawSpec = row.resource.spec or { };
      # The shared resource option base supplies typed execution-policy
      # defaults so Host/Guest declarations retain the existing Nix
      # ergonomics.  They are not part of a semantic Service/Binding
      # ResourceSpec, however, so remove only values that are still exactly
      # those compiler defaults.  An explicitly selected providerRef is
      # retained.
      executionDefaults = {
        defaultDomain = "system";
        allowedDomains = [ "system" ];
        defaultUserRef = null;
        budget = {
          cpu = { request = null; limit = null; };
          memory = { request = null; limit = null; };
          pids = { limit = null; };
          fds = { limit = null; };
          ioWeight = null;
          networkEgressBps = null;
          threadLimit = null;
        };
        networkAttachments = [ ];
        deviceAttachments = [ ];
        volumeAttachmentDefaults = [ ];
      };
      spec = builtins.removeAttrs rawSpec (lib.filter
        (field:
          builtins.hasAttr field rawSpec
          && builtins.hasAttr field executionDefaults
          && rawSpec.${field} == executionDefaults.${field})
        (lib.attrNames executionDefaults));
      names = builtins.attrNames spec;
      allowed = schemaFieldNames schema;
      unknown = lib.filter (name: !(builtins.elem name allowed) && !(extensionAllowed name)) names;
      missing = lib.filter (name: !(builtins.hasAttr name spec)) (schemaRequired schema);
      statusOnly = lib.filter
        (name: builtins.elem name (statusFieldNames row) && !(builtins.elem name allowed))
        names;
      providerRef = spec.providerRef or null;
    in
    [
      {
        assertion = schema != null;
        message = "${row.path}: no committed semantic base schema is installed for ${row.resource.type}.";
      }
      {
        assertion = unknown == [ ];
        message = "${row.path}.spec contains implementation detail or an unknown base field: ${lib.concatStringsSep ", " unknown}.";
      }
      {
        assertion = missing == [ ];
        message = "${row.path}.spec is missing required semantic base field(s): ${lib.concatStringsSep ", " missing}.";
      }
      {
        assertion = statusOnly == [ ];
        message = "${row.path}.spec contains status-only field(s): ${lib.concatStringsSep ", " statusOnly}.";
      }
      {
        assertion = resolvesType row.zoneName row.resources "Provider" providerRef;
        message = "${row.path}.spec.providerRef must resolve to a same-Zone Provider.";
      }
    ];

  ownerServiceAssertions = row:
    let
      factory = factoryForService row.resource.type;
      spec = row.resource.spec or { };
      ownerRef = row.resource.metadata.ownerRef or null;
      backingRefs =
        if row.resource.type == "audio.d2bus.org.AudioService"
        then spec.implementationEndpointRefs or [ ]
        else if row.resource.type == "telemetry.d2bus.org.TelemetryService"
        then spec.ingestEndpointRefs or [ ]
        else if row.resource.type == "usb.d2bus.org.UsbService"
        then lib.optional (spec ? backingDeviceRef) spec.backingDeviceRef
        else [ ];
      backingTypes = factory.allowedBackingRefTypes;
      backingValid =
        if backingTypes == null
        then backingRefs == [ ]
        else lib.all
          (ref:
            let parsed = parseRef ref;
            in parsed != null
              && builtins.elem parsed.type backingTypes
              && sameZoneRef row.zoneName row.resources ref)
          backingRefs;
      projectionMarker =
        (row.resource.type == "security-key.d2bus.org.SecurityKeyService"
          && (spec.mode or null) == "projection")
        || (row.resource.type != "security-key.d2bus.org.SecurityKeyService"
          && (spec.serviceRole or null) == "projection");
    in
    [
      {
        assertion = ownerRef == null;
        message = "${row.path}: Nix may author only authority Services; projection Services are Core-created.";
      }
      {
        assertion = !projectionMarker;
        message = "${row.path}.spec requests a projection Service; use ResourceImport and let Core create it.";
      }
      {
        assertion = backingValid;
        message =
          if backingTypes == null
          then "${row.path}: the signed factory does not declare an allowed backing-ref set; refusing to invent one."
          else "${row.path}: every Service backing ref must resolve to an allowed same-Zone type.";
      }
    ];

  bindingTargetRefs = row:
    let
      spec = row.resource.spec or { };
    in
    if row.resource.type == "audio.d2bus.org.AudioBinding"
    then lib.optional (spec ? targetRef) spec.targetRef
    else if row.resource.type == "security-key.d2bus.org.SecurityKeyBinding"
    then
      let target = spec.target or { };
      in (lib.optional (target ? guestRef) target.guestRef)
        ++ (lib.optional (target ? userRef) target.userRef)
    else if row.resource.type == "telemetry.d2bus.org.TelemetryBinding"
    then lib.optional (spec ? producerRef) spec.producerRef
    else lib.optional (spec ? guestRef) spec.guestRef;

  bindingAssertionsFor = row:
    let
      factory = factoryForBinding row.resource.type;
      spec = row.resource.spec or { };
      serviceRef = spec.serviceRef or null;
      service = parseRef serviceRef;
      targets = bindingTargetRefs row;
      targetValid = lib.all
        (ref:
          let parsed = parseRef ref;
          in parsed != null
            && builtins.elem parsed.type factory.allowedBindingTargetRefTypes
            && sameZoneRef row.zoneName row.resources ref)
        targets;
    in
    [
      {
        assertion = row.resource.metadata.ownerRef or null == null;
        message = "${row.path}: Bindings are operator-authored and must not be Core-owned.";
      }
      {
        assertion = service != null
          && builtins.elem service.type serviceTypes
          && service.type == (factoryForBinding row.resource.type).serviceType
          && sameZoneRef row.zoneName row.resources serviceRef;
        message = "${row.path}.spec.serviceRef must resolve to the matching same-Zone Service.";
      }
      {
        assertion = targets != [ ] && targetValid;
        message = "${row.path}.spec target refs must resolve to an allowed same-Zone target type.";
      }
    ];

  physicalKeyRows = lib.concatMap
    (row:
      let keys = [
        "authorityKey"
        "opaqueKeyDigest"
        "physicalUsbBacking"
        "physical-usb-backing"
      ];
      in lib.filter (key: builtins.hasAttr key (row.resource.spec or { })) keys)
    resourceRows;

  serviceAssertions = lib.concatLists (map
    (row: baseFieldAssertions row ++ ownerServiceAssertions row)
    serviceRows);
  bindingRowAssertions = row:
    baseFieldAssertions row ++ bindingAssertionsFor row;

  bindingAssertions = lib.concatLists (map
    (row: bindingRowAssertions row)
    bindingRows);

  providerFor = row: ref:
    let parsed = parseRef ref;
    in if parsed != null
      && parsed.type == "Provider"
      && builtins.hasAttr parsed.name row.resources
      && row.resources.${parsed.name}.type == "Provider"
    then row.resources.${parsed.name}
    else null;

  usbPolicy = cfg._resourceSharingPolicy.usb or { };
  usbPolicyOptedIn = usbPolicy.provider or false
    && usbPolicy.zone or false
    && usbPolicy.export or false
    && usbPolicy.device or false;

  exportAssertions = lib.concatLists (map
    (row:
      let
        spec = row.resource.spec or { };
        factory = factoryForService (spec.serviceType or "");
        target = if factory == null then null else
          let parsed = parseRef (spec.resourceRef or null);
          in if parsed != null && sameZoneRef row.zoneName row.resources spec.resourceRef
          then row.resources.${parsed.name}
          else null;
        targetType = if target == null then null else target.type;
        fingerprint = if factory == null then null
          else factory.projectionSchema."x-d2b-projection-schema-fingerprint";
        factoryFingerprint = if factory == null then null
          else factory.projectionSchema."x-d2b-factory-fingerprint";
        policyOk =
          (spec.serviceType or null) != "usb.d2bus.org.UsbService"
          || usbPolicyOptedIn;
        serviceOnly = target != null
          && targetType == spec.serviceType
          && builtins.elem targetType serviceTypes;
      in
      [
        {
          assertion = factory != null;
          message = "${row.path}.spec.serviceType must name one of the four frozen semantic Service types.";
        }
        {
          assertion = resolvesType row.zoneName row.resources "Provider" (spec.providerRef or null);
          message = "${row.path}.spec.providerRef must resolve to a same-Zone Provider.";
        }
        {
          assertion = serviceOnly;
          message = "${row.path}.spec.resourceRef must name only the local owner Service; Device, Endpoint, Binding, and cross-Zone refs are rejected.";
        }
        {
          assertion = (spec.projectionSchemaFingerprint or null) == fingerprint;
          message = "${row.path}.spec.projectionSchemaFingerprint does not match the signed projection schema.";
        }
        {
          assertion = (spec.factoryFingerprint or null) == factoryFingerprint;
          message = "${row.path}.spec.factoryFingerprint does not match the signed projection factory.";
        }
        {
          assertion = factory == null || factory.exportability != "forbidden";
          message = "${row.path}: this semantic capability is not exportable.";
        }
        {
          assertion = factory == null || factory.allowedBackingRefTypes != null;
          message = "${row.path}: refusing to invent an allowed backing-ref set for this projection factory.";
        }
        {
          assertion = policyOk;
          message = "${row.path}: USB export requires Provider, Zone, ResourceExport, and Device policy opt-in.";
        }
      ])
    exportRows);

  matchingExport = row: spec:
    lib.findFirst
      (candidate:
        let export = candidate.resource.spec or { };
            exportKeyParts = lib.splitString "/" (spec.exportKey or "");
            exportName =
              if lib.length exportKeyParts == 0
              then ""
              else builtins.elemAt exportKeyParts ((lib.length exportKeyParts) - 1);
            consumerZonePolicy = export.consumerZonePolicy or { };
            consumerZones = consumerZonePolicy.zones or [ ];
            parentZone = cfg.zones.${row.zoneName}.parentZone or null;
        in candidate.resourceName == exportName
          && candidate.zoneName == parentZone
          && builtins.elem "Zone/${row.zoneName}" consumerZones
          && (export.serviceType or null) == (spec.expectedServiceType or null)
          && (export.projectionSchemaFingerprint or null)
            == (spec.expectedProjectionSchemaFingerprint or null)
          && (export.factoryFingerprint or null) == (spec.expectedFactoryFingerprint or null))
      null
      (lib.filter
        (candidate:
          candidate.resource.type == "ResourceExport"
          && candidate.zoneName != row.zoneName)
        resourceRows);

  importAssertions = lib.concatLists (map
    (row:
      let
        spec = row.resource.spec or { };
        factory = factoryForService (spec.expectedServiceType or "");
        export = matchingExport row spec;
        exportSpec = if export == null then { } else export.resource.spec or { };
        exportOperations = exportSpec.operations or [ ];
        exportPolicy = exportSpec.consumerZonePolicy or { };
        requestedCapabilities = spec.requestedCapabilities or [ ];
        capabilityCeiling = exportPolicy.capabilityCeiling or exportOperations;
        projectionName = spec.projectionName or "";
        sameNameAuthored = lib.any
          (candidate:
            candidate.zoneName == row.zoneName
            && candidate.resourceName == projectionName)
          resourceRows;
        projectionAllowed =
          factory != null
          && factory.allowedBackingRefTypes != null;
      in
      [
        {
          assertion = resolvesType row.zoneName row.resources "Provider" (spec.providerRef or null);
          message = "${row.path}.spec.providerRef must resolve to a local Provider.";
        }
        {
          assertion = resolvesType row.zoneName row.resources "ZoneLink" (spec.zoneLinkRef or null);
          message = "${row.path}.spec.zoneLinkRef must resolve to a local ZoneLink.";
        }
        {
          assertion = factory != null;
          message = "${row.path}.spec.expectedServiceType must name a frozen semantic Service type.";
        }
        {
          assertion = projectionAllowed;
          message =
            if factory == null
            then "${row.path}: no signed projection factory is available."
            else "${row.path}: refusing to create a projection while the factory backing set is undetermined.";
        }
        {
          assertion = export != null;
          message = "${row.path}: no matching parent-Zone ResourceExport is available for exportKey and fingerprints.";
        }
        {
          assertion = export == null
            || lib.all (capability: builtins.elem capability exportOperations)
              requestedCapabilities;
          message = "${row.path}.spec.requestedCapabilities must be a subset of the exported operations.";
        }
        {
          assertion = export == null
            || lib.all (capability: builtins.elem capability capabilityCeiling)
              requestedCapabilities;
          message = "${row.path}.spec.requestedCapabilities exceeds the export capability ceiling.";
        }
        {
          assertion = builtins.isString (spec.exportKey or null)
            && builtins.stringLength spec.exportKey >= 1
            && builtins.stringLength spec.exportKey <= 128
            && builtins.match
              "^[a-z][A-Za-z0-9._-]{0,63}(/[A-Za-z0-9._-]{1,63})*$"
              spec.exportKey != null;
          message = "${row.path}.spec.exportKey must be a bounded non-ResourceRef key.";
        }
        {
          assertion = builtins.isString projectionName
            && builtins.match "^[a-z][a-z0-9-]{0,62}$" projectionName != null;
          message = "${row.path}.spec.projectionName must be a bounded resource name.";
        }
        {
          assertion = !sameNameAuthored;
          message = "${row.path}: projectionName collides with an authored resource; Core must create exactly one projection Service.";
        }
      ])
    importRows);

  projectionSpec = row:
    let
      spec = row.resource.spec or { };
      serviceType = spec.expectedServiceType;
      base = {
        providerRef = spec.providerRef;
      };
    in
    if serviceType == "audio.d2bus.org.AudioService"
    then base // {
      serviceRole = "projection";
      implementationEndpointRefs = [ ];
    }
    else if serviceType == "security-key.d2bus.org.SecurityKeyService"
    then base // { mode = "projection"; }
    else if serviceType == "telemetry.d2bus.org.TelemetryService"
    then base // {
      serviceRole = "projection";
      signals = spec.requestedCapabilities or [ ];
      quota = spec.requestedQuota or { };
      policy = { };
    }
    else base // {
      mode = "projection";
      accessPolicy = { };
      sourceSchemaFingerprint = spec.expectedProjectionSchemaFingerprint;
    };

  projectionsByZone = lib.foldl'
    (result: row:
      let spec = row.resource.spec or { };
      in result // {
        ${row.zoneName} = (result.${row.zoneName} or { }) // {
          ${spec.projectionName} = {
            apiVersion = "resources.d2bus.org/v3";
            type = spec.expectedServiceType;
            metadata = {
              name = spec.projectionName;
              zone = row.zoneName;
              ownerRef = "ResourceImport/${row.resourceName}";
            };
            spec = projectionSpec row;
          };
        };
      })
    { }
    importRows;

  physicalKeyAssertions = [
    {
      assertion = physicalKeyRows == [ ];
      message = "physical-usb-backing authority keys are Core-derived and may not be authored in Nix.";
    }
  ];

  legacySemanticAliasAssertions = map
    (row: {
      assertion =
        !(builtins.isString row.resource.type
          && lib.hasSuffix "State" row.resource.type);
      message = "${row.path}.type uses a retired semantic *State alias; declare the exact Service or Binding type.";
    })
    resourceRows;
in
{
  options.d2b._resourceSharingPolicy = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config = {
    assertions =
      serviceAssertions
      ++ bindingAssertions
      ++ exportAssertions
      ++ importAssertions
      ++ physicalKeyAssertions
      ++ legacySemanticAliasAssertions;

    d2b._resourceCompiler.sharing = {
      factories = factoryDefinitions;
      serviceTypes = serviceTypes;
      bindingTypes = bindingTypes;
      projectionsByZone = projectionsByZone;
      generatedResources = projectionsByZone;
      physicalUsbBacking = null;
    };
  };
}
