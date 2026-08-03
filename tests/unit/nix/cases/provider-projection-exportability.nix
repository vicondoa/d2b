# Nix admission coverage for the unhashed exportability field.
{ mkEval, lib, ... }:

let
  schemaPath =
    ../../../../docs/reference/schemas/v3/audio.d2bus.org_projection_spec.schema.json;
  schema = builtins.fromJSON (builtins.readFile schemaPath);
  serviceType = schema."x-d2b-resource-type";

  matchingFactory = {
    serviceType = schema."x-d2b-resource-type";
    bindingType = schema."x-d2b-binding-resource-type";
    projectionProtocolVersion =
      schema."x-d2b-projection-protocol-version";
    allowedBackingRefTypes = schema."x-d2b-allowed-backing-ref-types";
    allowedBindingTargetRefTypes =
      schema."x-d2b-allowed-binding-target-ref-types";
    exportability = schema."x-d2b-exportability";
    projectionSchemaFingerprint =
      schema."x-d2b-projection-schema-fingerprint";
    factoryFingerprint = schema."x-d2b-factory-fingerprint";
  };

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
      factories = {
        ${serviceType} = matchingFactory;
      };
    };
  };

  evaluated = factory:
    (mkEval [
      base
      ({ ... }: {
        d2b._providerProjectionValidation.factories =
          lib.mkForce { ${serviceType} = factory; };
      })
    ]).config.d2b._resourceCompiler.providerProjectionValidation.assertions;

  wrongExportability = matchingFactory // {
    exportability = "policy-gated";
  };
in
{
  "provider-projection-exportability/matching-fingerprints-do-not-mask-exportability" = {
    expr = {
      fingerprintMatches =
        wrongExportability.factoryFingerprint
        == matchingFactory.factoryFingerprint
        && wrongExportability.projectionSchemaFingerprint
          == matchingFactory.projectionSchemaFingerprint;
      rejected = lib.any
        (record:
          !record.assertion
          && lib.hasInfix "x-d2b-exportability" record.message)
        (evaluated wrongExportability);
    };
    expected = {
      fingerprintMatches = true;
      rejected = true;
    };
  };

  "provider-projection-exportability/matching-factory-is-accepted" = {
    expr = lib.all (record: record.assertion) (evaluated matchingFactory);
    expected = true;
  };
}
