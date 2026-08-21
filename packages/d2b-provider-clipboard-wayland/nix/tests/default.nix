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
  };
}
