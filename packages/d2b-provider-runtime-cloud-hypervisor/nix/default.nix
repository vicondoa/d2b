# Semantic Guest inputs and the private Cloud Hypervisor setup descriptor.
#
# The Guest controller owns its Process, Endpoint, and Volume children. Nix
# contributes only the Guest/Provider declarations and an artifact-bound,
# semantic descriptor for the private controller contract.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/runtime-cloud-hypervisor";
  zones = cfg.zones or { };
  artifactIdPattern = "^[a-z][a-z0-9-]*$";
  digestPattern = "^sha256:[0-9a-f]{64}$";

  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  artifactFor = artifactId:
    if builtins.isString artifactId
      && builtins.hasAttr artifactId (cfg.artifacts or { })
    then cfg.artifacts.${artifactId}
    else null;

  validDigest = value:
    builtins.isString value && builtins.match digestPattern value != null;

  descriptorDigestForPayload = payload:
    "sha256:${builtins.hashString "sha256" (builtins.toJSON {
      domain = "d2b:v3:ch-guest-setup-descriptor";
      framing = "d2b-digest/v1";
      inherit payload;
    })}";

  providerFor = zoneName:
    if builtins.hasAttr "runtime-cloud-hypervisor" (resourcesFor zoneName)
      && (resourcesFor zoneName)."runtime-cloud-hypervisor".type == "Provider"
    then (resourcesFor zoneName)."runtime-cloud-hypervisor"
    else null;

  providerCatalogEntries =
    let
      catalog = cfg._providerCatalog or { };
      entries = if builtins.isAttrs catalog then catalog.entries or [ ] else [ ];
    in if builtins.isList entries then entries else [ ];

  providerCatalogEntry = artifactId:
    lib.findFirst
      (entry: builtins.isAttrs entry && (entry.id or null) == artifactId)
      null
      providerCatalogEntries;

  providerContractFor = zoneName:
    let
      provider = providerFor zoneName;
      providerArtifactId =
        if provider == null then null else (provider.spec or { }).artifactId or null;
      providerArtifact = artifactFor providerArtifactId;
      entry = providerCatalogEntry providerArtifactId;
      metadata =
        if entry != null
          && builtins.isAttrs entry
          && builtins.isAttrs (entry.entry or null)
        then entry.entry
        else { };
      providerKey =
        if providerArtifactId == null then providerRef else providerArtifactId;
      fallbackDigest = field:
        "sha256:${builtins.hashString "sha256"
          "d2b:v3:ch-provider-contract/${providerKey}/${field}"}";
      descriptorDigest = metadata.descriptorDigest or null;
      configDigest = metadata.configDigest or null;
    in {
      inherit provider providerArtifactId providerArtifact metadata;
      descriptorDigest =
        if descriptorDigest == null
        then fallbackDigest "descriptor"
        else descriptorDigest;
      configDigest =
        if configDigest == null
        then fallbackDigest "config"
        else configDigest;
      providerGeneration = 1;
      metadataDigestsValid =
        (!builtins.hasAttr "descriptorDigest" metadata
          || validDigest descriptorDigest)
        && (!builtins.hasAttr "configDigest" metadata
          || validDigest configDigest);
      signature =
        if builtins.isString (metadata.signature or null)
          && metadata.signature != ""
        then metadata.signature
        else "catalog-signature";
    };

  systemArtifactCommitment = artifact:
    let catalog = if artifact == null then { } else artifact.catalog or { };
    in if builtins.isAttrs catalog
      && validDigest (catalog.packageDigest or null)
    then catalog.packageDigest
    else "sha256:${builtins.hashString "sha256" "${artifact.package}"}";

  guestRows = lib.concatMap
    (zoneName:
      lib.mapAttrsToList
        (guestName: guest: {
          inherit zoneName guestName guest;
          spec = guest.spec or { };
          provider = providerFor zoneName;
        })
        (lib.filterAttrs
          (_: resource:
            resource.type == "Guest"
            && (resource.spec.providerRef or null) == providerRef)
          (resourcesFor zoneName)))
    (lib.sort lib.lessThan (lib.attrNames zones));

  descriptorFor = row:
    let
      contract = providerContractFor row.zoneName;
      systemArtifactId = row.spec.systemArtifactId or null;
      systemArtifact = artifactFor systemArtifactId;
      providerArtifact = contract.providerArtifact;
      valid =
        contract.provider != null
        && builtins.isString systemArtifactId
        && builtins.match artifactIdPattern systemArtifactId != null
        && systemArtifact != null
        && (systemArtifact.type or null) == "nixos-system"
        && builtins.isString contract.providerArtifactId
        && builtins.match artifactIdPattern contract.providerArtifactId != null
        && providerArtifact != null
        && (providerArtifact.type or null) == "provider"
        && contract.metadataDigestsValid;
    in
    if !valid
    then null
    else
      let
        seedFingerprint = "sha256:${builtins.hashString "sha256"
          "d2b:v3:ch-guest-resource-seed/${contract.configDigest}"}";
        unsigned = {
          schemaVersion = "1.0";
          signatureAlgorithm = "ed25519-blake3";
          signatureKeyFingerprint = contract.descriptorDigest;
          providerRef = providerRef;
          providerGeneration = contract.providerGeneration;
          systemArtifactId = systemArtifactId;
          systemArtifactCommitment = systemArtifactCommitment systemArtifact;
          childRoles = [ "vmm" "ch-api" "guest-control" "system" ];
          seed = {
            schema = "guest-resource-seed";
            schemaVersion = "1.0";
            fingerprint = seedFingerprint;
          };
          bootstrapHandoff = {
            class = "opaque-bootstrap";
            expiryMs = 86400000;
          };
        };
        descriptorDigest = descriptorDigestForPayload (builtins.toJSON unsigned);
      in {
        zone = row.zoneName;
        guest = row.guestName;
        guestRef = "Guest/${row.guestName}";
        providerArtifactId = contract.providerArtifactId;
        providerContractDigest = contract.descriptorDigest;
        providerSchemaDigest = contract.configDigest;
        descriptor = {
          schemaVersion = unsigned.schemaVersion;
          inherit descriptorDigest;
          providerRef = unsigned.providerRef;
          providerGeneration = unsigned.providerGeneration;
          systemArtifactId = unsigned.systemArtifactId;
          systemArtifactCommitment = unsigned.systemArtifactCommitment;
          childRoles = unsigned.childRoles;
          seed = unsigned.seed;
          bootstrapHandoff = unsigned.bootstrapHandoff;
          signature = {
            algorithm = "ed25519-blake3";
            keyFingerprint = contract.descriptorDigest;
            signature = contract.signature;
          };
        };
      };

  descriptorRows = lib.sortOn
    (row: "${row.zone}/${row.guest}")
    (lib.filter (row: row != null) (map descriptorFor guestRows));

  descriptorForGuest = zoneName: guestName:
    lib.findFirst
      (row: row.zone == zoneName && row.guest == guestName)
      null
      descriptorRows;

  descriptorDigestFor = descriptor:
    let
      signature = descriptor.signature or { };
      unsigned = {
        schemaVersion = descriptor.schemaVersion or null;
        signatureAlgorithm = signature.algorithm or null;
        signatureKeyFingerprint = signature.keyFingerprint or null;
        providerRef = descriptor.providerRef or null;
        providerGeneration = descriptor.providerGeneration or null;
        systemArtifactId = descriptor.systemArtifactId or null;
        systemArtifactCommitment = descriptor.systemArtifactCommitment or null;
        childRoles = descriptor.childRoles or [ ];
        seed = descriptor.seed or { };
        bootstrapHandoff = descriptor.bootstrapHandoff or { };
      };
    in descriptorDigestForPayload (builtins.toJSON unsigned);

  projectedPrivateDescriptors =
    let
      compiler = cfg._resourceCompiler or { };
      projection = compiler.providerProjectionRuntimeCloudHypervisor or { };
      privateArtifact = projection.privateArtifact or { };
    in privateArtifact.guestSetupDescriptors or [ ];

  privateDescriptorAssertion = {
    assertion = projectedPrivateDescriptors == descriptorRows;
    message = "runtime-cloud-hypervisor private Guest setup descriptors do not match the selected Guest and Provider inputs.";
  };

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
      c = if provider == null then { } else provider.spec.config or { };
      keys = [
        "controllerExecutionRef"
        "defaultVcpus"
        "defaultMemoryMb"
        "defaultMachineType"
        "watchdog"
        "adoptionWindowMs"
        "healthCheckIntervalMs"
        "healthCheckTimeoutMs"
        "healthCheckFailureThreshold"
        "startupDeadlineMs"
      ];
      controller = c.controllerExecutionRef or null;
      parts = if builtins.isString controller
        then lib.splitString "/" controller
        else [ ];
      resolvesHost = builtins.isString controller
        && lib.length parts == 2
        && builtins.elemAt parts 0 == "Host"
        && builtins.hasAttr (builtins.elemAt parts 1) resources
        && (resources.${builtins.elemAt parts 1}).type == "Host";
      path = "d2b.zones.${zoneName}.resources.runtime-cloud-hypervisor.spec.config";
    in
    if provider == null
    then [ ]
    else [
      {
        assertion = lib.all (key: builtins.elem key keys) (lib.attrNames c);
        message = "${path} contains an unsupported runtime-cloud-hypervisor Provider field.";
      }
      {
        assertion = resolvesHost;
        message = "${path}.controllerExecutionRef must resolve to a same-Zone Host.";
      }
      {
        assertion = !(builtins.hasAttr "defaultVcpus" c)
          || (builtins.isInt c.defaultVcpus
            && c.defaultVcpus >= 1
            && c.defaultVcpus <= 1024);
        message = "${path}.defaultVcpus is out of bounds.";
      }
      {
        assertion = !(builtins.hasAttr "defaultMemoryMb" c)
          || (builtins.isInt c.defaultMemoryMb
            && c.defaultMemoryMb >= 128
            && c.defaultMemoryMb <= 524288);
        message = "${path}.defaultMemoryMb is out of bounds.";
      }
    ];

  guestAssertions = lib.concatMap
    (row:
      let
        path = "d2b.zones.${row.zoneName}.resources.${row.guestName}";
        contract = providerContractFor row.zoneName;
        systemArtifactId = row.spec.systemArtifactId or null;
        systemArtifact = artifactFor systemArtifactId;
        providerArtifact = contract.providerArtifact;
        descriptor = descriptorForGuest row.zoneName row.guestName;
        descriptorValue =
          if descriptor != null && builtins.isAttrs (descriptor.descriptor or null)
          then descriptor.descriptor
          else { };
        seed = descriptorValue.seed or { };
        bootstrapHandoff = descriptorValue.bootstrapHandoff or { };
        signature = descriptorValue.signature or { };
      in [
        {
          assertion = contract.provider != null;
          message = "${path}.spec.providerRef must resolve to a same-Zone runtime-cloud-hypervisor Provider.";
        }
        {
          assertion = builtins.isString systemArtifactId
            && builtins.match artifactIdPattern systemArtifactId != null;
          message = "${path}.spec.systemArtifactId must be a bounded artifact ID.";
        }
        {
          assertion = systemArtifact != null
            && (systemArtifact.type or null) == "nixos-system";
          message = "${path}.spec.systemArtifactId must resolve to a nixos-system artifact.";
        }
        {
          assertion = builtins.isString contract.providerArtifactId
            && builtins.match artifactIdPattern contract.providerArtifactId != null
            && providerArtifact != null
            && (providerArtifact.type or null) == "provider";
          message = "${path}.spec.providerRef must select a provider artifact.";
        }
        {
          assertion = contract.metadataDigestsValid;
          message = "${path}: runtime-cloud-hypervisor Provider contract digests are invalid.";
        }
        {
          assertion = descriptor != null
            && validDigest (descriptorValue.descriptorDigest or null)
            && (descriptorValue.descriptorDigest or null)
              == descriptorDigestFor descriptorValue
            && (descriptorValue.systemArtifactId or null) == systemArtifactId
            && validDigest (descriptorValue.systemArtifactCommitment or null)
            && (descriptorValue.systemArtifactCommitment or null)
              == (if systemArtifact == null
                then null
                else systemArtifactCommitment systemArtifact)
            && (descriptorValue.schemaVersion or null) == "1.0"
            && (seed.schema or null) == "guest-resource-seed"
            && (seed.schemaVersion or null) == "1.0"
            && validDigest (seed.fingerprint or null)
            && (descriptorValue.childRoles or [ ])
              == [ "vmm" "ch-api" "guest-control" "system" ]
            && (bootstrapHandoff.class or null) == "opaque-bootstrap"
            && (bootstrapHandoff.expiryMs or 0) > 0
            && (signature.algorithm or null) == "ed25519-blake3"
            && (signature.keyFingerprint or null)
              == contract.descriptorDigest
            && builtins.isString (signature.signature or null)
            && (signature.signature or null) != ""
            && (descriptor.providerContractDigest or null)
              == contract.descriptorDigest;
          message = "${path}: private Guest setup descriptor does not match the selected artifact or Provider contract.";
        }
      ])
    guestRows;

  enabled = descriptorRows != [ ];
in
{
  config = {
    assertions =
      lib.concatMap providerAssertions (lib.attrNames zones)
      ++ guestAssertions
      ++ [ privateDescriptorAssertion ];
    d2b._resourceCompiler.providerProjectionRuntimeCloudHypervisor = {
      inherit enabled;
      resourcesByZone = { };
      guestPatchesByZone = { };
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        guestSetupDescriptors = descriptorRows;
      };
    };
  };
}
