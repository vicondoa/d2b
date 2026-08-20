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
  };
}
