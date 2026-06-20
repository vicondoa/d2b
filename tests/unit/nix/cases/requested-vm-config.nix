{ mkEval, lib, flakeRoot, ... }:

let
  requested = mkEval [
    (import (flakeRoot + "/examples/qemu-media-dark-live.nix"))
  ];
  cfg = requested.config;
  vm = cfg.nixling.vms."dark-live";
  hostJson = cfg.nixling._bundle.hostJson.data;
  kdl = cfg.environment.etc."nixling/niri-vm-borders.kdl".text;
  rawArtifactText = builtins.toJSON {
    inherit (hostJson) qemuMedia vmRuntimes;
    niri = kdl;
  };
in
{
  "requested-vm-config/evaluates-without-hardware" = {
    expr = lib.all (assertion: assertion.assertion) cfg.assertions;
    expected = true;
  };

  "requested-vm-config/dark-env-declared" = {
    expr = {
      inherit (cfg.nixling.envs.dark) enable lanSubnet uplinkSubnet;
    };
    expected = {
      enable = true;
      lanSubnet = "10.60.0.0/24";
      uplinkSubnet = "203.0.113.0/30";
    };
  };

  "requested-vm-config/dark-live-manual-qemu-media" = {
    expr = {
      inherit (vm) enable env index autostart;
      runtimeKind = vm.runtime.kind;
    };
    expected = {
      enable = true;
      env = "dark";
      index = 10;
      autostart = false;
      runtimeKind = "qemu-media";
    };
  };

  "requested-vm-config/opaque-physical-usb-refs" = {
    expr = {
      boot = {
        inherit (vm.qemuMedia.source) kind ref path format readOnly;
      };
      backup = {
        inherit (vm.qemuMedia.removableSlots.backup.source) kind ref path format readOnly;
      };
    };
    expected = {
      boot = {
        kind = "physical-usb";
        ref = "boot";
        path = null;
        format = "raw";
        readOnly = true;
      };
      backup = {
        kind = "physical-usb";
        ref = "backup";
        path = null;
        format = "raw";
        readOnly = true;
      };
    };
  };

  "requested-vm-config/host-json-has-only-opaque-media" = {
    expr = hostJson.qemuMedia.sources;
    expected = [
      {
        vm = "dark-live";
        mediaRef = "backup";
        slot = "backup";
        sourceKind = "physical-usb";
        format = "raw";
        readOnly = true;
        registryScope = "root-only-runtime-state";
      }
      {
        vm = "dark-live";
        mediaRef = "boot";
        slot = "boot";
        sourceKind = "physical-usb";
        format = "raw";
        readOnly = true;
        registryScope = "root-only-runtime-state";
      }
    ];
  };

  "requested-vm-config/no-raw-usb-identities-in-artifacts" = {
    expr =
      !(lib.hasInfix "/dev/disk/by-id" rawArtifactText)
      && !(lib.hasInfix "/dev/bus/usb" rawArtifactText)
      && !(lib.hasInfix "usb-" rawArtifactText)
      && !(lib.hasInfix "busid" rawArtifactText)
      && !(lib.hasInfix "busId" rawArtifactText)
      && !(lib.hasInfix "serial" rawArtifactText);
    expected = true;
  };

  "requested-vm-config/no-live-os-or-process-marker-sentinels" = {
    expr =
      !(lib.hasInfix "ForbiddenLiveOSName" rawArtifactText)
      && !(lib.hasInfix "Windows" rawArtifactText)
      && !(lib.hasInfix "macOS" rawArtifactText)
      && !(lib.hasInfix "( W" rawArtifactText)
      && !(lib.hasInfix "W3fu" rawArtifactText)
      && !(lib.hasInfix "P6" rawArtifactText);
    expected = true;
  };

  "requested-vm-config/purple-qemu-media-niri-border" = {
    expr =
      lib.hasInfix "// Borders for qemu-media VM host window: dark-live" kdl
      && lib.hasInfix ''match app-id=r#"^nixling\.dark-live\."#'' kdl
      && lib.hasInfix ''active-color "#301934"'' kdl;
    expected = true;
  };
}
