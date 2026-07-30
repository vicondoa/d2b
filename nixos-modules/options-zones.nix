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

  resourceAssertions = zoneName: resources:
    lib.flatten (lib.mapAttrsToList
      (resourceName: resource:
        let
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
          policy = resource.spec;
          isExecutionTarget =
            resource.type == "Host" || resource.type == "Guest";
          canonicalRef = "${resource.type}/${resourceName}";
          defaultNetworks =
            lib.filter (attachment: attachment.default) policy.networkAttachments;
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
        ])
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

  config.assertions = zoneAssertions;
}
