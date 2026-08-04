# Nix admission coverage for the complete published projection-factory shape.
{ mkEval, lib, ... }:

let
  schemaRoot = ../../../../docs/reference/schemas/v3;

  readSchema = file:
    builtins.fromJSON (builtins.readFile (schemaRoot + "/${file}"));

  schemaDefinitions = [
    {
      serviceType = "audio.d2bus.org.AudioService";
      file = "audio.d2bus.org_projection_spec.schema.json";
    }
    {
      serviceType = "security-key.d2bus.org.SecurityKeyService";
      file = "security-key.d2bus.org_projection_spec.schema.json";
    }
    {
      serviceType = "telemetry.d2bus.org.TelemetryService";
      file = "telemetry.d2bus.org_projection_spec.schema.json";
    }
    {
      serviceType = "usb.d2bus.org.UsbService";
      file = "usb.d2bus.org_projection_spec.schema.json";
    }
  ];

  schemas = lib.listToAttrs (map
    (definition: lib.nameValuePair definition.serviceType
      (readSchema definition.file))
    schemaDefinitions);

  serviceTypes = map (definition: definition.serviceType) schemaDefinitions;

  matchingFactory = serviceType:
    let schema = schemas.${serviceType};
    in {
      serviceType = schema."x-d2b-resource-type";
      bindingType = schema."x-d2b-binding-resource-type";
      projectionProtocolVersion = schema."x-d2b-projection-protocol-version";
      allowedBackingRefTypes = schema."x-d2b-allowed-backing-ref-types";
      allowedBindingTargetRefTypes =
        schema."x-d2b-allowed-binding-target-ref-types";
      exportability = schema."x-d2b-exportability";
      projectionSchemaFingerprint =
        schema."x-d2b-projection-schema-fingerprint";
      factoryFingerprint = schema."x-d2b-factory-fingerprint";
    };

  matchingFactories = lib.listToAttrs (map
    (serviceType: lib.nameValuePair serviceType (matchingFactory serviceType))
    serviceTypes);
  matchingCfg = (mkEval [ base ]).config;

  base = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = {
      device = "tmpfs";
      fsType = "tmpfs";
    };
    environment.etc."machine-id".text =
      "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    d2b._providerProjectionValidation = {
      enable = true;
      factories = matchingFactories;
    };
  };

  records = override:
    (mkEval [ base override ]).config
      .d2b._resourceCompiler.providerProjectionValidation.assertions;

  failures = override:
    lib.filter (record: !record.assertion) (records override);

  mismatch = serviceType: descriptorFieldName: value:
    let
      factory = (matchingFactory serviceType) // {
        ${descriptorFieldName} = value;
      };
    in
    {
      d2b._providerProjectionValidation.factories =
        lib.mkForce { ${serviceType} = factory; };
    };

  rejectsField = serviceType: publishedFieldName: descriptorFieldName: value:
    lib.any
      (record:
        !record.assertion && lib.hasInfix publishedFieldName record.message)
      (failures (mismatch serviceType descriptorFieldName value));

  audio = "audio.d2bus.org.AudioService";
in
{
  "provider-projection-fields/all-eight-published-fields-match" = {
    expr = {
      # These projections are observations of one matching host. Reusing the
      # evaluated scenario keeps the inventory assertion from constructing an
      # equivalent NixOS system a second time.
      descriptorCount = lib.length
        matchingCfg.d2b._resourceCompiler.providerProjectionValidation.assertions;
      allPass = lib.all
        (record: record.assertion)
        matchingCfg.d2b._resourceCompiler.providerProjectionValidation.assertions;
      publishedFields = matchingCfg.d2b._resourceCompiler
        .providerProjectionValidation.publishedFieldNames;
    };
    expected = {
      descriptorCount = 4;
      allPass = true;
      publishedFields = [
        "x-d2b-projection-protocol-version"
        "x-d2b-resource-type"
        "x-d2b-binding-resource-type"
        "x-d2b-allowed-backing-ref-types"
        "x-d2b-allowed-binding-target-ref-types"
        "x-d2b-exportability"
        "x-d2b-projection-schema-fingerprint"
        "x-d2b-factory-fingerprint"
      ];
    };
  };

  "provider-projection-fields/every-published-field-mismatch-is-rejected" = {
    expr = {
      resourceType = rejectsField audio "x-d2b-resource-type" "serviceType"
        "usb.d2bus.org.UsbService";
      bindingType = rejectsField audio "x-d2b-binding-resource-type" "bindingType"
        "usb.d2bus.org.UsbBinding";
      protocolVersion = rejectsField audio
        "x-d2b-projection-protocol-version" "projectionProtocolVersion" "1.0";
      backingTypes = rejectsField audio "x-d2b-allowed-backing-ref-types"
        "allowedBackingRefTypes" [ ];
      targetTypes = rejectsField audio
        "x-d2b-allowed-binding-target-ref-types"
        "allowedBindingTargetRefTypes" [ "user" ];
      exportability = rejectsField audio "x-d2b-exportability" "exportability"
        "policy-gated";
      projectionFingerprint = rejectsField audio
        "x-d2b-projection-schema-fingerprint"
        "projectionSchemaFingerprint"
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";
      factoryFingerprint = rejectsField audio "x-d2b-factory-fingerprint"
        "factoryFingerprint"
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    };
    expected = {
      resourceType = true;
      bindingType = true;
      protocolVersion = true;
      backingTypes = true;
      targetTypes = true;
      exportability = true;
      projectionFingerprint = true;
      factoryFingerprint = true;
    };
  };

  "provider-projection-fields/legacy-absent-version-is-version-skew" = {
    expr =
      let
        legacyFactory = builtins.removeAttrs
          (matchingFactory audio)
          [ "projectionProtocolVersion" ];
        result = (mkEval [
          base
          ({ ... }: {
            d2b._providerProjectionValidation.factories =
              lib.mkForce { ${audio} = legacyFactory; };
          })
        ]).config.d2b._resourceCompiler.providerProjectionValidation.assertions;
      in
      lib.any
        (record:
          !record.assertion
          && lib.hasInfix "provider-projection-protocol-version-mismatch"
            record.message)
        result;
    expected = true;
  };
}
