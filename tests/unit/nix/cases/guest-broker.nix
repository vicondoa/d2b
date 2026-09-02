# Guest broker profile and instance-boundary evaluation contract.
#
# The fake host-tool package keeps this case focused on module composition.
# Host-broker's metadata-only overrideAttrs must still select the same output
# path as the Guest profile.
{ mkGuestEval, lib, pkgs, flakeRoot, ... }:

let
  broker = pkgs.runCommand "d2b-broker-guest-broker-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2b-broker"
  '';
  overrideBroker = pkgs.runCommand "d2b-broker-guest-broker-override-test" { } ''
    mkdir -p "$out/bin"
    touch "$out/bin/d2b-broker"
  '';
  d2bHostTools = { inherit broker; };
  hostToolOverrideKeys = [
    "d2b"
    "d2bd"
    "broker"
    "activationHelper"
    "hostActivationHelper"
    "unsafeLocalHelper"
    "resourceCompiler"
    "waylandProxy"
  ];
  d2bHostToolOverrides = lib.genAttrs hostToolOverrideKeys (_: overrideBroker);

  optionSinks = { lib, ... }: {
    options.d2b = lib.mkOption {
      type = lib.types.submodule {
        freeformType = lib.types.attrsOf lib.types.anything;
      };
      default = { };
    };
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
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
    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.sockets = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.slices = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = { };
    };
    options.systemd.tmpfiles.rules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
  };

  common = usePrebuilt: { ... }: {
    config = {
      d2b.site = {
        stateDir = "/var/lib/d2b";
        usePrebuiltHostTools = usePrebuilt;
        bundle = { };
        audit = {
          retentionDays = 14;
        };
      };
      d2b._index = {
        realms.enabledList = [ ];
      };
    };
  };

  eval = usePrebuilt: module:
    (mkGuestEval {
      modules = [
        optionSinks
        (common usePrebuilt)
        module
      ];
      specialArgs = {
        inherit d2bHostTools;
        d2bHostToolOverrides = null;
        name = "guest-test";
        d2bUsePrebuiltHostTools = usePrebuilt;
      };
    }).config;

  evalWithOverrides = module:
    (mkGuestEval {
      modules = [
        optionSinks
        (common false)
        module
      ];
      specialArgs = {
        inherit d2bHostTools d2bHostToolOverrides;
        name = "guest-test";
        d2bUsePrebuiltHostTools = false;
      };
    }).config;

  host = eval false (import (flakeRoot + "/nixos-modules/host-broker.nix") {
    inputs = { };
  });
  guest = eval false (import (flakeRoot + "/nixos-modules/guest-broker.nix"));
  prebuiltHost = eval true (import (flakeRoot + "/nixos-modules/host-broker.nix") {
    inputs = { };
  });
  prebuiltGuest = eval true (import (flakeRoot + "/nixos-modules/guest-broker.nix"));
  overriddenGuest =
    evalWithOverrides (import (flakeRoot + "/nixos-modules/guest-broker.nix"));

  brokerFrom = packages:
    lib.findFirst (package: package.outPath == broker.outPath) null packages;
  hostBroker = brokerFrom host.environment.systemPackages;
  guestBroker = brokerFrom guest.environment.systemPackages;
  prebuiltHostBroker = brokerFrom prebuiltHost.environment.systemPackages;
  prebuiltGuestBroker = brokerFrom prebuiltGuest.environment.systemPackages;
  prebuiltBroker =
    (import (flakeRoot + "/nix/prebuilt.nix") { inherit pkgs lib; })."d2b-broker";
  hostService = host.systemd.services.d2b-broker.serviceConfig;
  guestService = guest.systemd.services.d2b-broker-guest.serviceConfig;
  hostExecStart = hostService.ExecStart;
  guestExecStart = guestService.ExecStart;
  prebuiltHostExecStart =
    prebuiltHost.systemd.services.d2b-broker.serviceConfig.ExecStart;
  prebuiltGuestExecStart =
    prebuiltGuest.systemd.services.d2b-broker-guest.serviceConfig.ExecStart;
  hostSocket =
    host.systemd.sockets.d2b-broker.socketConfig.ListenSequentialPacket;
  guestSocket =
    guest.systemd.sockets.d2b-broker-guest.socketConfig.ListenSequentialPacket;
in
{
  "guest-broker/broker-package-out-path-equality" = {
    expr = hostBroker != null
      && guestBroker != null
      && hostBroker.outPath == guestBroker.outPath;
    expected = true;
  };
  "guest-broker/exec-start-binary-store-relation" = {
    expr = hostBroker != null
      && guestBroker != null
      && lib.hasPrefix "${hostBroker.outPath}/bin/d2b-broker host " hostExecStart
      && lib.hasPrefix "${guestBroker.outPath}/bin/d2b-broker guest " guestExecStart;
    expected = true;
  };
  "guest-broker/legacy-prebuilt-selects-built-profile-package" = {
    expr = prebuiltHostBroker != null
    && prebuiltGuestBroker != null
    && prebuiltHostBroker.outPath == broker.outPath
    && prebuiltGuestBroker.outPath == broker.outPath
    && prebuiltHostBroker.outPath != prebuiltBroker.outPath
    && lib.hasPrefix "${broker.outPath}/bin/d2b-broker host " prebuiltHostExecStart
    && lib.hasPrefix "${broker.outPath}/bin/d2b-broker guest " prebuiltGuestExecStart;
    expected = true;
  };
  "guest-broker/instance-roots-are-distinct" = {
    expr = hostSocket == "/run/d2b/priv.sock"
      && guestSocket == "/run/d2b/guest-broker.sock"
      && hostSocket != guestSocket
      && lib.hasInfix "--state-dir /var/lib/d2b" hostExecStart
      && lib.hasInfix "--state-dir /var/lib/d2b/guest-broker" guestExecStart
      && lib.hasInfix "--audit-dir /var/lib/d2b/audit" hostExecStart
      && lib.hasInfix "--audit-dir /var/lib/d2b/guest-audit" guestExecStart;
    expected = true;
  };
  "guest-broker/service-kill-mode-process" = {
    expr = guestService.KillMode;
    expected = "process";
  };
  "guest-broker/host-tool-override-selects-guest-broker" = {
    expr =
      let
        packages = overriddenGuest.environment.systemPackages;
        selected = lib.findFirst
          (package: package.outPath == overrideBroker.outPath)
          null
          packages;
        service = overriddenGuest.systemd.services.d2b-broker-guest.serviceConfig;
      in
      selected != null
      && selected.outPath != broker.outPath
      && lib.hasPrefix "${overrideBroker.outPath}/bin/d2b-broker guest " service.ExecStart;
    expected = true;
  };
}
