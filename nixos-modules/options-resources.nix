# Generic Zone resource authoring extensions and Credential validation.
{ config, lib, ... }:

let
  cfg = config.d2b;
  types = lib.types;

  resourceRefPattern =
    "([A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}";
  audiencePattern = "^[A-Za-z0-9._:/@-]+$";
  credentialRefPattern = "Credential/[a-z][a-z0-9-]{0,62}";
  credentialOperations = [
    "acquire-token"
    "refresh-token"
    "revoke-token"
    "sign-challenge"
    "inspect-metadata"
  ];

  parseRef = ref:
    let parts = if builtins.isString ref then lib.splitString "/" ref else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resolvesAs = resources: expectedTypes: ref:
    let parsed = parseRef ref;
    in parsed != null
      && builtins.elem parsed.type expectedTypes
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == parsed.type;

  stringsIn = value:
    if builtins.isString value then [ value ]
    else if builtins.isList value then lib.concatMap stringsIn value
    else if builtins.isAttrs value then
      lib.concatMap stringsIn (lib.attrValues value)
    else [ ];

  containsSensitiveShape = value:
    let
      lower = lib.toLower value;
      compact = lib.replaceStrings [ "-" "_" "." " " "=" ":" "/" ]
        [ "" "" "" "" "" "" "" ] lower;
      shapedMarkers = [
        "sharedaccesskey"
        "accountkey"
        "privatekey"
      ];
    in
      lib.hasInfix "-----BEGIN" value
      || lib.hasPrefix "eyJ" value
      || lib.any (marker: lib.hasInfix marker compact) shapedMarkers
      || builtins.match ".*[0-9A-Fa-f]{32,}.*" value != null;

  providerFor = resources: providerRef:
    let parsed = parseRef providerRef;
    in if parsed != null
      && parsed.type == "Provider"
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == "Provider"
    then resources.${parsed.name}
    else null;

  providerConfig = provider:
    if provider != null && provider.spec ? config && builtins.isAttrs provider.spec.config
    then provider.spec.config
    else { };

  credentialBinding = resource:
    let
      spec = resource.spec;
      scope = spec.scope or { };
    in builtins.toJSON [
      (spec.providerRef or null)
      (scope.executionRef or null)
      (scope.userRef or null)
      (spec.audience or null)
    ];

  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (lib.attrNames value);

  optionalRef = value:
    value == null
    || (builtins.isString value && builtins.match resourceRefPattern value != null);

  credentialRefViolations = path: value:
    if !builtins.isAttrs value then [ ]
    else lib.flatten (lib.mapAttrsToList
      (field: fieldValue:
        let fieldPath = path ++ [ field ];
        in if lib.hasSuffix "CredentialRef" field || field == "credentialRef"
        then lib.optional
          (!(builtins.isString fieldValue)
            || builtins.match credentialRefPattern fieldValue == null)
          (lib.concatStringsSep "." fieldPath)
        else if builtins.isAttrs fieldValue
        then credentialRefViolations fieldPath fieldValue
        else [ ])
      value);

  credentialAssertions = zoneName: resources:
    let
      credentials = lib.filterAttrs (_: resource: resource.type == "Credential") resources;
      bindings = map credentialBinding (lib.attrValues credentials);
      perCredential = lib.flatten (lib.mapAttrsToList
        (resourceName: resource:
          let
            path = "d2b.zones.${zoneName}.resources.${resourceName}";
            rawSpec = resource.spec;
            spec = builtins.removeAttrs rawSpec [
              "defaultDomain"
              "allowedDomains"
              "defaultUserRef"
              "budget"
              "networkAttachments"
              "deviceAttachments"
              "volumeAttachmentDefaults"
            ];
            scope = spec.scope or { };
            rotation = spec.rotation or { };
            provider = providerFor resources (spec.providerRef or null);
            providerCfg = providerConfig provider;
            providerRef = parseRef (spec.providerRef or null);
            artifactId =
              if provider != null && provider.spec ? artifactId
              then provider.spec.artifactId
              else null;
            artifact =
              if artifactId != null && builtins.hasAttr artifactId cfg.artifacts
              then cfg.artifacts.${artifactId}
              else null;
            domainFilter = scope.domainFilter or null;
            supportedDomains = providerCfg.credentialDomains or [ ];
            supportedOperations = providerCfg.supportedOperations or [ ];
            allowedOperations = spec.allowedOperations or [ ];
            proactiveWindow = rotation.proactiveWindowMs or null;
            maximumLifetime = rotation.maxLeaseLifetimeMs or 0;
            refChecks = [
              { field = "consumerRef"; value = spec.consumerRef or null; types = [ "Provider" ]; }
              { field = "scope.executionRef"; value = scope.executionRef or null; types = [ "Host" "Guest" ]; }
              { field = "scope.userRef"; value = scope.userRef or null; types = [ "User" ]; }
              { field = "identityGuestRef"; value = spec.identityGuestRef or null; types = [ "Guest" ]; }
              { field = "loginEndpointRef"; value = spec.loginEndpointRef or null; types = [ "Endpoint" ]; }
            ];
          in [
            {
              assertion = rawSpec.defaultDomain == "system"
                && rawSpec.allowedDomains == [ "system" ]
                && rawSpec.defaultUserRef == null
                && rawSpec.networkAttachments == [ ]
                && rawSpec.deviceAttachments == [ ]
                && rawSpec.volumeAttachmentDefaults == [ ];
              message = "${path}.spec overrides Credential execution policy defaults. Remove those overrides so the Credential remains system-only with no default user or network, device, or Volume attachments.";
            }
            {
              assertion = exactKeys [
                "providerRef"
                "updatePolicy"
                "scope"
                "audience"
                "consumerRef"
                "allowedOperations"
                "rotation"
                "expiry"
                "revocation"
                "identityGuestRef"
                "loginEndpointRef"
                "provider"
              ] spec;
              message = "${path}.spec contains an unsupported field. Remove fields not declared by the Credential ResourceType schema.";
            }
            {
              assertion = exactKeys [ "executionRef" "domainFilter" "userRef" ] scope
                && optionalRef (scope.executionRef or null)
                && optionalRef (scope.userRef or null)
                && builtins.elem domainFilter [ null "system" "user" ];
              message = "${path}.spec.scope is invalid. Keep only executionRef, domainFilter, and userRef; use resource references and set domainFilter to null, system, or user.";
            }
            {
              assertion = exactKeys [ "policy" "proactiveWindowMs" "maxLeaseLifetimeMs" ] rotation
                && builtins.elem (rotation.policy or "on-expiry")
                  [ "on-expiry" "proactive" "on-demand" ]
                && (proactiveWindow == null || builtins.isInt proactiveWindow)
                && builtins.isInt maximumLifetime;
              message = "${path}.spec.rotation is invalid. Keep only policy, proactiveWindowMs, and maxLeaseLifetimeMs; select on-expiry, proactive, or on-demand and use integer durations.";
            }
            {
              assertion = exactKeys [ "hardDeadlineMs" ] (spec.expiry or { })
                && builtins.isInt ((spec.expiry or { }).hardDeadlineMs or 0);
              message = "${path}.spec.expiry is invalid. Keep only hardDeadlineMs and set it to an integer duration.";
            }
            {
              assertion = exactKeys [ "onOwnerDelete" "onProviderGeneration" ] (spec.revocation or { })
                && builtins.elem ((spec.revocation or { }).onOwnerDelete or "immediate")
                  [ "immediate" "drain-leases" ]
                && builtins.elem ((spec.revocation or { }).onProviderGeneration or "immediate")
                  [ "immediate" "drain-leases" ];
              message = "${path}.spec.revocation is invalid. Keep only onOwnerDelete and onProviderGeneration, and set each to immediate or drain-leases.";
            }
            {
              assertion = spec ? providerRef && provider != null;
              message = "${path}.spec.providerRef must resolve to a Provider in Zone ${zoneName}.";
            }
            {
              assertion = artifactId != null && artifact != null;
              message = "${path}.spec.providerRef must select a Provider with a declared artifactId.";
            }
            {
              assertion = artifact != null && artifact.type == "provider";
              message = "${path}.spec.providerRef artifactId must resolve to a provider artifact.";
            }
            {
              assertion = spec ? audience
                && builtins.isString spec.audience
                && builtins.stringLength spec.audience <= 256
                && builtins.match audiencePattern spec.audience != null;
              message = "${path}.spec.audience must match ${audiencePattern} and be at most 256 bytes.";
            }
            {
              assertion = domainFilter == null || builtins.elem domainFilter supportedDomains;
              message = "${path}.spec.scope.domainFilter is not supported by its Provider. Set it to null or to a domain declared by the referenced Provider's config.credentialDomains.";
            }
            {
              assertion = domainFilter != "user" || scope.userRef or null != null;
              message = "${path}.spec.scope.userRef is required for the user domain.";
            }
            {
              assertion = rotation.policy or "on-expiry" != "proactive"
                || (proactiveWindow != null && proactiveWindow > 0 && maximumLifetime > 0);
              message = "${path}.spec.rotation proactive policy requires positive proactiveWindowMs and maxLeaseLifetimeMs.";
            }
            {
              assertion = proactiveWindow == null
                || maximumLifetime == 0
                || proactiveWindow < maximumLifetime / 2;
              message = "${path}.spec.rotation.proactiveWindowMs must be less than half maxLeaseLifetimeMs.";
            }
            {
              assertion = allowedOperations != [ ]
                && lib.length allowedOperations == lib.length (lib.unique allowedOperations)
                && lib.all (operation: builtins.elem operation credentialOperations) allowedOperations
                && lib.all (operation: builtins.elem operation supportedOperations) allowedOperations;
              message = "${path}.spec.allowedOperations must be a non-empty unique subset of its Provider supportedOperations.";
            }
            {
              assertion = !(providerRef != null
                && providerRef.name == "credential-secret-service"
                && domainFilter == "system");
              message = "${path}: credential-secret-service does not support system-domain placement. Set ${path}.spec.scope.domainFilter to user or select a Provider that declares the system domain.";
            }
            {
              assertion = !(providerRef != null
                && providerRef.name == "credential-managed-identity"
                && domainFilter == "user");
              message = "${path}: credential-managed-identity does not support user-domain placement. Set ${path}.spec.scope.domainFilter to system or select a Provider that declares the user domain.";
            }
            {
              assertion = lib.all (value: !containsSensitiveShape value) (stringsIn spec);
              message = "${path}.spec contains secret-shaped material; use a Credential reference instead.";
            }
            {
              assertion = !(spec ? managedBy) && !(spec ? configurationGeneration);
              message = "${path}.spec must not author runtime management metadata.";
            }
          ] ++ map
            (check: {
              assertion = check.value == null || resolvesAs resources check.types check.value;
              message = "${path}.spec.${check.field} must resolve in Zone ${zoneName}.";
            })
            refChecks)
        credentials);
    in perCredential ++ [{
      assertion = lib.length bindings == lib.length (lib.unique bindings);
      message = "d2b.zones.${zoneName}.resources contains a duplicate Credential binding tuple. Change providerRef, scope, or audience on one Credential so every binding tuple is unique.";
    }];

  allCredentialAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone: credentialAssertions zoneName zone.resources)
    cfg.zones);

  credentialRefAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone: lib.mapAttrsToList
      (resourceName: resource:
        let violations = credentialRefViolations [ "spec" ] resource.spec;
        in {
          assertion = violations == [ ];
          message =
            "d2b.zones.${zoneName}.resources.${resourceName}: credential-value-must-be-ref. Set each named field to a Credential/<name> reference or remove the field"
            + lib.optionalString (violations != [ ])
              " (${lib.concatStringsSep ", " violations})";
        })
      zone.resources)
    cfg.zones);
in
{
  options.d2b.zones = lib.mkOption {
    type = types.attrsOf (types.submodule {
      options = {
        retainedGenerations = lib.mkOption {
          type = types.ints.between 1 16;
          default = 3;
          description = ''
            Number of prior configuration bundles retained for rollback.
            Retention is count-bounded and has no time-based expiry.
          '';
        };

        resources = lib.mkOption {
          type = types.attrsOf (types.submodule {
            options.metadata = lib.mkOption {
              type = types.submodule {
                options = {
                  labels = lib.mkOption {
                    type = types.attrsOf types.str;
                    default = { };
                    description = "Optional presentation labels.";
                  };
                  annotations = lib.mkOption {
                    type = types.attrsOf types.str;
                    default = { };
                    description = "Optional presentation annotations.";
                  };
                };
              };
            };
          });
        };
      };
    });
  };

  options.d2b._resourceCompiler = lib.mkOption {
    type = types.attrsOf types.anything;
    default = { };
    internal = true;
    visible = false;
    description = "Internal configuration-publication compiler contract.";
  };

  config.assertions = allCredentialAssertions ++ credentialRefAssertions;
}
