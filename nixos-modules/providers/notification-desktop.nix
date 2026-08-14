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
      dbusEnabled = c.dbusSinkEnabled or true;
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
        assertion = !dbusEnabled || resolves row.zoneName "User" (c.hostUserRef or null);
        message = "${row.path}.spec.config.hostUserRef must resolve to a same-Zone User when D-Bus is enabled.";
      }
      {
        assertion = !dbusEnabled || (
          resolves row.zoneName "Provider" (c.displayWaylandRef or null)
        );
        message = "${row.path}.spec.config.displayWaylandRef must select Provider/display-wayland when D-Bus is enabled.";
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
    ] ++ sourceAssertions;
in
{
  config.assertions = lib.concatMap assertionsFor providerRows;
}
