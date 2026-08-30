{ pkgs, ... }:

let
  guestSystem = pkgs.runCommand "d2b-example-guest-system" { } ''
    mkdir -p "$out"
  '';
  cloudHypervisorProvider = pkgs.runCommand "d2b-example-cloud-hypervisor-provider" { } ''
    mkdir -p "$out"
  '';
in
{
  boot.loader.systemd-boot.enable = false;
  boot.loader.grub.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text = "00000000000000000000000000000000";
  networking.hostName = "demo";
  system.stateVersion = "25.11";

  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" "video" "audio" ];
  };

  d2b.site = {
    waylandUser = "alice";
    launcherUsers = [ "alice" ];
    yubikey.enable = true;
  };

  d2b.artifacts = {
    guest-system = {
      package = guestSystem;
      type = "nixos-system";
    };
    cloud-hypervisor-provider = {
      package = cloudHypervisorProvider;
      type = "provider";
    };
  };

  d2b.zones.local-root.resources = {
    host = {
      type = "Host";
      spec.providerRef = "Provider/system-core";
    };
    runtime-cloud-hypervisor = {
      type = "Provider";
      spec = {
        artifactId = "cloud-hypervisor-provider";
        config.controllerExecutionRef = "Host/host";
      };
    };
    corp-desktop = {
      type = "Guest";
      spec = {
        providerRef = "Provider/runtime-cloud-hypervisor";
        systemArtifactId = "guest-system";
      };
    };
  };

  d2b.guestSystems.local-root.corp-desktop = {
    config.system.build.toplevel = guestSystem;
  };
}
