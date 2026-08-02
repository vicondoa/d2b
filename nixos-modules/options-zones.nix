{ config, lib, ... }:

let
  cfg = config.d2b;
  resourceTypes = import ./resources.nix { inherit lib; };

  zoneNamePattern = "^[a-z][a-z0-9-]{0,62}$";

  zoneNameType = lib.types.strMatching zoneNamePattern;

  # The distinguished local root Zone. It is the one Zone that must not
  # declare a parent; every other Zone must.
  localRootZoneName = "local-root";

  # Maximum number of Zone names on one compiler-authored ancestry path,
  # counting the Zone itself and the local root.
  maxAncestryNames = 16;

  trustedPublisherType = lib.types.submodule {
    freeformType = null;
    options.signingKey = lib.mkOption {
      type = lib.types.str;
      description = "Publisher verification key used only by the Nix compiler.";
    };
  };

  # Parse a "Type/name" reference, or report that it is not one.
  #
  # The type and length checks are load-bearing, not defensive noise. These
  # helpers back assertions, and an assertion exists to report a misconfigured
  # value clearly. Indexing a split without checking its shape turns a bad ref
  # into a fatal evaluation abort - "expected a string" or an out-of-bounds
  # index - which reports the wrong problem and buries the offending option
  # path. Returning null lets every caller answer false and let its own
  # assertion produce the real message.
  parseRef = ref:
    let
      parts = if builtins.isString ref then lib.splitString "/" ref else [ ];
    in
    if lib.length parts == 2 then
      {
        type = builtins.elemAt parts 0;
        name = builtins.elemAt parts 1;
      }
    else
      null;

  resolvesAs = resources: expectedType: ref:
    let
      parsed = parseRef ref;
    in
    parsed != null
    && parsed.type == expectedType
    && builtins.hasAttr parsed.name resources
    && resources.${parsed.name}.type == expectedType;

  resolvesExactly = resources: ref:
    let
      parsed = parseRef ref;
    in
    parsed != null
    && builtins.hasAttr parsed.name resources
    && resources.${parsed.name}.type == parsed.type;

  artifactFor = artifactId:
    if builtins.isString artifactId
      && builtins.hasAttr artifactId (cfg.artifacts or { })
    then cfg.artifacts.${artifactId}
    else null;

  artifactAssertions = zoneName: resourceName: resource:
    let
      path = "d2b.zones.${zoneName}.resources.${resourceName}";
      spec = resource.spec or { };
      source = spec.source or { };
      systemIds = lib.filter (value: value != null) [
        (spec.systemArtifactId or null)
        (source.systemArtifactId or null)
      ];
      artifactsDeclared = (cfg.artifacts or { }) != { };
      providerArtifact = artifactFor (spec.artifactId or null);
      providerCatalog = cfg.providerCatalog or { };
      providerCatalogMatches = lib.filter
        (entry:
          let
            selected = entry.artifactId or (entry.entry.artifactId or null);
          in selected == (spec.artifactId or null))
        (lib.attrValues providerCatalog);
    in
      lib.optionals (artifactsDeclared && resource.type == "Provider") [
        {
          assertion = builtins.isString (spec.artifactId or null)
            && builtins.match "^[a-z][a-z0-9-]*$" spec.artifactId != null;
          message = "${path}.spec.artifactId must be a bounded plain artifact ID.";
        }
        {
          assertion = providerArtifact != null
            && (providerArtifact.type or null) == "provider";
          message = "${path}.spec.artifactId must resolve to a provider artifact.";
        }
        {
          assertion = providerCatalog == { }
            || lib.length providerCatalogMatches == 1;
          message = "${path}.spec.artifactId must resolve to exactly one provider catalog entry.";
        }
      ]
      ++ lib.optionals (artifactsDeclared && systemIds != [ ]) [
        {
          assertion = lib.all
            (artifactId:
              let artifact = artifactFor artifactId;
              in builtins.isString artifactId
                && builtins.match "^[a-z][a-z0-9-]*$" artifactId != null
                && artifact != null
                && (artifact.type or null) == "nixos-system")
            systemIds;
          message = "${path}: systemArtifactId and source.systemArtifactId must resolve to nixos-system artifacts.";
        }
      ];

  resourceAssertions = zoneName: resources:
    lib.flatten (lib.mapAttrsToList
      (resourceName: resource:
        let
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
          policy = resource.spec;
          isExecutionTarget =
            resource.type == "Host" || resource.type == "Guest";
          isProcess =
            resource.type == "Process" || resource.type == "EphemeralProcess";
          canonicalRef = "${resource.type}/${resourceName}";
          defaultNetworks =
            lib.filter (attachment: attachment.default) policy.networkAttachments;
          executionTarget =
            let parsed = parseRef (policy.executionRef or null);
            in if parsed != null && builtins.hasAttr parsed.name resources
              then resources.${parsed.name}
              else null;
          targetHasNoIsolation =
            isProcess
            && executionTarget != null
            && executionTarget.type == "Host"
            && (executionTarget.spec.isolationPosture or null) == "none";
        in
        [
          {
            assertion = builtins.match zoneNamePattern resourceName != null;
            message = "${path}: resource name must match ${zoneNamePattern}.";
          }
          {
            assertion =
              resource.metadata.ownerRef == null
              || resolvesExactly resources resource.metadata.ownerRef;
            message = "${path}.metadata.ownerRef must resolve in Zone ${zoneName}.";
          }
          {
            assertion = resource.metadata.ownerRef != canonicalRef;
            message = "${path}.metadata.ownerRef must not refer to the resource itself.";
          }
          {
            assertion = !isExecutionTarget || policy.providerRef != null;
            message = "${path}.spec.providerRef is required for Host and Guest resources.";
          }
          {
            assertion =
              policy.providerRef == null
              || resolvesAs resources "Provider" policy.providerRef;
            message = "${path}.spec.providerRef must resolve to a Provider in Zone ${zoneName}.";
          }
          {
            assertion =
              !isExecutionTarget
              || (policy.allowedDomains != [ ]
                && lib.length policy.allowedDomains <= 2
                && lib.length (lib.unique policy.allowedDomains)
                  == lib.length policy.allowedDomains);
            message = "${path}.spec.allowedDomains must contain one or two unique domains.";
          }
          {
            assertion =
              !isExecutionTarget
              || builtins.elem policy.defaultDomain policy.allowedDomains;
            message = "${path}.spec.defaultDomain must be present in allowedDomains.";
          }
          {
            assertion =
              !isExecutionTarget
              || !(builtins.elem "user" policy.allowedDomains)
              || policy.defaultUserRef != null;
            message = "${path}.spec.defaultUserRef is required when allowedDomains contains user.";
          }
          {
            assertion =
              !isExecutionTarget
              || policy.defaultUserRef == null
              || resolvesAs resources "User" policy.defaultUserRef;
            message = "${path}.spec.defaultUserRef must resolve to a User in Zone ${zoneName}.";
          }
          {
            assertion =
              !isExecutionTarget || lib.length policy.networkAttachments <= 64;
            message = "${path}.spec.networkAttachments must contain at most 64 entries.";
          }
          {
            assertion = !isExecutionTarget || lib.length defaultNetworks <= 1;
            message = "${path}.spec.networkAttachments may contain at most one default.";
          }
          {
            assertion =
              !isExecutionTarget
              || lib.all
                (attachment: resolvesAs resources "Network" attachment.networkRef)
                policy.networkAttachments;
            message = "${path}.spec.networkAttachments must resolve to Networks in Zone ${zoneName}.";
          }
          {
            assertion =
              !isExecutionTarget || lib.length policy.deviceAttachments <= 64;
            message = "${path}.spec.deviceAttachments must contain at most 64 entries.";
          }
          {
            assertion =
              !isExecutionTarget
              || lib.all
                (attachment: resolvesAs resources "Device" attachment.deviceRef)
                policy.deviceAttachments;
            message = "${path}.spec.deviceAttachments must resolve to Devices in Zone ${zoneName}.";
          }
          {
            assertion =
              !isExecutionTarget || lib.length policy.volumeAttachmentDefaults <= 64;
            message = "${path}.spec.volumeAttachmentDefaults must contain at most 64 entries.";
          }
          {
            assertion = !targetHasNoIsolation || (policy.domain or null) == "user";
            message = "${path}.spec.domain must be user for a no-isolation Host target.";
          }
          {
            assertion =
              !isProcess
              || (policy.domain or null) != "user"
              || (policy.userRef or null) != null
              || (executionTarget != null
                && (executionTarget.spec.defaultUserRef or null) != null);
            message = "${path}.spec.userRef is required for user-domain execution when the target has no default user.";
          }
        ]
        ++ artifactAssertions zoneName resourceName resource)
      resources);

  zoneAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone:
      [
        {
          assertion = builtins.match zoneNamePattern zoneName != null;
          message = "d2b.zones.${zoneName}: Zone name must match ${zoneNamePattern}.";
        }
      ]
      ++ resourceAssertions zoneName zone.resources)
    cfg.zones);
  zoneLinks = zone:
    lib.filterAttrs (_: resource: resource.type == "ZoneLink") zone.resources;

  topologyEnabled =
    builtins.hasAttr localRootZoneName cfg.zones
    || lib.any (zone: zone.parentZone != null) (lib.attrValues cfg.zones);

  parentWalk = current: seen:
    if current == null then
      {
        cycle = false;
        missing = false;
        depthExceeded = false;
        path = seen;
      }
    else if builtins.elem current seen then
      {
        cycle = true;
        missing = false;
        depthExceeded = false;
        path = seen ++ [ current ];
      }
    else if !builtins.hasAttr current cfg.zones then
      {
        cycle = false;
        missing = true;
        depthExceeded = false;
        path = seen ++ [ current ];
      }
    else if lib.length seen >= maxAncestryNames then
      {
        cycle = false;
        missing = false;
        depthExceeded = true;
        path = seen;
      }
    else
      parentWalk cfg.zones.${current}.parentZone (seen ++ [ current ]);

  topologyAssertions =
    if !topologyEnabled then
      [ ]
    else
      lib.flatten (lib.mapAttrsToList
        (zoneName: zone:
          let walk = parentWalk zoneName [ ];
          in [
            {
              assertion =
                if zoneName == localRootZoneName
                then zone.parentZone == null
                else zone.parentZone != null;
              message =
                if zoneName == localRootZoneName
                then "d2b.zones.${zoneName}.parentZone is forbidden on the local-root Zone."
                else "d2b.zones.${zoneName}.parentZone is required for every non-root Zone.";
            }
            {
              assertion =
                zone.parentZone == null
                || builtins.hasAttr zone.parentZone cfg.zones;
              message = "d2b.zones.${zoneName}.parentZone must resolve to a declared Zone.";
            }
            {
              assertion = zone.parentZone == null || zone.parentZone != zoneName;
              message = "d2b.zones.${zoneName}.parentZone must not name itself.";
            }
            {
              assertion = !walk.cycle;
              message = "d2b.zones.${zoneName}.parentZone forms a cycle.";
            }
            {
              assertion = !walk.depthExceeded;
              message = "d2b.zones.${zoneName}.parentZone ancestry exceeds ${toString maxAncestryNames} Zone names.";
            }
          ])
        cfg.zones);

  zoneLinkAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone:
      let
        links = zoneLinks zone;
        path = "d2b.zones.${zoneName}.resources";
      in
      [
        {
          assertion = zoneName != localRootZoneName || links == { };
          message = "${path}: local-root must not declare a ZoneLink resource.";
        }
        {
          assertion = !topologyEnabled || zoneName == localRootZoneName || lib.length (lib.attrNames links) <= 1;
          message = "${path}: a child Zone may declare at most one ZoneLink resource.";
        }
      ]
      ++ lib.flatten (lib.mapAttrsToList
        (resourceName: resource:
          let
            spec = resource.spec or { };
            providerRef = spec.transportProviderRef or null;
          in [
            {
              assertion = spec.childZoneName or null == zoneName;
              message = "${path}.${resourceName}.spec.childZoneName must equal its enclosing Zone name.";
            }
            {
              assertion = providerRef != null && resolvesAs zone.resources "Provider" providerRef;
              message = "${path}.${resourceName}.spec.transportProviderRef must resolve to a Provider in Zone ${zoneName}.";
            }
          ])
        links))
    cfg.zones);

  ownerCycleFor = zoneName: resources: resourceName:
    let
      walk = current: seen:
        if !builtins.hasAttr current resources then false
        else if builtins.elem current seen then true
        else
          let owner = resources.${current}.metadata.ownerRef or null;
          in owner != null
            && (let parsed = parseRef owner;
                in parsed != null && walk parsed.name (seen ++ [ current ]));
    in
    walk resourceName [ ];

  ownerCycleAssertions = lib.flatten (lib.mapAttrsToList
    (zoneName: zone:
      lib.mapAttrsToList
        (resourceName: _resource: {
          assertion = !ownerCycleFor zoneName zone.resources resourceName;
          message = "d2b.zones.${zoneName}.resources.${resourceName}.metadata.ownerRef forms an owner cycle.";
        })
        zone.resources)
    cfg.zones);
in
{
  options.d2b.zones = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule {
      freeformType = null;
      options.parentZone = lib.mkOption {
        type = lib.types.nullOr zoneNameType;
        default = null;
        example = "local-root";
        description = ''
          Compiler-only parent Zone name. Required for every non-root Zone
          and forbidden on the distinguished local root Zone
          "${localRootZoneName}". This is not a ResourceRef: it never enters
          a ResourceSpec and is emitted only into the sealed allocator
          bootstrap topology, never into Zone.spec.

          The value must name another declared Zone, must differ from the
          Zone declaring it, and the complete child-to-parent graph must be
          acyclic with at most ${toString maxAncestryNames} Zone names on any
          ancestry path. Conflicting definitions fail through normal Nix
          module merging.
        '';
      };
      options.label = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = ''
          Human-readable Zone label. This compiler setting is not part of the
          runtime-created Zone self-resource spec.
        '';
      };
      options.trustedPublishers = lib.mkOption {
        type = lib.types.attrsOf trustedPublisherType;
        default = { };
        description = ''
          Additional Provider publisher roots trusted for this Zone. Keys and
          signing material are compiler inputs, never ResourceSpec fields.
        '';
      };
      options.resources = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule resourceTypes.resourceModule);
        default = { };
        description = ''
          Zone-local resources, keyed by ResourceName. Each entry mirrors the
          canonical ResourceSpec shape for its ResourceType; there is no
          second Nix vocabulary and no extra nesting.
        '';
      };
    });
    default = { };
    description = "Zone-local resource identity and authoring declarations.";
  };

  options.d2b._zoneCompiler = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
    description = "Internal v3 Zone topology/compiler projection.";
  };

  config = {
    assertions = zoneAssertions ++ topologyAssertions ++ zoneLinkAssertions ++ ownerCycleAssertions;
    d2b._zoneCompiler = {
      localRoot = localRootZoneName;
      maxAncestryNames = maxAncestryNames;
      topology = lib.listToAttrs (lib.mapAttrsToList
        (zoneName: zone:
          lib.nameValuePair zoneName {
            parentZone = zone.parentZone;
            label = zone.label;
            retainedGenerations = zone.retainedGenerations;
            trustedPublishers = zone.trustedPublishers;
            stateDir = "${toString cfg.site.stateDir}/zones/${zoneName}";
          })
        cfg.zones);
      selfResources = lib.mapAttrs
        (zoneName: _zone: {
          apiVersion = "resources.d2bus.org/v3";
          type = "Zone";
          metadata = {
            name = zoneName;
            zone = zoneName;
          };
          spec = { };
        })
        cfg.zones;
    };
  };
}
