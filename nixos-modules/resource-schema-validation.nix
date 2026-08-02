# Build-time validation of every emitted standard ResourceSpec.
#
# The committed JSON Schemas are generated artifacts. A Provider package may
# replace the default schema farm with `pkgs.d2b-resource-schemas`, but the
# fallback keeps local evaluation hermetic and makes a missing package visible
# rather than silently disabling validation.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  validationEnabled = cfg._resourceSchemaValidation.enable;
  standardResourceTypes = import ./generated/resource-types.nix;
  schemaFarm =
    if builtins.hasAttr "d2b-resource-schemas" pkgs
    then pkgs."d2b-resource-schemas"
    else pkgs.linkFarm "d2b-resource-schemas" (map
      (resourceType: {
        name = "${resourceType}.schema.json";
        path = ../docs/reference/schemas/v3 + "/${resourceType}.schema.json";
      })
      standardResourceTypes);

  # Evaluation-time checks must not depend on realizing a schema farm
  # derivation.  The committed schemas are the source of truth for Nix
  # assertions; the farm is retained as an explicit build input below so a
  # Provider package can replace it for the derivation-time round trip.
  schemaFor = resourceType:
    let path = ../docs/reference/schemas/v3 + "/${resourceType}.schema.json";
    in if builtins.pathExists path
    then builtins.fromJSON (builtins.readFile path)
    else null;

  executionDefaults = {
    providerRef = null;
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

  emittedSpec = resourceType: spec:
    if resourceType == "Host" || resourceType == "Guest"
    then spec
    else builtins.removeAttrs spec (lib.filter
      (field:
        builtins.hasAttr field spec
        && builtins.hasAttr field executionDefaults
        && spec.${field} == executionDefaults.${field})
      (lib.attrNames executionDefaults));

  rows = lib.concatMap
    (zoneName:
      lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
        })
        cfg.zones.${zoneName}.resources)
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  typeNames = value:
    if builtins.isList value then value
    else if builtins.isString value then [ value ]
    else [ ];

  typeMatches = expected: value:
    builtins.elem expected (typeNames expected);

  scalarTypeMatches = expected: value:
    if expected == "object" then builtins.isAttrs value
    else if expected == "array" then builtins.isList value
    else if expected == "string" then builtins.isString value
    else if expected == "integer" then builtins.isInt value
    else if expected == "number" then builtins.isInt value || builtins.isFloat value
    else if expected == "boolean" then builtins.isBool value
    else if expected == "null" then value == null
    else true;

  resolveRef = root: reference:
    let
      prefix = "#/definitions/";
      name = lib.removePrefix prefix reference;
    in
    if lib.hasPrefix prefix reference && builtins.hasAttr "definitions" root
    then root.definitions.${name}
    else { };

  schemaErrors = root: schema: value: path:
    let
      resolved =
        if schema ? "$ref"
        then schemaErrors root (resolveRef root schema."$ref") value path
        else [ ];
      typeErrors =
        if !(schema ? type) then [ ]
        else
          let accepted = typeNames schema.type;
          in lib.optional
            (!(lib.any (kind: scalarTypeMatches kind value) accepted))
            "${path} has the wrong JSON type";
      constErrors =
        lib.optional (schema ? const && value != schema.const)
          "${path} does not equal the schema constant";
      enumErrors =
        lib.optional (schema ? enum && !(builtins.elem value schema.enum))
          "${path} is outside the schema enum";
      stringErrors =
        if !(builtins.isString value) then [ ]
        else
          lib.optional
            (schema ? pattern && builtins.match schema.pattern value == null)
            "${path} does not match the schema pattern"
          ++ lib.optional
            (schema ? minLength && builtins.stringLength value < schema.minLength)
            "${path} is shorter than the schema minimum"
          ++ lib.optional
            (schema ? maxLength && builtins.stringLength value > schema.maxLength)
            "${path} is longer than the schema maximum";
      numberErrors =
        if !(builtins.isInt value || builtins.isFloat value) then [ ]
        else
          lib.optional (schema ? minimum && value < schema.minimum)
            "${path} is below the schema minimum"
          ++ lib.optional (schema ? maximum && value > schema.maximum)
            "${path} is above the schema maximum";
      arrayErrors =
        if !builtins.isList value then [ ]
        else
          lib.optional (schema ? minItems && lib.length value < schema.minItems)
            "${path} has fewer items than the schema minimum"
          ++ lib.optional (schema ? maxItems && lib.length value > schema.maxItems)
            "${path} has more items than the schema maximum"
          ++ (if schema ? items
              then lib.concatLists (lib.imap0
                (index: item:
                  schemaErrors root schema.items item "${path}.${toString index}")
                value)
              else [ ]);
      objectErrors =
        if !builtins.isAttrs value then [ ]
        else
          let
            properties = if schema ? properties then schema.properties else { };
            names = builtins.attrNames value;
            required = if schema ? required then schema.required else [ ];
            missing = lib.filter (name: !(builtins.hasAttr name value)) required;
            unknown =
              if schema ? additionalProperties && schema.additionalProperties == false
              then lib.filter (name: !(builtins.hasAttr name properties)) names
              else [ ];
            knownErrors = lib.concatLists (map
              (name:
                schemaErrors root properties.${name} value.${name} "${path}.${name}")
              (lib.filter (name: builtins.hasAttr name properties) names));
          in
          map (name: "${path}.${name} is required by the schema") missing
          ++ map (name: "${path}.${name} is not declared by the schema") unknown
          ++ knownErrors;
      branches =
        if schema ? anyOf
        then lib.optional
          (!(lib.any (branch: schemaErrors root branch value path == [ ]) schema.anyOf))
          "${path} does not satisfy any schema branch"
        else if schema ? oneOf
        then lib.optional
          (lib.length (lib.filter (branch: schemaErrors root branch value path == [ ]) schema.oneOf) != 1)
          "${path} does not satisfy exactly one schema branch"
        else [ ];
    in
    resolved ++ typeErrors ++ constErrors ++ enumErrors ++ stringErrors
    ++ numberErrors ++ arrayErrors ++ objectErrors ++ branches;

  rowErrors = row:
    let
      schema = schemaFor row.resource.type;
      specSchema =
        if schema != null && schema ? properties && schema.properties ? spec
        then schema.properties.spec
        else null;
      spec = emittedSpec row.resource.type (row.resource.spec or { });
    in
    if !(builtins.elem row.resource.type standardResourceTypes)
    then [ ]
    else if specSchema == null
    then [ "${row.path}: no committed ResourceType schema is installed" ]
    else schemaErrors schema specSchema spec "${row.path}.spec";

  assertions = map
    (row: {
      assertion = rowErrors row == [ ];
      message =
        if rowErrors row == [ ]
        then "${row.path}.spec matches the committed v3 ResourceType schema."
        else lib.concatStringsSep "; " (rowErrors row);
    })
    rows;

  resourcesJson = builtins.toJSON (map
    (row: {
      apiVersion = "resources.d2bus.org/v3";
      type = row.resource.type;
      metadata = {
        name = row.resourceName;
        zone = row.zoneName;
      };
      spec = emittedSpec row.resource.type (row.resource.spec or { });
    })
    rows);

  buildValidation = pkgs.runCommand "d2b-resource-spec-schema-validation"
    {
      inherit resourcesJson schemaFarm;
      passAsFile = [ "resourcesJson" ];
      nativeBuildInputs = [ pkgs.python3 ];
    } ''
      set -euo pipefail
      python3 - "$resourcesJsonPath" "$schemaFarm" "$out" <<'PY'
      import json
      import pathlib
      import sys

      resources_path, schema_root, output = sys.argv[1:]
      resources = json.loads(pathlib.Path(resources_path).read_text())

      def resolve(root, schema):
          ref = schema.get("$ref")
          if not ref:
              return schema
          prefix = "#/definitions/"
          if not ref.startswith(prefix):
              raise ValueError(f"unsupported schema reference: {ref}")
          return root["definitions"][ref[len(prefix):]]

      def matches_type(kind, value):
          return {
              "object": isinstance(value, dict),
              "array": isinstance(value, list),
              "string": isinstance(value, str),
              "integer": isinstance(value, int) and not isinstance(value, bool),
              "number": isinstance(value, (int, float)) and not isinstance(value, bool),
              "boolean": isinstance(value, bool),
              "null": value is None,
          }.get(kind, True)

      def check(root, schema, value, path):
          schema = resolve(root, schema)
          for branch_key in ("anyOf", "oneOf"):
              if branch_key in schema:
                  passing = []
                  for branch in schema[branch_key]:
                      try:
                          check(root, branch, value, path)
                      except ValueError:
                          continue
                      passing.append(branch)
                  expected = 1 if branch_key == "oneOf" else 1
                  if len(passing) != expected:
                      raise ValueError(f"{path} does not satisfy {branch_key}")
                  return
          if "const" in schema and value != schema["const"]:
              raise ValueError(f"{path} does not equal the schema constant")
          if "enum" in schema and value not in schema["enum"]:
              raise ValueError(f"{path} is outside the schema enum")
          expected = schema.get("type")
          if expected is not None:
              kinds = expected if isinstance(expected, list) else [expected]
              if not any(matches_type(kind, value) for kind in kinds):
                  raise ValueError(f"{path} has the wrong JSON type")
          if isinstance(value, str):
              if "pattern" in schema:
                  import re
                  if re.fullmatch(schema["pattern"], value) is None:
                      raise ValueError(f"{path} does not match the schema pattern")
              if "minLength" in schema and len(value) < schema["minLength"]:
                  raise ValueError(f"{path} is shorter than the schema minimum")
              if "maxLength" in schema and len(value) > schema["maxLength"]:
                  raise ValueError(f"{path} is longer than the schema maximum")
          if isinstance(value, (int, float)) and not isinstance(value, bool):
              if "minimum" in schema and value < schema["minimum"]:
                  raise ValueError(f"{path} is below the schema minimum")
              if "maximum" in schema and value > schema["maximum"]:
                  raise ValueError(f"{path} is above the schema maximum")
          if isinstance(value, list):
              if "minItems" in schema and len(value) < schema["minItems"]:
                  raise ValueError(f"{path} has fewer items than the schema minimum")
              if "maxItems" in schema and len(value) > schema["maxItems"]:
                  raise ValueError(f"{path} has more items than the schema maximum")
              if "items" in schema:
                  for index, item in enumerate(value):
                      check(root, schema["items"], item, f"{path}.{index}")
          if isinstance(value, dict):
              properties = schema.get("properties", {})
              for required in schema.get("required", []):
                  if required not in value:
                      raise ValueError(f"{path}.{required} is required by the schema")
              if schema.get("additionalProperties") is False:
                  unknown = sorted(set(value) - set(properties))
                  if unknown:
                      raise ValueError(f"{path}.{unknown[0]} is not declared by the schema")
              for name, item in value.items():
                  if name in properties:
                      check(root, properties[name], item, f"{path}.{name}")

      for resource in resources:
          resource_type = resource["type"]
          schema_path = pathlib.Path(schema_root) / f"{resource_type}.schema.json"
          if not schema_path.exists():
              raise SystemExit(f"{resource_type}: committed schema is missing")
          root = json.loads(schema_path.read_text())
          check(root, root["properties"]["spec"], resource["spec"],
                f"{resource_type}/{resource['metadata']['name']}.spec")

      pathlib.Path(output).write_text("v3 ResourceSpec schemas validated\n")
      PY
    '';
in
{
  options.d2b._resourceSchemaValidation.enable = lib.mkOption {
    type = lib.types.bool;
    default = false;
    internal = true;
    visible = false;
    description = "Enable validation against committed v3 ResourceType schemas.";
  };

  config = {
    assertions = lib.mkIf validationEnabled assertions;
    d2b._resourceCompiler.schemaValidation = lib.mkIf validationEnabled {
      inherit schemaFarm buildValidation;
      resourceTypes = standardResourceTypes;
      errors = lib.concatMap rowErrors rows;
    };
  };
}
