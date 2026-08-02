# Provider-owned schema validation for Provider.config and spec.provider.settings.
#
# Provider packages publish their signed schemas through the internal
# `_providerSettingsSchemas` table. This module deliberately has no fallback
# vocabulary: when validation is enabled, a missing digest-to-schema entry is
# an error instead of an invented Provider-specific field set.
{ config, lib, ... }:

let
  cfg = config.d2b;
  validation = cfg._providerSettingsValidation;
  schemas = validation.schemas;

  providerRows = lib.concatMap
    (zoneName:
      lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
        })
        (lib.filterAttrs (_: resource: resource.type == "Provider") cfg.zones.${zoneName}.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  parseRef = value:
    let parts = if builtins.isString value then lib.splitString "/" value else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  providerArtifactId = row: row.resource.spec.artifactId or null;

  schemaFor = row:
    let
      artifactId = providerArtifactId row;
      entry =
        if artifactId != null
        then lib.findFirst
          (candidate: (candidate.id or null) == artifactId)
          null
          (cfg._providerCatalog.entries or [ ])
        else null;
      digest =
        if entry != null
          && builtins.hasAttr "entry" entry
          && builtins.hasAttr "settingsSchemaDigest" entry.entry
        then entry.entry.settingsSchemaDigest
        else if entry != null && builtins.hasAttr "settingsSchemaDigest" entry
        then entry.settingsSchemaDigest
        else null;
    in
    if digest != null && builtins.hasAttr digest schemas
    then schemas.${digest}
    else if artifactId != null && builtins.hasAttr artifactId schemas
    then schemas.${artifactId}
    else null;

  schemaErrors = schema: value: path:
    let
      type = schema.type or null;
      typeOk =
        type == null
        || (type == "object" && builtins.isAttrs value)
        || (type == "array" && builtins.isList value)
        || (type == "string" && builtins.isString value)
        || (type == "integer" && builtins.isInt value)
        || (type == "boolean" && builtins.isBool value)
        || (type == "null" && value == null);
      enumOk = !(schema ? enum) || builtins.elem value schema.enum;
      patternOk =
        !(schema ? pattern)
        || !(builtins.isString value)
        || builtins.match schema.pattern value != null;
      numericOk =
        !(builtins.isInt value)
        || (!(schema ? minimum) || value >= schema.minimum)
        && (!(schema ? maximum) || value <= schema.maximum);
      objectErrors =
        if !builtins.isAttrs value then [ ]
        else
          let
            properties = schema.properties or { };
            required = schema.required or [ ];
            missing = lib.filter (name: !(builtins.hasAttr name value)) required;
            unknown =
              if (schema.additionalProperties or true) == false
              then lib.filter (name: !(builtins.hasAttr name properties))
                (builtins.attrNames value)
              else [ ];
          in
          map (name: "${path}.${name} is required by the signed schema") missing
          ++ map (name: "${path}.${name} is not declared by the signed schema") unknown
          ++ lib.concatLists (map
            (name:
              if builtins.hasAttr name properties
              then schemaErrors properties.${name} value.${name} "${path}.${name}"
              else [ ])
            (builtins.attrNames value));
      arrayErrors =
        if !builtins.isList value || !(schema ? items) then [ ]
        else lib.concatLists (lib.imap0
          (index: item: schemaErrors schema.items item "${path}.${toString index}")
          value);
    in
    lib.optional (!typeOk) "${path} has the wrong JSON type"
    ++ lib.optional (!enumOk) "${path} is outside the signed schema enum"
    ++ lib.optional (!patternOk) "${path} does not match the signed schema pattern"
    ++ lib.optional (!numericOk) "${path} is outside the signed numeric bounds"
    ++ objectErrors ++ arrayErrors;

  settingsRows = lib.concatMap
    (row:
      let
        spec = row.resource.spec or { };
        providerExtension = spec.provider or null;
        settings =
          if builtins.isAttrs providerExtension
          then providerExtension.settings or { }
          else { };
        schema = schemaFor row;
        config = spec.config or { };
        configSchema =
          if schema != null && schema ? configSchema
          then schema.configSchema
          else schema;
        settingsSchema =
          if schema != null && schema ? settingsSchema
          then schema.settingsSchema
          else schema;
      in
      [
        {
          assertion = configSchema != null;
          message = "${row.path}: no signed Provider schema is available for config validation.";
        }
        {
          assertion = configSchema == null || schemaErrors configSchema config "${row.path}.spec.config" == [ ];
          message =
            if configSchema == null
            then "${row.path}: Provider config validation is unavailable."
            else lib.concatStringsSep "; " (schemaErrors configSchema config "${row.path}.spec.config");
        }
      ]
      ++ lib.optional (providerExtension != null) {
        assertion = settingsSchema != null;
        message = "${row.path}.spec.provider.settings has no signed schema.";
      }
      ++ lib.optional (providerExtension != null && settingsSchema != null) {
        assertion = schemaErrors settingsSchema settings "${row.path}.spec.provider.settings" == [ ];
        message = lib.concatStringsSep "; "
          (schemaErrors settingsSchema settings "${row.path}.spec.provider.settings");
      })
    providerRows;
in
{
  options.d2b._providerSettingsValidation = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      internal = true;
      visible = false;
      description = "Enable fail-closed signed Provider settings validation.";
    };
    schemas = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
      internal = true;
      visible = false;
      description = "Digest- or artifact-keyed signed Provider schemas.";
    };
  };

  config = lib.mkIf validation.enable {
    assertions = settingsRows;
  };
}
