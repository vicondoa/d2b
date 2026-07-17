# examples/with-observability/configuration.nix
#
# Copy-pasteable operator-facing NixOS configuration that turns on
# the d2b observability subsystem end-to-end:
#
#   * host-side flag    → `d2b.observability.enable = true`
#   * workload opt-in   → import the guest observability component
#   * stack workload    → `sys-obs.local-root.d2b`
#
# Setting the host flag declares the `sys-obs` workload in `local-root`.
# The workload `work-app` lives in the separate `work` realm and
# forwards its telemetry to the stack VM over the vsock transport.
#
# See ./README.md for the topology diagram and validation steps.
{ lib, ... }:

{
  # ---------------------------------------------------------------
  # Host NixOS baseline — PLACEHOLDER stubs so `nix flake check`
  # evaluates without touching real hardware. Replace these with
  # your real bootloader + hardware-configuration.nix + disk layout
  # when copying this example onto a live host.
  # ---------------------------------------------------------------
  boot.loader.systemd-boot.enable = false;
  boot.loader.grub.enable = false;
  boot.loader.efi.canTouchEfiVariables = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = {
    device = "tmpfs";
    fsType = "tmpfs";
  };
  environment.etc."machine-id".text =
    "00000000000000000000000000000000";

  networking.hostName = "demo";
  system.stateVersion = "25.11";
  d2b.acceptDestructiveV2Cutover = true;

  # Host-side human user. Same `alice` placeholder used across the
  # other examples; replace with your login name on a real host.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" ];
  };

  # ---------------------------------------------------------------
  # d2b.site — host-wide knobs.
  #
  # Headless example: no Wayland session, no lifecycle users, no
  # host-side YubiKey rules. Observability does not require any of
  # those, so this stays a minimal-surface host config.
  # ---------------------------------------------------------------
  d2b.site = {
    waylandUser = null;
    launcherUsers = [ ];
    yubikey.enable = false;
  };

  # ---------------------------------------------------------------
  # d2b.observability — turn on the framework's telemetry layer.
  #
  # Setting `enable = true` causes the framework to:
  #   * declare the `sys-obs.local-root.d2b` workload carrying the
  #     native SigNoz stack
  #   * enable the host-side OTLP relay and the
  #     per-VM observability sidecar wiring for any VM that opts in
  #
  # ---------------------------------------------------------------
  d2b.observability = {
    enable = true;
  };

  # ---------------------------------------------------------------
  # d2b.realms.work — the realm that owns the workload.
  # ---------------------------------------------------------------
  d2b.realms.work = {
    path = "work.local-root";
    providers.runtime-local = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };
    workloads.work-app = {
      provider = "runtime-local";
      autostart = true;
      config = {
        imports = [
          ../../nixos-modules/components/observability/guest.nix
        ];
        networking.hostName = lib.mkDefault "work-app";
        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };
      };
    };
  };
}
