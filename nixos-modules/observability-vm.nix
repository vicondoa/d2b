# Auto-declared observability stack VM.
#
# TODO: `auto-obs-vm`.
# When `nixling.observability.enable = true`, materialise the reserved
# observability env and the headless stack VM that will later host the
# single-host Grafana/Prometheus/Loki/Tempo sink.
{ config, lib, ... }:

let
  cfg = config.nixling.observability;
in
{
  config = lib.mkIf cfg.enable {
    nixling.envs.${cfg.env} = {
      lanSubnet = lib.mkDefault cfg.lanSubnet;
      uplinkSubnet = lib.mkDefault cfg.uplinkSubnet;
    };

    # `cfg.vmName` defaults to `sys-obs-stack`, matching the framework's
    # `sys-<env>-<role>` namespace for auto-declared system VMs. The current
    # reserved-prefix exemption in assertions.nix only whitelists the per-env
    # net VMs; the matching carve-out for this observability VM belongs with
    # the transport-vsock assertions work, not in this module.
    nixling.vms.${cfg.vmName} = {
      env = lib.mkDefault cfg.env;
      index = lib.mkDefault cfg.index;
      autostart = lib.mkDefault true;

      graphics.enable = lib.mkDefault false;
      tpm.enable = lib.mkDefault false;
      usbip.yubikey = lib.mkDefault false;
      audio.enable = lib.mkDefault false;
      audit.enable = lib.mkDefault false;

      config = {
        imports = [
          ./components/observability/stack.nix
        ];
        nixling.observability = {
          retention = lib.mkDefault cfg.retention;
          grafana = lib.mkDefault cfg.grafana;
          transport.relayPackage = lib.mkDefault cfg.transport.relayPackage;
          alerts = lib.mkDefault cfg.alerts;
        };

        # Grafana + Prometheus + Loki + Tempo + Alloy in one VM at
        # microvm.nix's 512M default OOM-kills grafana within seconds
        # of boot. 2 GiB is the minimum that lets the whole stack
        # come up with retention windows in the default range
        # (metrics 30d, logs 14d, traces 7d) and ~tens of monitored
        # VMs. Use `lib.mkDefault` so site operators can override if
        # they're scraping more or want to trim memory.
        microvm.mem = lib.mkDefault 2048;
      };
    };
  };
}
