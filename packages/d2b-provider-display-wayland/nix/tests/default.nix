{ lib, ... }:

let
  base = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.anything;
      default = [ ];
    };
    options.d2b.zones = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.d2b._resourceCompiler = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
  };
  resources = {
    host = { type = "Host"; };
    user = { type = "User"; };
    guest = { type = "Guest"; };
    display-wayland = { type = "Provider"; spec.config.principalPoolSize = 4; };
    policy = { type = "display-wayland.d2bus.org.WaylandPolicy"; spec = { }; };
    session = {
      type = "display-wayland.d2bus.org.WaylandSession";
      spec = {
        guestRef = "Guest/guest";
        hostRef = "Host/host";
        userRef = "User/user";
        policyRef = "display-wayland.d2bus.org.WaylandPolicy/policy";
        crossDomainTrusted = true;
        identity = { activeColor = "#7fc8ff"; inactiveColor = "#45475a"; urgentColor = "#f38ba8"; };
        filter = { allowGlobals = [ "wl_compositor" ]; denyGlobals = [ ]; };
      };
    };
  };
  valid = lib.evalModules {
    modules = [ base (import ../default.nix) { config.d2b.zones.dev.resources = resources; } ];
  };
  invalid = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      { config.d2b.zones.dev.resources = lib.recursiveUpdate resources {
          session.spec.crossDomainTrusted = false;
        }; }
    ];
  };
  projected = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          display-wayland = { type = "Provider"; spec = { }; };
          host = { type = "Host"; spec = { }; };
          guest = { type = "Guest"; spec = { }; };
          session = {
            type = "display-wayland.d2bus.org.WaylandSession";
            spec = {
              guestRef = "Guest/guest";
              hostRef = "Host/host";
              userRef = "User/alice";
            };
          };
        };
      }
    ];
  };
  invalidProjection = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.display-wayland = {
          type = "Provider";
          spec.config.unsupported = true;
        };
      }
    ];
  };
  allTrue = value: lib.all (assertion: assertion.assertion) value.config.assertions;
  anyFalse = value: lib.any (assertion: !assertion.assertion) value.config.assertions;
in
{
  cases = {
    "provider-display-wayland/modules-evaluate" = {
      expr = builtins.deepSeq valid.config.assertions true;
      expected = true;
      propagateError = true;
    };
    "provider-display-wayland/valid-session-and-policy" = {
      expr = allTrue valid;
      expected = true;
    };
    "provider-display-wayland/rejects-untrusted-session" = {
      expr = anyFalse invalid;
      expected = true;
    };
    "provider-display-wayland/session-process-and-endpoint" = {
      expr = {
        processes = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionDisplayWayland.processesByZone.dev);
        resources = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionDisplayWayland.resourcesByZone.dev);
      };
      expected = {
        processes = [ "wayland-frontend-session" "wayland-proxy-session" ];
        resources = [ "wayland-session" ];
      };
    };
    "provider-display-wayland/rejects-unknown-provider-field" = {
      expr = lib.any
        (record: !record.assertion)
        invalidProjection.config.assertions;
      expected = true;
    };
  };
}
