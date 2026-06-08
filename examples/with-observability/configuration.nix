# examples/with-observability/configuration.nix
#
# Copy-pasteable operator-facing NixOS configuration that turns on
# the nixling observability subsystem end-to-end:
#
#   * host-side flag    → `nixling.observability.enable = true`
#   * per-VM opt-in     → `nixling.vms.work-app.observability.enable = true`
#   * stack VM reserved → `nixling.observability.vmName = "sys-obs-stack"`
#
# Setting the host flag auto-declares the `obs` env and the
# `sys-obs-stack` VM (Grafana + Prometheus + Loki + Tempo + Alloy).
# The workload VM `work-app` lives in a separate `work` env and
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

  # Host-side human user. Same `alice` placeholder used across the
  # other examples; replace with your login name on a real host.
  users.users.alice = {
    isNormalUser = true;
    uid = 1000;
    extraGroups = [ "wheel" ];
  };

  # ---------------------------------------------------------------
  # nixling.site — host-wide knobs.
  #
  # Headless example: no Wayland session, no launcher polkit, no
  # host-side YubiKey rules. Observability does not require any of
  # those, so this stays a minimal-surface host config.
  # ---------------------------------------------------------------
  nixling.site = {
    waylandUser = null;
    launcherUsers = [ ];
    yubikey.enable = false;
  };

  # ---------------------------------------------------------------
  # nixling.observability — turn on the framework's telemetry layer.
  #
  # Setting `enable = true` causes the framework to:
  #   * auto-declare `nixling.envs.obs`     (LAN  10.40.0.0/24,
  #                                          uplink 203.0.113.0/30)
  #   * auto-declare `nixling.vms.<vmName>` carrying the
  #     Grafana + Prometheus + Loki + Tempo + Alloy stack
  #     (defaults: 2 GiB RAM, retention 30d/14d/7d, Grafana on
  #     http://10.40.0.10:3000 at index 10)
  #   * enable the host-side OTLP relay, CH exporter, and the
  #     per-VM observability sidecar wiring for any VM that opts in
  #
  # `vmName` is set explicitly to its current default to document
  # the public surface and to pin the VM name this example asserts
  # against in tests/examples-with-observability-eval.sh.
  # ---------------------------------------------------------------
  nixling.observability = {
    enable = true;
    vmName = "sys-obs-stack";
  };

  # ---------------------------------------------------------------
  # nixling.envs.work — the env that hosts the workload VM.
  #
  # The `obs` env is auto-declared by `observability.enable` above,
  # so we only need to declare envs for *workload* VMs here. Pick
  # CIDRs that don't collide with the auto-declared obs env
  # (10.40.0.0/24 / 203.0.113.0/30) or with each other.
  # ---------------------------------------------------------------
  nixling.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  # ---------------------------------------------------------------
  # nixling.vms.work-app — one headless workload VM that opts into
  # observability. The per-VM `observability.enable = true` toggle
  # attaches the guest-side Alloy agent + journald + node-exporter
  # scrape configs and wires the OTLP relay path through the host
  # into `sys-obs-stack`'s vsock receiver.
  #
  # Topology:
  #   work-app guest  → /run/nixling/otlp.sock
  #   host relay      → AF_VSOCK port 14317 on sys-obs-stack
  #   sys-obs-stack   → Prometheus / Loki / Tempo / Grafana
  # ---------------------------------------------------------------
  nixling.vms.work-app = {
    enable = true;
    env = "work";
    index = 10;
    ssh.user = "alice";

    observability.enable = true;

    config = {
      networking.hostName = lib.mkDefault "work-app";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
    };
  };
}
