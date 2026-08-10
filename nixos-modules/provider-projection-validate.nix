# Validate a Provider projection factory against its committed semantic schema.
#
# The schema artifacts are the signed Core catalog.  Provider descriptors are
# supplied through the internal validation seam below; a descriptor is never
# allowed to restate only the fingerprints, because exportability is not a
# fingerprint input.
{ config, lib, ... }:

let
  cfg = config.d2b;
  schemaRoot = ../docs/reference/schemas/v3;
  legacyProtocolVersion = "1.0";

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

  publishedSchemas = lib.listToAttrs (map
    (definition: lib.nameValuePair definition.serviceType
      (builtins.fromJSON
        (builtins.readFile (schemaRoot + "/${definition.file}"))))
    schemaDefinitions);

  # This is the complete published factory surface.  Keep the protocol and
  # exportability entries before the fingerprints: the former diagnoses
  # version skew and the latter is not covered by either fingerprint.
  publishedFields = [
    {
      descriptor = "projectionProtocolVersion";
      published = "x-d2b-projection-protocol-version";
      legacy = true;
    }
    {
      descriptor = "serviceType";
      published = "x-d2b-resource-type";
      legacy = false;
    }
    {
      descriptor = "bindingType";
      published = "x-d2b-binding-resource-type";
      legacy = false;
    }
    {
      descriptor = "allowedBackingRefTypes";
      published = "x-d2b-allowed-backing-ref-types";
      legacy = false;
    }
    {
      descriptor = "allowedBindingTargetRefTypes";
      published = "x-d2b-allowed-binding-target-ref-types";
      legacy = false;
    }
    {
      descriptor = "exportability";
      published = "x-d2b-exportability";
      legacy = false;
    }
    {
      descriptor = "projectionSchemaFingerprint";
      published = "x-d2b-projection-schema-fingerprint";
      legacy = false;
    }
    {
      descriptor = "factoryFingerprint";
      published = "x-d2b-factory-fingerprint";
      legacy = false;
    }
  ];

  fingerprintFields = lib.filter (field:
    field.descriptor == "projectionSchemaFingerprint"
    || field.descriptor == "factoryFingerprint")
    publishedFields;

  preFingerprintFields = lib.filter (field:
    !(builtins.elem field fingerprintFields))
    publishedFields;

  validation = cfg._providerProjectionValidation;

  # `factories` is the canonical seam.  `descriptors` is retained as a
  # descriptive alias for callers that provide a complete Provider descriptor
  # rather than only its factory.
  descriptorTable =
    (validation.descriptors or { }) // (validation.factories or { });
  validationEnabled = validation.enable || descriptorTable != { };

  descriptorRows = lib.mapAttrsToList
    (name: value: {
      inherit name value;
      path = "d2b._providerProjectionValidation.factories.${name}";
    })
    descriptorTable;

  factoryOf = value:
    if !builtins.isAttrs value then null
    else if builtins.hasAttr "projectionFactory" value
      && builtins.isAttrs value.projectionFactory
    then value.projectionFactory
    else if builtins.hasAttr "factory" value
      && builtins.isAttrs value.factory
    then value.factory
    else value;

  # A descriptor can carry its Provider metadata around a nested
  # `projectionFactory`.  Read the nested factory first, then permit the
  # outer descriptor to supply the field.  The service type is also accepted
  # as the table key when a provider publishes a map keyed by ResourceType.
  descriptorField = row: field:
    let
      outer = if builtins.isAttrs row.value then row.value else { };
      factory = factoryOf row.value;
      hasFactoryField = builtins.isAttrs factory
        && builtins.hasAttr field factory;
      hasOuterField = builtins.hasAttr field outer;
      value =
        if hasFactoryField
        then factory.${field}
        else if hasOuterField
        then outer.${field}
        else null;
    in
    {
      present = hasFactoryField || hasOuterField;
      inherit value;
    };

  descriptorServiceType = row:
    let
      direct = descriptorField row "serviceType";
    in
    if direct.present then direct.value
    else if builtins.hasAttr row.name publishedSchemas then row.name
    else null;

  schemaFor = serviceType:
    if builtins.isString serviceType
      && builtins.hasAttr serviceType publishedSchemas
    then publishedSchemas.${serviceType}
    else null;

  expectedField = schema: field:
    if builtins.hasAttr field.published schema
    then {
      present = true;
      value = schema.${field.published};
    }
    else {
      present = false;
      value = null;
    };

  actualField = row: field:
    let actual = descriptorField row field.descriptor;
    in
    if field.legacy && !actual.present
    then {
      present = true;
      value = legacyProtocolVersion;
    }
    else actual;

  fieldLabel = field: field.published;

  # Missing fields are kept separate from mismatches.  In particular, an
  # absent protocol field is a legacy 1.0 descriptor, not a malformed one.
  missingPublished = schema: lib.filter
    (field: !(expectedField schema field).present)
    publishedFields;

  missingDescriptor = row: lib.filter
    (field:
      !(field.legacy)
      && !(descriptorField row field.descriptor).present)
    publishedFields;

  mismatch = row: schema: field:
    let
      actual = actualField row field;
      expected = expectedField schema field;
    in actual.present && expected.present && actual.value != expected.value;

  firstMismatch = row: schema: fields:
    lib.findFirst
      (field: mismatch row schema field)
      null
      fields;

  mismatchAssertion = path: field: code: {
    assertion = false;
    message = "${path}: ${code} (${fieldLabel field}).";
  };

  descriptorAssertions = row:
    let
      serviceType = descriptorServiceType row;
      # A table keyed by a semantic Service type is useful for tests and for
      # the generated in-tree descriptors: it keeps the expected artifact
      # stable while a descriptor's declared serviceType is being checked.
      schemaServiceType =
        if builtins.hasAttr row.name publishedSchemas
        then row.name
        else serviceType;
      schema = schemaFor schemaServiceType;
      invalidDescriptor = !builtins.isAttrs row.value;
      unknownService = !invalidDescriptor && schema == null;
      absentPublished =
        if schema == null then [ ] else missingPublished schema;
      absentDescriptor =
        if invalidDescriptor || schema == null then [ ] else missingDescriptor row;
      protocolField =
        lib.findFirst
          (field: mismatch row schema field)
          null
          (if schema == null then [ ] else
            lib.filter (field: field.descriptor == "projectionProtocolVersion")
              publishedFields);
      identityField =
        if schema == null then null
        else firstMismatch row schema preFingerprintFields;
      fingerprintField =
        if schema == null then null
        else firstMismatch row schema fingerprintFields;
    in
    if invalidDescriptor then [
      {
        assertion = false;
        message = "${row.path}: provider-projection-descriptor-invalid.";
      }
    ]
    else if unknownService then [
      {
        assertion = false;
        message = "${row.path}: provider-projection-service-type-unknown.";
      }
    ]
    else if absentPublished != [ ] then [
      (mismatchAssertion row.path (lib.head absentPublished)
        "provider-projection-published-field-missing")
    ]
    else if absentDescriptor != [ ] then [
      (mismatchAssertion row.path (lib.head absentDescriptor)
        "provider-projection-descriptor-field-missing")
    ]
    else if protocolField != null then [
      (mismatchAssertion row.path protocolField
        "provider-projection-protocol-version-mismatch")
    ]
    else if identityField != null then [
      (mismatchAssertion row.path identityField
        "provider-projection-field-mismatch")
    ]
    else if fingerprintField != null then [
      (mismatchAssertion row.path fingerprintField
        "provider-projection-fingerprint-mismatch")
    ]
    else [
      {
        assertion = true;
        message = "${row.path}: provider-projection-fields-match.";
      }
    ];

  assertions = lib.concatMap descriptorAssertions descriptorRows;
in
{
  options.d2b._providerProjectionValidation = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      internal = true;
      visible = false;
      description = "Enable fail-closed Provider projection factory validation.";
    };
    descriptors = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
      internal = true;
      visible = false;
      description = "Provider descriptors carrying signed projection factories.";
    };
    factories = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
      internal = true;
      visible = false;
      description = "Provider projection factory descriptors keyed by provider or service.";
    };
  };

  config = {
    # Supplying a descriptor always enables validation.  The explicit switch
    # is useful for a positive empty-table wiring check, but it cannot disable
    # checks for an actual Provider factory.
    assertions = lib.mkIf validationEnabled assertions;
    d2b._resourceCompiler.providerProjectionValidation = {
      inherit assertions descriptorRows publishedSchemas;
      enabled = validationEnabled;
      publishedFieldNames = map (field: field.published) publishedFields;
    };
  };
}
