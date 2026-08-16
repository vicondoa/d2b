# Eval-time validation for notification-desktop Provider configuration.
{ config, lib, ... }:

let
  cfg = config.d2b;
  categories = [
    "device.added" "device.removed" "device.error"
    "network.connected" "network.disconnected" "network.error"
    "presence.online" "presence.offline"
    "security.event" "security.error"
    "transfer.complete" "transfer.error" "transfer.cancelled"
    "update.available" "update.downloading" "update.ready" "update.error"
    "system.info" "system.warning" "system.error"
  ];
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
    (row: row.resource.type == "Provider" && row.resourceName == "notification-desktop")
    rows;
  assertionsFor = row:
    let
      spec = row.resource.spec or { };
      c = spec.config or { };
      sources = c.guestSources or [ ];
      sourceAssertions = lib.concatMap
        (rawSource:
          let source = if builtins.isAttrs rawSource then rawSource else { };
          in [
          {
            assertion = resolves row.zoneName "Guest" (source.guestRef or null);
            message = "${row.path}.spec.config.guestSources guestRef must resolve to a same-Zone Guest.";
          }
          {
            assertion = (source.categories or [ ]) != [ ]
              && lib.all (category: builtins.elem category categories) (source.categories or [ ]);
            message = "${row.path}.spec.config.guestSources contains an invalid category.";
          }
        ])
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
        assertion = resolvesExact row.zoneName "Provider" "display-wayland" (c.displayWaylandRef or null);
        message = "${row.path}.spec.config.displayWaylandRef must select Provider/display-wayland.";
      }
      {
        assertion = (c.maxPendingNotifications or 64) >= 8
          && (c.maxPendingNotifications or 64) <= 1024;
        message = "${row.path}.spec.config.maxPendingNotifications must be between 8 and 1024.";
      }
      {
        assertion = (c.actionNonceTtlSecs or 120) >= 30
          && (c.actionNonceTtlSecs or 120) <= 600;
        message = "${row.path}.spec.config.actionNonceTtlSecs must be between 30 and 600.";
      }
      {
        assertion = (c.actionNonceStoreSize or 256) >= 64
          && (c.actionNonceStoreSize or 256) <= 4096;
        message = "${row.path}.spec.config.actionNonceStoreSize must be between 64 and 4096.";
      }
      {
        assertion = (c.acknowledgeTimeoutSecs or 3600) >= 1
          && (c.acknowledgeTimeoutSecs or 3600) <= 86400;
        message = "${row.path}.spec.config.acknowledgeTimeoutSecs must be between 1 and 86400.";
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
