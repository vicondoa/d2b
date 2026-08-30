{ pkgs, ... }:

let
  guestSystem = pkgs.runCommand "d2b-example-guest-system" { } ''
    mkdir -p "$out"
  '';
  qemuMediaProvider = pkgs.runCommand "d2b-example-qemu-media-provider" { } ''
    mkdir -p "$out"
  '';
in
{
  # Provider and Guest resources are Zone-local. Physical device selection is
  # an authenticated runtime operation, not a host path or static selector in
  # the Guest spec.
  d2b.artifacts = {
    guest-system = {
      package = guestSystem;
      type = "nixos-system";
    };
    qemu-media-provider = {
      package = qemuMediaProvider;
      type = "provider";
    };
  };

  d2b.zones.local-root.resources = {
    host = {
      type = "Host";
      spec.providerRef = "Provider/system-core";
    };
    runtime-qemu-media = {
      type = "Provider";
      spec = {
        artifactId = "qemu-media-provider";
        config.controllerExecutionRef = "Host/host";
      };
    };
    dark-live = {
      type = "Guest";
      spec = {
        providerRef = "Provider/runtime-qemu-media";
        systemArtifactId = "guest-system";
      };
    };
  };

  d2b.guestSystems.local-root.dark-live = {
    config.system.build.toplevel = guestSystem;
  };
}
