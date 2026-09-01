{ lib, ... }:

let
  base = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.anything;
      default = [ ];
    };
    options.d2b.zones = lib.mkOption {
      type = lib.types.attrs;
      default = { };
    };
    options.d2b._resourceCompiler = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
  };
  enabled = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources = {
          shell-terminal = { type = "Provider"; spec = { }; };
          guest = { type = "Guest"; spec = { }; };
          alice = { type = "User"; spec = { }; };
          shell = {
            type = "shell-terminal.d2bus.org.ShellPool";
            spec = {
              providerRef = "Provider/shell-terminal";
              executionRef = "Guest/guest";
              userRef = "User/alice";
            };
          };
        };
      }
    ];
  };
  invalid = lib.evalModules {
    modules = [
      base
      (import ../default.nix)
      {
        config.d2b.zones.dev.resources.shell-terminal = {
          type = "Provider";
          spec.config.unsupported = true;
        };
      }
    ];
  };
in
{
  cases = {
    "provider-shell-terminal/guest-process" = {
      expr = enabled.config.d2b._resourceCompiler
        .providerProjectionShellTerminal.processesByZone.dev
        ."shell-shell".spec.template;
      expected = "shell-supervisor-main";
    };

    "provider-shell-terminal/guest-process-is-zone-local" = {
      expr = enabled.config.d2b._resourceCompiler
        .providerProjectionShellTerminal.processesByZone.dev
        ."shell-shell".metadata.ownerRef;
      expected = "shell-terminal.d2bus.org.ShellPool/shell";
    };
    "provider-shell-terminal/rejects-provider-settings" = {
      expr = lib.any (record: !record.assertion) invalid.config.assertions;
      expected = true;
    };
  };
}
