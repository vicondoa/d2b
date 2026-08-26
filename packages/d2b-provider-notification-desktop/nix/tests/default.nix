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
    display-wayland = { type = "Provider"; spec = { }; };
    guest = { type = "Guest"; };
    notification-desktop = {
      type = "Provider";
      spec.config = {
        hostExecutionRef = "Host/host";
        hostUserRef = "User/user";
        displayWaylandRef = "Provider/display-wayland";
        guestSources = [ { guestRef = "Guest/guest"; categories = [ "security.event" ]; } ];
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
          notification-desktop.spec.config.guestSources = [ {
            guestRef = "Guest/guest";
            categories = [ "not-a-category" ];
          } ];
        }; }
    ];
  };
  projected = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          host = { type = "Host"; spec = { }; };
          user = { type = "User"; spec = { }; };
          guest = { type = "Guest"; spec = { }; };
          notification-desktop = {
            type = "Provider";
            spec.config = {
              hostExecutionRef = "Host/host";
              hostUserRef = "User/user";
              guestSources = [{
                guestRef = "Guest/guest";
                categories = [ "system.info" ];
              }];
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
        config.d2b.zones.dev.resources.notification-desktop = {
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
    "provider-notification-desktop/modules-evaluate" = {
      expr = builtins.deepSeq valid.config.assertions true;
      expected = true;
      propagateError = true;
    };
    "provider-notification-desktop/valid-source-and-admission" = {
      expr = allTrue valid;
      expected = true;
    };
    "provider-notification-desktop/rejects-unknown-category" = {
      expr = anyFalse invalid;
      expected = true;
    };
    "provider-notification-desktop/projects-guest-source" = {
      expr = {
        processes = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionNotificationDesktop.processesByZone.dev);
        endpoint = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionNotificationDesktop.resourcesByZone.dev);
      };
      expected = {
        processes = [ "notification-guest-guest" "notification-host" ];
        endpoint = [ "notification-sink" ];
      };
    };
    "provider-notification-desktop/rejects-unknown-provider-field" = {
      expr = lib.any
        (record: !record.assertion)
        invalidProjection.config.assertions;
      expected = true;
    };
  };
}
