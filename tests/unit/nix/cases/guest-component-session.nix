# Nix-unit coverage for the ordinary Guest ComponentSession package shape.
#
# The package inputs are deliberately inert derivations. This case checks the
# mode-bound systemd contract without selecting a provider-specific enrollment
# implementation that U7/U8 still own.
{ mkGuestEval, lib, pkgs, flakeRoot, ... }:

let
  d2bd = pkgs.runCommand "d2bd-guest-component-session-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2bd"
  '';
  broker = pkgs.runCommand "d2b-broker-guest-component-session-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2b-broker"
  '';
  optionSinks = { lib, ... }: {
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.tmpfiles.rules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
    options.users.users = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.users.groups = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
  };
  hostTools = { inherit d2bd broker; };
  evaluated = (mkGuestEval {
    modules = [
      optionSinks
      (import (flakeRoot + "/nixos-modules/component-session.nix"))
      ({ ... }: {
        d2b.componentSession = {
          enable = true;
          guestConfigPath = null;
          shell = {
            enable = false;
            defaultName = "default";
            maxSessions = 8;
            maxAttached = 1;
          };
        };
      })
    ];
    specialArgs = {
      d2bInputs = { };
      d2bHostTools = hostTools;
      name = "guest";
    };
  }).config;
  service = evaluated.systemd.services.d2bd-guest.serviceConfig;
  packagePaths = map toString evaluated.environment.systemPackages;
in
{
  "guest-component-session/starts-d2bd-guest" = {
    expr = lib.hasInfix "/bin/d2bd guest " service.ExecStart;
    expected = true;
  };

  "guest-component-session/uses-guest-broker-and-no-public-socket" = {
    expr = {
      broker = lib.hasInfix "--broker-socket /run/d2b/guest-broker.sock" service.ExecStart;
      public = lib.hasInfix "public.sock" service.ExecStart;
      localZone = lib.hasInfix "--config" service.ExecStart;
    };
    expected = {
      broker = true;
      public = false;
      localZone = false;
    };
  };

  "guest-component-session/binds-enrollment-inputs-at-start" = {
    expr = {
      guest = lib.hasInfix "--guest-ref Guest/guest" service.ExecStart;
      zone = lib.hasInfix "--zone local" service.ExecStart;
      schema = lib.hasInfix "--schema-fingerprint sha256:" service.ExecStart;
      privateKey = lib.hasInfix "--local-private-key /var/lib/d2b/component-session/guest.key"
        service.ExecStart;
      parentKey = lib.hasInfix "--parent-public-key /var/lib/d2b/component-session/parent.pub"
        service.ExecStart;
      writableBootId = lib.hasInfix "--boot-id-path" service.ExecStart;
    };
    expected = {
      guest = true;
      zone = true;
      schema = true;
      privateKey = true;
      parentKey = true;
      writableBootId = false;
    };
  };

  "guest-component-session/does-not-install-retired-guest-agent" = {
    expr = {
      package = lib.any (path: lib.hasInfix "d2b-guestd" path) packagePaths;
      service = builtins.hasAttr "d2b-guestd" evaluated.systemd.services;
      credential = builtins.hasAttr "LoadCredential" service;
    };
    expected = {
      package = false;
      service = false;
      credential = false;
    };
  };
}
