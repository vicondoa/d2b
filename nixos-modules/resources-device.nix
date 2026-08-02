# Device ResourceType validation and the canonical Device resource projection.
#
# This module deliberately keeps physical-device resolution out of Nix. The
# selector is a stable operator label plus bounded filters; Core and the
# selected Provider resolve the physical backing privately.
{ config, lib, ... }:

let
  cfg = config.d2b;
  tokenPattern = "^[a-z][a-z0-9-]{0,62}$";
  providerRefPattern = "^Provider/[a-z][a-z0-9-]{0,62}$";
  resourceRefPattern =
    "^([A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$";
  hexIdPattern = "^[0-9a-f]{4}$";
  deviceFilterText = maxBytes: value:
    builtins.isString value
    && builtins.stringLength value >= 1
    && builtins.stringLength value <= maxBytes
    && builtins.match "^[!-~]+$" value != null
    && !(lib.hasInfix "/" value)
    && !(lib.hasInfix "\\" value);
  executionPolicyDefaultFields = [
    "defaultDomain"
    "allowedDomains"
    "defaultUserRef"
    "budget"
    "networkAttachments"
    "deviceAttachments"
    "volumeAttachmentDefaults"
  ];

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (lib.attrNames value);

  parseRef = ref:
    let parts = if builtins.isString ref then lib.splitString "/" ref else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resolvesAs = resources: expectedType: ref:
    let parsed = parseRef ref;
    in parsed != null
      && parsed.type == expectedType
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == expectedType;

  providerFor = resources: providerRef:
    let parsed = parseRef providerRef;
    in if parsed != null
      && parsed.type == "Provider"
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == "Provider"
    then resources.${parsed.name}
    else null;

  artifactFor = artifactId:
    if builtins.isString artifactId
      && builtins.hasAttr artifactId (cfg.artifacts or { })
    then cfg.artifacts.${artifactId}
    else null;

  devices = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (name: resource: {
          inherit zoneName zone name resource;
          spec = resource.spec;
          path = "d2b.zones.${zoneName}.resources.${name}";
        })
        (lib.filterAttrs (_: resource: resource.type == "Device") zone.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  providerName = device:
    let parsed = parseRef (attrOr device.spec "providerRef" null);
    in if parsed == null then null else parsed.name;

  providerSettings = device:
    let extension = attrOr device.spec "provider" { };
    in attrOr extension "settings" { };

  providerSchemaId = device:
    let extension = attrOr device.spec "provider" { };
    in attrOr extension "schemaId" null;

  selector = device:
    let inventory = attrOr device.spec "inventory" { };
    in attrOr inventory "selector" { };

  selectorLabel = device:
    attrOr (selector device) "label" null;

  selectorFields = busClass:
    if busClass == "usb" || busClass == "hidraw"
    then [ "busClass" "label" "vendorId" "productId" "serial" ]
    else if busClass == "drm"
    then [ "busClass" "label" "pciSlot" ]
    else if busClass == "pci"
    then [ "busClass" "label" "slot" ]
    else if busClass == "tpm"
    then [ "busClass" "label" "index" ]
    else [ ];

  validSelector = device:
    let
      value = selector device;
      busClass = attrOr value "busClass" null;
      label = attrOr value "label" null;
      vendorId = attrOr value "vendorId" null;
      productId = attrOr value "productId" null;
      serial = attrOr value "serial" null;
      pciSlot = attrOr value "pciSlot" null;
      slot = attrOr value "slot" null;
      index = attrOr value "index" 0;
    in
    builtins.isAttrs value
    && busClass != null
    && builtins.elem busClass [ "usb" "hidraw" "drm" "pci" "tpm" ]
    && exactKeys (selectorFields busClass) value
    && builtins.isString label
    && builtins.match tokenPattern label != null
    && (busClass == "usb" || busClass == "hidraw"
      -> (vendorId == null || (builtins.isString vendorId && builtins.match hexIdPattern vendorId != null))
      && (productId == null || (builtins.isString productId && builtins.match hexIdPattern productId != null))
      && (serial == null || deviceFilterText 128 serial))
    && (busClass == "drm" -> (pciSlot == null || deviceFilterText 31 pciSlot))
    && (busClass == "pci" -> (slot == null || deviceFilterText 31 slot))
    && (busClass == "tpm"
      -> builtins.isInt index && index >= 0 && index <= 255);

  providerSettingsFields = provider:
    if provider == "device-tpm"
    then [ "logLevel" "executionRef" ]
    else if provider == "device-usbip"
    then [ "env" ]
    else if provider == "device-security-key"
    then [ "vsockPort" "sessionRingSize" "leaseTimeoutSecs" ]
    else if provider == "device-gpu"
    then [
      "renderNodeOnly"
      "videoSidecar"
      "videoNvidiaDecode"
      "contextTypes"
      "displays"
      "egl"
      "vulkan"
      "crossDomainTrusted"
      "virglVideo"
    ]
    else [ ];

  providerSettingsValid = device:
    let
      name = providerName device;
      settings = providerSettings device;
      renderNodeOnly = attrOr settings "renderNodeOnly" false;
      videoSidecar = attrOr settings "videoSidecar" false;
      virglVideo = attrOr settings "virglVideo" false;
      contextTypes = attrOr settings "contextTypes" [ "virgl" "virgl2" "cross-domain" ];
      displays = attrOr settings "displays" [ { hidden = true; } ];
      executionRef = attrOr settings "executionRef" null;
    in
    builtins.isAttrs settings
    && exactKeys (providerSettingsFields name) settings
    && (name != "device-tpm"
      || (builtins.isInt (attrOr settings "logLevel" 20)
        && attrOr settings "logLevel" 20 >= 1
        && attrOr settings "logLevel" 20 <= 20
        && (executionRef == null || resolvesAs device.zone.resources "Host" executionRef)))
    && (name != "device-usbip"
      || (builtins.isString (attrOr settings "env" "")
        && builtins.match tokenPattern (attrOr settings "env" "") != null))
    && (name != "device-security-key"
      || (builtins.isInt (attrOr settings "vsockPort" 14320)
        && attrOr settings "vsockPort" 0 >= 1
        && attrOr settings "vsockPort" 0 <= 65535
        && builtins.isInt (attrOr settings "sessionRingSize" 32)
        && attrOr settings "sessionRingSize" 0 >= 8
        && attrOr settings "sessionRingSize" 0 <= 256
        && builtins.isInt (attrOr settings "leaseTimeoutSecs" 300)
        && attrOr settings "leaseTimeoutSecs" 0 >= 30
        && attrOr settings "leaseTimeoutSecs" 0 <= 3600))
    && (name != "device-gpu"
      || (builtins.isBool renderNodeOnly
        && builtins.isBool videoSidecar
        && builtins.isBool (attrOr settings "videoNvidiaDecode" false)
        && builtins.isList contextTypes
        && lib.length contextTypes >= 1
        && lib.length contextTypes <= 3
        && lib.length (lib.unique contextTypes) == lib.length contextTypes
        && lib.all (value: builtins.elem value [ "virgl" "virgl2" "cross-domain" ]) contextTypes
        && builtins.isList displays
        && lib.length displays <= 8
        && lib.all (display: exactKeys [ "hidden" ] display && builtins.isBool display.hidden) displays
        && builtins.isBool (attrOr settings "egl" true)
        && builtins.isBool (attrOr settings "vulkan" true)
        && builtins.isBool (attrOr settings "crossDomainTrusted" false)
        && builtins.isBool virglVideo
        && !(videoSidecar && virglVideo)));

  stringsIn = value:
    if builtins.isString value then [ value ]
    else if builtins.isList value then lib.concatMap stringsIn value
    else if builtins.isAttrs value then lib.concatMap stringsIn (lib.attrValues value)
    else [ ];

  settingsHasForbiddenArtifact = value:
    builtins.isAttrs value
    && lib.any
      (key: builtins.elem key [ "artifactId" "storePath" "stateDirPath" "path" ])
      (lib.attrNames value);

  noInlineSecret = value:
    lib.all
      (text:
        !(lib.hasInfix "/nix/store/" text)
        && !(lib.hasInfix "PRIVATE KEY" text)
        && !(lib.hasInfix "BEGIN " text))
      (stringsIn value);

  canonicalSettings = device:
    let
      name = providerName device;
      authored = providerSettings device;
      defaults =
        if name == "device-tpm"
        then { logLevel = 20; }
        else if name == "device-security-key"
        then { vsockPort = 14320; sessionRingSize = 32; leaseTimeoutSecs = 300; }
        else if name == "device-gpu"
        then {
          renderNodeOnly = false;
          videoSidecar = false;
          videoNvidiaDecode = false;
          contextTypes = [ "cross-domain" "virgl" "virgl2" ];
          displays = [ { hidden = true; } ];
          egl = true;
          vulkan = true;
          crossDomainTrusted = false;
          virglVideo = false;
        }
        else { };
    in defaults // authored;

  canonicalSpec = device:
    let
      spec = device.spec;
      executionDefaults = [
        "defaultDomain"
        "allowedDomains"
        "defaultUserRef"
        "budget"
        "networkAttachments"
        "deviceAttachments"
        "volumeAttachmentDefaults"
      ];
    in
    (builtins.removeAttrs spec executionDefaults)
    // {
      maxConcurrentClaims = attrOr spec "maxConcurrentClaims" 1;
      inventory = {
        selector = selector device;
      };
    }
    // lib.optionalAttrs (attrOr spec "provider" null != null) {
      provider = (attrOr spec "provider" { }) // {
        settings = canonicalSettings device;
      };
    };

  canonicalResource = zoneName: resourceName: device: {
    apiVersion = "resources.d2bus.org/v3";
    type = "Device";
    metadata = {
      name = resourceName;
      zone = zoneName;
    }
    // lib.optionalAttrs (device.resource.metadata.ownerRef != null) {
      ownerRef = device.resource.metadata.ownerRef;
    }
    // lib.optionalAttrs (attrOr device.resource.metadata "labels" { } != { }) {
      labels = attrOr device.resource.metadata "labels" { };
    }
    // lib.optionalAttrs (attrOr device.resource.metadata "annotations" { } != { }) {
      annotations = attrOr device.resource.metadata "annotations" { };
    };
    spec = canonicalSpec device;
  };

  compiledByZone = lib.foldl'
    (result: device:
      result // {
        ${device.zoneName} = (result.${device.zoneName} or { }) // {
          ${device.name} = canonicalResource device.zoneName device.name device;
        };
      })
    { }
    devices;

  providerInstallRefs = lib.unique (lib.filter builtins.isString
    (map (device: attrOr device.spec "providerRef" null) devices));

  providerInstallations = map
    (providerRef:
      let
        parsed = parseRef providerRef;
      in {
        inherit providerRef;
        providerName = if parsed == null then null else parsed.name;
      })
    providerInstallRefs;

  deviceAssertions = lib.flatten (map
    (device:
      let
        spec = device.spec;
        resources = device.zone.resources;
        provider = providerFor resources (attrOr spec "providerRef" null);
        name = providerName device;
        extension = attrOr spec "provider" null;
        settings = providerSettings device;
        arbitration = attrOr spec "arbitration" null;
        deviceClass = attrOr spec "deviceClass" null;
        maxClaims = attrOr spec "maxConcurrentClaims" 1;
        value = selector device;
        busClass = attrOr value "busClass" null;
        providerSpec = if provider == null then { } else provider.spec or { };
        providerArtifactId = providerSpec.artifactId or null;
        providerArtifact =
          if builtins.isString providerArtifactId
          then artifactFor providerArtifactId
          else null;
      in [
        {
          assertion = exactKeys [
            "providerRef"
            "updatePolicy"
            "provider"
            "deviceClass"
            "arbitration"
            "maxConcurrentClaims"
            "inventory"
          ] spec
            || exactKeys ([
              "providerRef"
              "updatePolicy"
              "provider"
              "deviceClass"
              "arbitration"
              "maxConcurrentClaims"
              "inventory"
            ] ++ executionPolicyDefaultFields) spec;
          message = "${device.path}.spec contains an unsupported field. Remove fields not declared by the Device ResourceSpec schema.";
        }
        {
          assertion = builtins.match providerRefPattern (attrOr spec "providerRef" "") != null;
          message = "${device.path}.spec.providerRef must match Provider/<name> (invalid-provider-ref).";
        }
        {
          assertion = provider != null;
          message = "${device.path}.spec.providerRef must resolve to an installed Provider (unresolved-provider-ref).";
        }
        {
          assertion = provider == null
            || (builtins.isString providerArtifactId
              && builtins.match tokenPattern providerArtifactId != null);
          message = "${device.path}.spec.providerRef must select an explicit Provider install with a bounded artifactId.";
        }
        {
          assertion = provider == null
            || (providerSpec ? config && builtins.isAttrs providerSpec.config);
          message = "${device.path}.spec.providerRef must select a Provider install with a config object.";
        }
        {
          assertion = (cfg.artifacts or { }) == { }
            || provider == null
            || (providerArtifact != null && (providerArtifact.type or null) == "provider");
          message = "${device.path}.spec.providerRef artifactId must resolve to a provider artifact.";
        }
        {
          assertion = builtins.elem name [
            "device-tpm"
            "device-usbip"
            "device-security-key"
            "device-gpu"
          ];
          message = "${device.path}.spec.providerRef must select one of the frozen Device Providers.";
        }
        {
          assertion = builtins.elem deviceClass [ "emulated" "physical" ];
          message = "${device.path}.spec.deviceClass is invalid (invalid-device-class).";
        }
        {
          assertion = builtins.elem arbitration [ "exclusive" "shared" ];
          message = "${device.path}.spec.arbitration is invalid (invalid-arbitration).";
        }
        {
          assertion = builtins.isInt maxClaims && maxClaims >= 1 && maxClaims <= 16;
          message = "${device.path}.spec.maxConcurrentClaims is outside 1-16 (max-claims-out-of-bounds).";
        }
        {
          assertion = arbitration != "exclusive" || maxClaims == 1;
          message = "${device.path}: exclusive arbitration requires maxConcurrentClaims = 1 (exclusive-max-claims-conflict).";
        }
        {
          assertion = arbitration != "shared" || deviceClass == "physical";
          message = "${device.path}: shared arbitration requires a physical Device (emulated-shared-arbitration).";
        }
        {
          assertion = deviceClass != "emulated" || value == { };
          message = "${device.path}: emulated Devices must not carry an inventory selector (emulated-with-nonempty-selector).";
        }
        {
          assertion = deviceClass != "physical" || validSelector device;
          message = "${device.path}: physical Devices require a valid closed inventory selector (physical-missing-selector-label or unknown-bus-class).";
        }
        {
          assertion = name != "device-tpm"
            || (deviceClass == "emulated" && value == { });
          message = "${device.path}: device-tpm requires an emulated Device with an empty inventory selector (tpm-device-shape-invalid).";
        }
        {
          assertion = name != "device-usbip"
            || (deviceClass == "physical"
              && arbitration == "exclusive"
              && busClass == "usb");
          message = "${device.path}: device-usbip requires an exclusive physical USB Device (usbip-device-shape-invalid).";
        }
        {
          assertion = name != "device-security-key"
            || (deviceClass == "physical"
              && arbitration == "exclusive"
              && busClass == "hidraw");
          message = "${device.path}: device-security-key requires an exclusive physical hidraw Device (security-key-device-shape-invalid).";
        }
        {
          assertion = name != "device-gpu"
            || (deviceClass == "physical" && busClass == "drm");
          message = "${device.path}: device-gpu requires a physical DRM Device (gpu-device-shape-invalid).";
        }
        {
          assertion = extension == null
            || (builtins.isAttrs extension
              && exactKeys [ "schemaId" "schemaVersion" "settings" ] extension
              && builtins.isString (attrOr extension "schemaId" null)
              && builtins.isString (attrOr extension "schemaVersion" null));
          message = "${device.path}.spec.provider must be the strict schemaId/schemaVersion/settings envelope.";
        }
        {
          assertion = extension == null
            || (name != null
              && providerSchemaId device == "${name}.d2bus.org/Device/spec");
          message = "${device.path}.spec.provider.schemaId must bind to the selected Provider (spec-provider-schema-invalid).";
        }
        {
          assertion = extension == null || providerSettingsValid device;
          message = "${device.path}.spec.provider.settings is invalid for its signed Provider schema (invalid-provider-settings).";
        }
        {
          assertion = extension == null
            || !(name == "device-tpm"
              && builtins.isAttrs settings
              && builtins.hasAttr "startupClear" settings);
          message = "${device.path}.spec.provider.settings.startupClear is not supported; the TPM flush is mandatory (tpm-startup-clear-forbidden).";
        }
        {
          assertion = extension == null || !settingsHasForbiddenArtifact settings;
          message = "${device.path}.spec.provider.settings must not carry artifactId or a store/path field (spec-provider-shadow).";
        }
        {
          assertion = extension == null || noInlineSecret settings;
          message = "${device.path}.spec.provider.settings contains inline secret or store material (inline-secret-in-settings).";
        }
        {
          assertion = !(name == "device-gpu" && arbitration == "shared")
            || attrOr settings "renderNodeOnly" false;
          message = "${device.path}: shared GPU arbitration requires renderNodeOnly = true (shared-arbitration-requires-render-node-only).";
        }
        {
          assertion = !(name == "device-gpu" && attrOr settings "videoSidecar" false)
            || !(attrOr settings "renderNodeOnly" false);
          message = "${device.path}: videoSidecar requires a full GPU Device.";
        }
        {
          assertion = !(name == "device-gpu" && attrOr settings "videoNvidiaDecode" false)
            || attrOr settings "videoSidecar" false;
          message = "${device.path}: videoNvidiaDecode requires videoSidecar = true (nvidia-decode-requires-video-sidecar).";
        }
      ])
    devices);

  duplicateLabelAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone:
      let
        rows = lib.filter (device: selectorLabel device != null)
          (lib.mapAttrsToList
            (name: resource: {
              inherit name resource;
              spec = resource.spec;
              zone = zone;
            })
            (lib.filterAttrs (_: resource: resource.type == "Device") zone.resources));
        labels = map (row: selectorLabel {
          spec = row.spec;
          resource = row.resource;
          zone = row.zone;
        }) rows;
        duplicates = lib.unique (lib.filter
          (label: lib.length (lib.filter (candidate: candidate == label) labels) > 1)
          labels);
      in map
        (label: {
          assertion = false;
          message = "d2b.zones.${zoneName}: duplicate Device inventory selector label (${label}) (duplicate-device-label).";
        })
        duplicates)
    cfg.zones);
  validationAssertions = deviceAssertions ++ duplicateLabelAssertions;
in
{
  options.d2b.zones = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule {
      options.resources = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule { });
      };
    });
  };

  config = lib.mkIf (devices != [ ]) {
    assertions = validationAssertions;

    d2b._index.devices = {
      list = devices;
      byZone = compiledByZone;
    };
    d2b._resourceCompiler.devices = {
      list = devices;
      byZone = compiledByZone;
      providerRefs = providerInstallRefs;
      providerInstallations = providerInstallations;
    };
    # Keep the Device validator's canonical assertion records available to
    # module-level consumers without forcing the bundle integrity gate. The
    # latter deliberately rejects path-shaped provider settings by throwing.
    d2b._resourceCompiler.deviceValidation = validationAssertions;
  };
}
