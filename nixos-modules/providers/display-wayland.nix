# Eval-time validation for the display-wayland Provider resource surface.
{ config, lib, ... }:

let
  cfg = config.d2b;
  knownGlobals = [
    "wl_compositor"
    "wl_shm"
    "wl_seat"
    "wl_output"
    "wl_subcompositor"
    "xdg_wm_base"
    "wl_data_device_manager"
    "zwlr_data_control_manager_v1"
    "zwp_primary_selection_device_manager_v1"
    "zwp_linux_dmabuf_v1"
    "zwp_pointer_constraints_v1"
    "zwp_relative_pointer_manager_v1"
    "zwlr_layer_shell_v1"
    "wp_drm_lease_device_v1"
    "zwp_virtual_keyboard_manager_v1"
  ];
  refParts = value:
    if builtins.isString value
      then lib.splitString "/" value
      else [ ];
  resolves = zoneName: expectedType: value:
    let parts = refParts value;
    in lib.length parts == 2
      && builtins.elemAt parts 0 == expectedType
      && builtins.hasAttr (builtins.elemAt parts 1) cfg.zones.${zoneName}.resources
      && cfg.zones.${zoneName}.resources.${builtins.elemAt parts 1}.type == expectedType;
  resourceRows = lib.concatMap
    (zoneName:
      lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          path = "d2b.zones.${zoneName}.resources.${resourceName}";
        })
        cfg.zones.${zoneName}.resources)
    (lib.sort lib.lessThan (lib.attrNames (cfg.zones or { })));
  providerRows = lib.filter
    (row: row.resource.type == "Provider" && row.resourceName == "display-wayland")
    resourceRows;
  sessionRows = lib.filter
    (row: row.resource.type == "display-wayland.d2bus.org.WaylandSession")
    resourceRows;
  configAssertions = row:
    let
      providerConfig = row.resource.spec.config or { };
      poolSize = providerConfig.principalPoolSize or 4;
    in [
      {
        assertion = builtins.isInt poolSize && poolSize >= 1 && poolSize <= 32;
        message = "${row.path}.spec.config.principalPoolSize must be between 1 and 32.";
      }
    ];
  sessionAssertions = row:
    let
      spec = row.resource.spec or { };
      filter = spec.filter or { };
      allowGlobals = filter.allowGlobals or [ ];
      denyGlobals = filter.denyGlobals or [ ];
      colors = [
        ((spec.identity or { }).activeColor or "")
        ((spec.identity or { }).inactiveColor or "")
        ((spec.identity or { }).urgentColor or "")
      ];
    in [
      {
        assertion = spec.crossDomainTrusted or false;
        message = "${row.path}.spec.crossDomainTrusted must be true.";
      }
      {
        assertion = resolves row.zoneName "Guest" (spec.guestRef or null);
        message = "${row.path}.spec.guestRef must resolve to a same-Zone Guest.";
      }
      {
        assertion = resolves row.zoneName "Host" (spec.hostRef or null);
        message = "${row.path}.spec.hostRef must resolve to a same-Zone Host.";
      }
      {
        assertion = resolves row.zoneName "User" (spec.userRef or null);
        message = "${row.path}.spec.userRef must resolve to a same-Zone User.";
      }
      {
        assertion = resolves row.zoneName
          "display-wayland.d2bus.org.WaylandPolicy"
          (spec.policyRef or null);
        message = "${row.path}.spec.policyRef must be a qualified WaylandPolicy reference.";
      }
      {
        assertion = lib.all (color:
          builtins.isString color
          && builtins.match "^#[0-9a-fA-F]{6}$" color != null) colors;
        message = "${row.path}.spec.identity colors must use #rrggbb.";
      }
      {
        assertion = lib.all (value:
          builtins.elem value knownGlobals) (allowGlobals ++ denyGlobals);
        message = "${row.path}.spec.filter contains an unknown Wayland global.";
      }
    ];
in
{
  config.assertions =
    lib.concatMap configAssertions providerRows
    ++ lib.concatMap sessionAssertions sessionRows;
}
