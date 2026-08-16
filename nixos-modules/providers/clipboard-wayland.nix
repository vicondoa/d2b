# Eval-time validation for clipboard-wayland Provider configuration.
{ config, lib, ... }:

let
  cfg = config.d2b;
  parseRef = value:
    if builtins.isString value then lib.splitString "/" value else [ ];
  resolves = zoneName: expectedType: value:
    let parts = parseRef value;
    in lib.length parts == 2
      && builtins.elemAt parts 0 == expectedType
      && builtins.hasAttr (builtins.elemAt parts 1) cfg.zones.${zoneName}.resources
      && cfg.zones.${zoneName}.resources.${builtins.elemAt parts 1}.type == expectedType;
  resolvesExact = zoneName: expectedType: expectedName: value:
    resolves zoneName expectedType value
    && builtins.elemAt (parseRef value) 1 == expectedName;
  rows = lib.concatMap
    (zoneName:
      lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
        })
        cfg.zones.${zoneName}.resources)
    (lib.sort lib.lessThan (lib.attrNames (cfg.zones or { })));
  providerRows = lib.filter
    (row: row.resource.type == "Provider" && row.resourceName == "clipboard-wayland")
    rows;
  assertionsFor = row:
    let
      c = row.resource.spec.config or { };
      caps = c.caps or { };
      policy = c.policy or { };
      display = c.displayWaylandRef or null;
      sources = c.guestSources or [ ];
      sourceAssertions = lib.concatMap
        (rawSource:
          let source = if builtins.isAttrs rawSource then rawSource else { };
          in [{
            assertion = resolves row.zoneName "Guest" (source.guestRef or null);
            message = "${row.path}.spec.config.guestSources guestRef must resolve to a same-Zone Guest.";
          }])
        sources;
    in [
      {
        assertion = resolves row.zoneName "Host" (c.hostExecutionRef or null);
        message = "${row.path}.spec.config.hostExecutionRef must resolve to a same-Zone Host.";
      }
      {
        assertion = resolves row.zoneName "User" (c.hostUserRef or null);
        message = "${row.path}.spec.config.hostUserRef must resolve to a same-Zone User.";
      }
      {
        assertion = resolvesExact row.zoneName "Provider" "display-wayland" display;
        message = "${row.path}.spec.config.displayWaylandRef must select Provider/display-wayland.";
      }
      {
        assertion = (caps.maxHistoryEntries or 20) >= 1
          && (caps.maxHistoryEntries or 20) <= 200;
        message = "${row.path}.spec.config.caps.maxHistoryEntries must be between 1 and 200.";
      }
      {
        assertion = (caps.maxItemBytes or 8388608) >= 4096
          && (caps.maxItemBytes or 8388608) <= 67108864
          && (caps.maxTotalBytes or 67108864) >= (caps.maxItemBytes or 8388608);
        message = "${row.path}.spec.config.caps item and total byte bounds are invalid.";
      }
      {
        assertion = ((policy.crossZone or { }).enable or false) == false;
        message = "${row.path}.spec.config.policy.crossZone.enable must remain false.";
      }
      {
        assertion = lib.length sources >= 1 && lib.length sources <= 16;
        message = "${row.path}.spec.config.guestSources must contain between one and sixteen sources.";
      }
      {
        assertion =
          let guestRefs = map (source: (if builtins.isAttrs source then source else { }).guestRef or null) sources;
          in lib.length guestRefs == lib.length (lib.unique guestRefs);
        message = "${row.path}.spec.config.guestSources must not contain duplicate guestRef values.";
      }
    ] ++ sourceAssertions;
in
{
  config.assertions = lib.concatMap assertionsFor providerRows;
}
