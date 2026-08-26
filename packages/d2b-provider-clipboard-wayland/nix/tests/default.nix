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
    clipboard-wayland = {
      type = "Provider";
      spec.config = {
        hostExecutionRef = "Host/host";
        hostUserRef = "User/user";
        displayWaylandRef = "Provider/display-wayland";
        caps = { maxHistoryEntries = 20; maxItemBytes = 8388608; maxTotalBytes = 67108864; };
        policy = { crossZone.enable = false; };
        guestSources = [ { guestRef = "Guest/guest"; } ];
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
          clipboard-wayland.spec.config.policy.crossZone.enable = true;
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
          clipboard-wayland = {
            type = "Provider";
            spec.config = {
              hostExecutionRef = "Host/host";
              hostUserRef = "User/user";
              guestSources = [ { guestRef = "Guest/guest"; } ];
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
        config.d2b.zones.dev.resources.clipboard-wayland = {
          type = "Provider";
          spec.config = {
            unsupported = true;
          };
        };
      }
    ];
  };
  allTrue = value: lib.all (assertion: assertion.assertion) value.config.assertions;
  anyFalse = value: lib.any (assertion: !assertion.assertion) value.config.assertions;
in
{
  cases = {
    "provider-clipboard-wayland/modules-evaluate" = {
      expr = builtins.deepSeq valid.config.assertions true;
      expected = true;
      propagateError = true;
    };
    "provider-clipboard-wayland/valid-placement-and-policy" = {
      expr = allTrue valid;
      expected = true;
    };
    "provider-clipboard-wayland/rejects-cross-zone-transfer" = {
      expr = anyFalse invalid;
      expected = true;
    };
    "provider-clipboard-wayland/projects-guest-source" = {
      expr = {
        processes = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionClipboardWayland.processesByZone.dev);
        endpoint = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionClipboardWayland.resourcesByZone.dev);
      };
      expected = {
        processes = [ "clipboard-guest-guest" "clipboard-host" ];
        endpoint = [ "clipboard-bridge" ];
      };
    };
    "provider-clipboard-wayland/rejects-unknown-provider-field" = {
      expr = lib.any
        (record: !record.assertion)
        invalidProjection.config.assertions;
      expected = true;
    };
  };
}
